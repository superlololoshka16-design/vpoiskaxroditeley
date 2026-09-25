use core_utils::FxBuild;
use std::collections::HashSet;
use std::sync::OnceLock;

const PGL_LIST: &[u8] = include_bytes!("pgl_domains.txt");

static BLOCKLIST: OnceLock<HashSet<&'static str, FxBuild>> = OnceLock::new();

fn blocklist() -> &'static HashSet<&'static str, FxBuild> {
    BLOCKLIST.get_or_init(|| {
        let mut set: HashSet<&'static str, FxBuild> = HashSet::default();
        for line in PGL_LIST.split(|&b| b == b'\n') {
            let line = line.trim_ascii();
            if !line.is_empty() && line[0] != b'#' && line.is_ascii() {
                set.insert(core::str::from_utf8(line).expect("pgl domains are ascii"));
            }
        }
        set
    })
}

#[inline]
pub fn is_blocked_host(host: &str) -> bool {
    let bl = blocklist();
    if bl.contains(host) {
        return true;
    }
    let mut domain = host;
    while let Some(pos) = domain.find('.') {
        domain = &domain[pos + 1..];
        if bl.contains(domain) {
            return true;
        }
    }
    false
}

pub fn url_blocked(url: &str) -> bool {
    let raw = core_utils::url::split_authority(url).host;
    if raw.is_empty() {
        return false;
    }
    match crate::fetch::normalize_host_whatwg(raw) {
        Some(norm) => host_blocked(norm.as_str()),
        None => true,
    }
}

#[inline]
pub fn host_blocked(norm_host: &str) -> bool {
    !norm_host.is_empty() && is_blocked_host(norm_host)
}
