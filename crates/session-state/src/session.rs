use crate::profile::Profile;
use compact_str::CompactString;
use core_utils::FxBuild;
use core_utils::BytesExt as _;
use core_utils::StrExt as _;
use indexmap::IndexMap;
use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u64);

impl TabId {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub const fn as_raw(self) -> u64 {
        self.0
    }
}

#[repr(align(64))]
pub struct SessionHot {
    pub last_seen_ms: AtomicU64,
    pub trust: AtomicI32,
}

pub struct Session {
    pub id: TabId,
    pub jar: CookieJar,
    pub cookie_scratch: CookieScratch,
    pub profile: Arc<Profile>,
    pub origin: CompactString,
    pub hot: SessionHot,
}

impl Session {
    pub fn new(profile: Arc<Profile>, origin: &str) -> Self {
        Self {
            id: TabId::new(),
            jar: CookieJar::new(),
            cookie_scratch: CookieScratch::default(),
            profile,
            origin: CompactString::new(origin),
            hot: SessionHot {
                last_seen_ms: AtomicU64::new(core_utils::unix_ms()),
                trust: AtomicI32::new(0),
            },
        }
    }

    pub fn record_challenge(&self, passed: bool) {
        let w = 2;
        if passed {
            self.hot.trust.fetch_add(w, Ordering::Release);
        } else {
            self.hot.trust.fetch_sub(w, Ordering::Release);
        }
    }

    pub fn trust(&self) -> i32 {
        self.hot.trust.load(Ordering::Acquire)
    }

    pub fn touch(&self) {
        self.hot
            .last_seen_ms
            .store(core_utils::unix_ms(), Ordering::Release);
    }

    pub fn rebase(&mut self, profile: Arc<Profile>, origin: &str) {
        self.id = TabId::new();
        self.origin = CompactString::new(origin);
        self.profile = profile;
    }

    pub fn host(&self) -> CompactString {
        core_utils::host_of(self.origin.as_str())
    }
}


const JAR_CAP: usize = 512;

#[derive(Debug, Clone)]
struct CookieEntry {
    value: CompactString,
    host_only: bool,
    expires_ms: u64,
    secure: bool,
    creation_seq: u64,
}

#[derive(Default, Clone)]
pub struct CookieJar {
    map: IndexMap<(CompactString, CompactString, CompactString), CookieEntry, FxBuild>,
    by_name: std::collections::HashMap<CompactString, SmallVec<[u32; 2]>, FxBuild>,
    next_seq: u64,
}

fn host_key(domain: &str) -> CompactString {
    let d = domain.trim();
    let d = d.strip_prefix('.').unwrap_or(d);
    core_utils::ascii_lower_compact(d)
}

fn domain_matches(host: &str, cookie_domain: &str, host_only: bool) -> bool {
    if host_only || host == cookie_domain {
        return host == cookie_domain;
    }
    !cookie_domain.is_empty()
        && host.len() > cookie_domain.len()
        && host.ends_with(cookie_domain)
        && host.as_bytes()[host.len() - cookie_domain.len() - 1] == b'.'
}

fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    match request_path.strip_prefix(cookie_path) {
        Some("") => true,
        Some(rest) => cookie_path.ends_with('/') || rest.starts_with('/'),
        None => false,
    }
}

fn parent_path(url: &str) -> CompactString {
    let mut out = CompactString::new("");
    parent_path_into(url, &mut out);
    out
}

fn parent_path_into(url: &str, out: &mut CompactString) {
    out.clear();
    let path = core_utils::path_of(url);
    let q = path.find(['?', '#']).unwrap_or(path.len());
    let path = &path[..q];
    if path.is_empty() {
        out.push('/');
        return;
    }
    match path.rfind('/') {
        Some(0) | None => out.push('/'),
        Some(i) => out.push_str(&path[..i]),
    }
}

fn attr_of(part: &str) -> Option<(&str, &str)> {
    let eq = part.find('=')?;
    Some((part[..eq].trim(), part[eq + 1..].trim()))
}

#[derive(Default)]
pub struct CookieScratch {
    host: CompactString,
    path: CompactString,
    buf: SmallVec<[u8; 256]>,
    out: CompactString,
}
impl CookieJar {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn clear(&mut self) {
        self.map.clear();
        self.by_name.clear();
        self.next_seq = 0;
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn copy_matching(&mut self, from: &CookieJar, url: &str) {
        let host = host_key(core_utils::host_of(url).as_str());
        let host = host.as_str();
        let path = parent_path(url);
        let secure_ok = url_is_secure(url);
        let now = core_utils::unix_ms();
        for ((name, domain, cpath), entry) in from.map.iter() {
            if entry.expires_ms <= now {
                continue;
            }
            if entry.secure && !secure_ok {
                continue;
            }
            if !domain_matches(host, domain.as_str(), entry.host_only) {
                continue;
            }
            if !path_matches(path.as_str(), cpath.as_str()) {
                continue;
            }
            let existing = self.map.get_index_of(&(name, domain, cpath));
            let creation_seq = match existing {
                Some(idx) => self.map.get_index(idx).map(|(_, e)| e.creation_seq).unwrap_or_default(),
                None => {
                    let seq = self.next_seq;
                    self.next_seq = self.next_seq.wrapping_add(1);
                    seq
                }
            };
            let key = (name, domain, cpath);
            let value = entry.value.clone();
            let (host_only, expires_ms, secure) = (entry.host_only, entry.expires_ms, entry.secure);
            self.map.insert(
                key,
                CookieEntry {
                    value,
                    host_only,
                    expires_ms,
                    secure,
                    creation_seq,
                },
            );
            if existing.is_none() {
                let idx = (self.map.len() - 1) as u32;
                match self.by_name.get_mut(name) {
                    Some(v) => v.push(idx),
                    None => {
                        self.by_name.insert(CompactString::new(name), {
                            let mut v = SmallVec::new();
                            v.push(idx);
                            v
                        });
                    }
                }
            }
        }
    }
    pub fn ingest(&mut self, set_cookie: &str) {
        self.ingest_scoped(set_cookie, "", "");
    }

