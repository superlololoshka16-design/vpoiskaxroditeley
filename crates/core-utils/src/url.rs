use compact_str::CompactString;

pub struct Authority<'a> {
    pub scheme: &'a str,
    pub userinfo: Option<(&'a str, Option<&'a str>)>,

    pub host: &'a str,
    pub port: Option<&'a str>,
    pub host_bracketed: bool,

    pub tail: &'a str,
}

#[inline]
pub fn split_authority(s: &str) -> Authority<'_> {
    let (scheme, rest) = match s.split_once("://") {
        Some((a, b)) => (a, b),
        None => ("", s),
    };
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(auth_end);
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, hp)) => (
            Some(match u.split_once(':') {
                Some((n, p)) => (n, Some(p)),
                None => (u, None),
            }),
            hp,
        ),
        None => (None, authority),
    };

    let (host, port, host_bracketed) = if let Some(stripped) = hostport.strip_prefix('[') {
        match stripped.split_once(']') {
            Some((h, p)) => (h, p.strip_prefix(':'), true),
            None => (hostport, None, true),
        }
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) if !h.is_empty() => (h, Some(p), false),
            _ => (hostport, None, false),
        }
    };
    Authority {
        scheme,
        userinfo,
        host,
        port,
        host_bracketed,
        tail,
    }
}

pub trait StrExt {
    fn host(&self) -> CompactString;
    fn host_port(&self) -> CompactString;
    fn origin_parts(&self) -> (CompactString, CompactString);
    fn href_parts(&self) -> HrefParts;
    fn join_origin(&self, path: &str, trim_leading_slash: bool) -> CompactString;
}

const EMPTY: &str = "";

#[inline]
fn port_displayed(scheme: &str, port: &str) -> bool {
    let default = (scheme.eq_ignore_ascii_case("http") && port == "80")
        || (scheme.eq_ignore_ascii_case("https") && port == "443");
    !default && !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
}

fn origin_of_authority(a: &Authority<'_>) -> (CompactString, CompactString) {
    let host = crate::encoding::ascii_lower_compact(a.host);
    let mut origin = CompactString::with_capacity(a.scheme.len() + 3 + host.len() + 8);
    origin.push_str(a.scheme);
    origin.push_str("://");
    if a.host_bracketed {
        origin.push('[');
    }
    origin.push_str(host.as_str());
    if a.host_bracketed {
        origin.push(']');
    }
    if let Some(p) = a.port
        && port_displayed(a.scheme, p)
    {
        origin.push(':');
        origin.push_str(p);
    }
    (origin, host)
}

impl StrExt for str {
    #[inline]
    fn host(&self) -> CompactString {
        let a = split_authority(self);
        crate::encoding::ascii_lower_compact(a.host)
    }

    #[inline]
    fn host_port(&self) -> CompactString {
        let a = split_authority(self);
        let mut out = crate::encoding::ascii_lower_compact(a.host);
        let v6 = a.host_bracketed;
        if let Some(p) = a.port
            && port_displayed(a.scheme, p)
        {
            if v6 {
                out.insert(0, '[');
                out.push(']');
            }
            out.push(':');
            out.push_str(p);
        }
        out
    }

    #[inline]
    fn origin_parts(&self) -> (CompactString, CompactString) {
        let a = split_authority(self);
        if a.scheme.is_empty() {
            return (CompactString::new(EMPTY), CompactString::new(EMPTY));
        }
        origin_of_authority(&a)
    }

    fn href_parts(&self) -> HrefParts {
        let a = split_authority(self);
        if a.scheme.is_empty() {
            return HrefParts {
                origin: CompactString::new(EMPTY),
                host: CompactString::new(EMPTY),
                path: CompactString::new(EMPTY),
                search: CompactString::new(EMPTY),
                hash: CompactString::new(EMPTY),
            };
        }
        let (origin, host) = origin_of_authority(&a);
        let tail = a.tail;
        let hash_at = tail.find('#').unwrap_or(tail.len());
        let no_hash = &tail[..hash_at];
        let search_at = no_hash.find('?').unwrap_or(no_hash.len());
        let (path, search) = no_hash.split_at(search_at);
        HrefParts {
            origin,
            host,
            path: if path.is_empty() {
                CompactString::const_new("/")
            } else {
                CompactString::new(path)
            },
            search: CompactString::new(search),
            hash: if hash_at < tail.len() {
                CompactString::new(&tail[hash_at..])
            } else {
                CompactString::new(EMPTY)
            },
        }
    }

    fn join_origin(&self, path: &str, trim_leading_slash: bool) -> CompactString {
        if path.starts_with("http://") || path.starts_with("https://") {
            return CompactString::new(path);
        }
        let scheme_at = self.find("://");
        let cut = match scheme_at {
            Some(s) => {
                let after = &self[s + 3..];
                let hit = after.find('/').map(|i| s + 3 + i).unwrap_or(self.len());
                &self[..hit]
            }
            None => self,
        };
        if path.starts_with("//") {
            let tail = if trim_leading_slash {
                path.trim_start_matches('/')
            } else {
                path
            };
            let scheme_end = scheme_at.unwrap_or(0);
            let mut out = CompactString::with_capacity(scheme_end + tail.len() + 1);
            if scheme_end > 0 {
                out.push_str(&self[..scheme_end]);
                out.push(':');
            }
            out.push_str(tail);
            return out;
        }
        if trim_leading_slash {
            let trimmed = path.trim_start_matches('/');
            let mut out = CompactString::with_capacity(cut.len() + 1 + trimmed.len());
            out.push_str(cut);
            out.push('/');
            out.push_str(trimmed);
            return out;
        }
        let mut out = CompactString::with_capacity(cut.len() + path.len());
        out.push_str(cut);
        out.push_str(path);
        out
    }
}

pub struct HrefParts {
    pub origin: CompactString,
    pub host: CompactString,
    pub path: CompactString,
    pub search: CompactString,
    pub hash: CompactString,
}

#[inline]
pub fn host_of(url: &str) -> CompactString {
    url.host()
}

pub fn host_of_into(url: &str, out: &mut compact_str::CompactString) {
    let a = split_authority(url);
    out.clear();
    for &b in a.host.as_bytes() {
        out.push(char::from(b.to_ascii_lowercase()));
    }
}

#[inline]
pub fn path_of(url: &str) -> &str {
    let after = url.find("://").map(|i| i + 3).unwrap_or(0);
    url[after..]
        .find('/')
        .map(|i| after + i)
        .map(|i| &url[i..])
        .unwrap_or("/")
}

#[inline]
pub fn origin_of(href: &str) -> (CompactString, CompactString) {
    href.origin_parts()
}

pub fn join_origin(base: &str, path: &str, trim_leading_slash: bool) -> CompactString {
    base.join_origin(path, trim_leading_slash)
}
pub fn split_href(href: &str) -> HrefParts {
    href.href_parts()
}