    pub fn ingest_scoped(&mut self, set_cookie: &str, url_host: &str, url_path: &str) {
        let semi = set_cookie.find(';');
        let first_raw = &set_cookie[..semi.unwrap_or(set_cookie.len())];
        let attrs_raw = &set_cookie[semi.map_or(set_cookie.len(), |i| i + 1)..];
        let line = first_raw.trim();
        let Some(eq) = line.find('=') else {
            return;
        };
        let (name, value) = line.split_at(eq);
        let name = name.trim_end();
        let value = value[1..].trim();
        if name.is_empty() || name.len() > 256 || value.len() > 4096 {
            return;
        }
        let mut domain = if url_host.is_empty() {
            CompactString::const_new("")
        } else {
            host_key(url_host)
        };
        let mut host_only = true;
        let mut path = if url_path.is_empty() {
            CompactString::const_new("/")
        } else {
            CompactString::new(url_path)
        };
        let mut expires_ms = u64::MAX;
        let mut max_age_seen = false;
        let mut secure = false;
        for part in attrs_raw.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match attr_of(part) {
                None => {
                    if part.eq_ignore_ascii_case("secure") {
                        secure = true;
                    }
                    continue;
                }
                Some((key, v)) => {
                    if key.eq_ignore_ascii_case("secure") {
                        secure = true;
                    } else if key.eq_ignore_ascii_case("domain") && !v.is_empty() {
                        let d = host_key(v);
                        if !url_host.is_empty() && domain_matches(url_host, d.as_str(), false) {
                            domain = d;
                            host_only = false;
                        }
                    } else if key.eq_ignore_ascii_case("path")
                        && !v.is_empty()
                        && v.starts_with('/')
                    {
                        path = CompactString::new(v);
                    } else if key.eq_ignore_ascii_case("max-age") {
                        max_age_seen = true;
                        expires_ms = match v.parse::<i64>() {
                            Ok(secs) if secs > 0 => {
                                core_utils::unix_ms().saturating_add((secs as u64) * 1000)
                            }
                            _ => 0,
                        };
                    } else if key.eq_ignore_ascii_case("expires")
                        && !max_age_seen
                        && let Some(ms) = http_date_ms(v)
                    {
                        expires_ms = ms;
                    }
                }
            }
        }
        if self.map.len() >= JAR_CAP {
            let now = core_utils::unix_ms();
            self.map.retain(|_, e| e.expires_ms > now);
            if self.map.len() >= JAR_CAP {
                let mut victim_idx = 0usize;
                let mut victim_key = (u64::MAX, u64::MAX);
                for (i, (_, e)) in self.map.iter().enumerate() {
                    let key = (e.expires_ms, e.creation_seq);
                    if key < victim_key {
                        victim_key = key;
                        victim_idx = i;
                    }
                }
                if self.map.shift_remove_index(victim_idx).is_some() {
                    self.by_name.clear();
                    for (i, (k, _)) in self.map.iter().enumerate() {
                        match self.by_name.get_mut(k.0.as_str()) {
                            Some(v) => v.push(i as u32),
                            None => {
                                let mut v = SmallVec::new();
                                v.push(i as u32);
                                self.by_name.insert(k.0.clone(), v);
                            }
                        }
                    }
                }
            }
        }
        let key = (CompactString::new(name), domain, path);
        let existing_idx = self.map.get_index_of(&key);
        let creation_seq = match existing_idx {
            Some(idx) => self.map.get_index(idx).map(|(_, e)| e.creation_seq).unwrap_or_default(),
            None => {
                let seq = self.next_seq;
                self.next_seq = self.next_seq.wrapping_add(1);
                seq
            }
        };
        self.map.insert(key, CookieEntry {
            value: CompactString::new(value),
            host_only,
            expires_ms,
            secure,
            creation_seq,
        });
        if existing_idx.is_none() {
            let idx = (self.map.len() - 1) as u32;
            match self.by_name.get_mut(name) {
                Some(v) => v.push(idx),
                None => {
                    self.by_name.insert(CompactString::new(name), {
                        let mut v = SmallVec::new();
                        v.push(idx);
                        v
                    });
                }
            }
        }
    }


    pub fn ingest_for_url(&mut self, set_cookie: &str, url: &str) {
        let host = url.host();
        let path = parent_path(url);
        self.ingest_scoped(set_cookie, host.as_str(), path.as_str());
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        let now = core_utils::unix_ms();
        let idxs = self.by_name.get(name)?;
        idxs.iter().find_map(|&i| {
            let ((_, _, _), e) = self.map.get_index(i as usize)?;
            (e.expires_ms > now).then(|| e.value.as_str())
        })
    }

    pub fn get_for_host(&self, name: &str, host: &str) -> Option<&str> {
        let now = core_utils::unix_ms();
        let host = host_key(host);
        let idxs = self.by_name.get(name)?;
        idxs.iter().find_map(|&i| {
            let ((_, d, _), e) = self.map.get_index(i as usize)?;
            (domain_matches(host.as_str(), d.as_str(), e.host_only) && e.expires_ms > now)
                .then(|| e.value.as_str())
        })
    }
    pub fn header_into(&self, out: &mut SmallVec<[u8; 256]>) {
        self.header_for_into("", "", true, out);
    }

    pub fn header_for_url(&self, url: &str) -> Option<CompactString> {
        let mut scratch = CookieScratch::default();
        self.header_for_url_into(url, &mut scratch)
            .map(|h| CompactString::new(h))
    }

    pub fn header_for_url_into<'a>(
        &self,
        url: &str,
        scratch: &'a mut CookieScratch,
    ) -> Option<&'a str> {
        scratch.host.clear();
        core_utils::host_of_into(url, &mut scratch.host);
        let host = scratch.host.as_str();
        scratch.path.clear();
        parent_path_into(url, &mut scratch.path);
        let secure_ok = url_is_secure(url);
        scratch.buf.clear();
        self.header_for_into(host, scratch.path.as_str(), secure_ok, &mut scratch.buf);
        header_string(&scratch.buf).map(|s| {
            scratch.out.clear();
            scratch.out.push_str(s.as_str());
            scratch.out.as_str()
        })
    }

    fn header_for_into(
        &self,
        host: &str,
        path: &str,
        secure_ok: bool,
        out: &mut SmallVec<[u8; 256]>,
    ) {
        let now = core_utils::unix_ms();
        let mut idx: SmallVec<[(usize, usize, u64); 24]> = SmallVec::new();
        for (i, ((_, domain, cpath), entry)) in self.map.iter().enumerate() {
            if entry.expires_ms <= now {
                continue;
            }
            if entry.secure && !secure_ok {
                continue;
            }
            if !host.is_empty() {
                if !domain_matches(host, domain.as_str(), entry.host_only) {
                    continue;
                }
            } else if !domain.is_empty() && entry.host_only {
                continue;
            }
            if !path.is_empty() && !path_matches(path, cpath.as_str()) {
                continue;
            }
            idx.push((i, cpath.len(), entry.creation_seq));
        }
        idx.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
        let mut first = true;
        for (i, _, _) in idx {
            let ((name, _, _), entry) = self.map.get_index(i).unwrap();
            if !first {
                out.push(b';');
                out.push(b' ');
            }
            out.extend_from_slice(name.as_bytes());
            out.push(b'=');
            out.extend_from_slice(entry.value.as_bytes());
            first = false;
        }
    }

    pub fn header_str(&self) -> Option<CompactString> {
        let mut buf: SmallVec<[u8; 256]> = SmallVec::new();
        self.header_into(&mut buf);
        header_string(&buf)
    }
}

fn url_is_secure(url: &str) -> bool {
    url.as_bytes().starts_with_ci(b"https:")
}

fn header_string(buf: &[u8]) -> Option<CompactString> {
    let s = std::str::from_utf8(buf).ok()?;
    if s.is_empty() {
        return None;
    }
    Some(CompactString::new(s))
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

fn http_date_ms(v: &str) -> Option<u64> {
    let mut it = v
        .split([' ', ','])
        .filter(|p| !p.is_empty());
    it.next()?;
    let day: u32 = it.next()?.parse().ok()?;
    if !(1..=31).contains(&day) {
        return None;
    }
    let mon_str = it.next()?;
    let mon = MONTHS
        .iter()
        .position(|m| mon_str.eq_ignore_ascii_case(m))? as u32;
    let year: i64 = it.next()?.parse().ok()?;
    if !(1970..=9999).contains(&year) {
        return None;
    }
    let time = it.next()?;
    let mut tp = time.split(':');
    let hour: i64 = tp.next()?.parse().ok()?;
    let minute: i64 = tp.next()?.parse().ok()?;
    let second: i64 = tp.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) || !(0..=59).contains(&second) {
        return None;
    }
    let ms = core_utils::epoch_ms_from_civil(year as i32, (mon + 1) as u8, day as u8, hour as u8, minute as u8, second as u8, 0);
    if ms < 0 {
        return Some(0);
    }
    Some(ms as u64)
}
