use aho_corasick::AhoCorasick;
use compact_str::CompactString;
use core_utils::BytesExt;
use std::sync::{LazyLock, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptchaFamily {
    Hcaptcha,
    RecaptchaV2,
    RecaptchaV3,
    Turnstile,
    CloudflareChallenge,
    Geetest,
    Funcaptcha,
    AwsWaf,
    YandexSmartCaptcha,
    Proton,
    Datadome,
    ArkoseLabs,
    Unknown,
}

impl CaptchaFamily {
    pub const ALL: &'static [CaptchaFamily] = &[
        CaptchaFamily::Hcaptcha,
        CaptchaFamily::RecaptchaV2,
        CaptchaFamily::RecaptchaV3,
        CaptchaFamily::Turnstile,
        CaptchaFamily::CloudflareChallenge,
        CaptchaFamily::Geetest,
        CaptchaFamily::Funcaptcha,
        CaptchaFamily::AwsWaf,
        CaptchaFamily::YandexSmartCaptcha,
        CaptchaFamily::Proton,
        CaptchaFamily::Datadome,
        CaptchaFamily::ArkoseLabs,
        CaptchaFamily::Unknown,
    ];

    const NAMES: [&'static str; 13] = [
        "hcaptcha",
        "recaptcha-v2",
        "recaptcha-v3",
        "turnstile",
        "cloudflare-challenge",
        "geetest",
        "funcaptcha",
        "aws-waf",
        "yandex-smartcaptcha",
        "proton",
        "datadome",
        "arkose-labs",
        "unknown",
    ];

    #[inline]
    pub const fn as_str(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    #[inline]
    pub const fn idx(self) -> u8 {
        self as u8
    }

    #[inline]
    pub const fn from_idx(v: u8) -> Self {
        if v as usize >= Self::ALL.len() {
            CaptchaFamily::Unknown
        } else {
            Self::ALL[v as usize]
        }
    }

    #[inline]
    pub const fn is_invisible(self) -> bool {
        matches!(
            self,
            CaptchaFamily::RecaptchaV3
                | CaptchaFamily::Datadome
                | CaptchaFamily::CloudflareChallenge
        )
    }
}

pub(crate) const PRIORITY: &[CaptchaFamily] = &[
    CaptchaFamily::Hcaptcha,
    CaptchaFamily::RecaptchaV2,
    CaptchaFamily::Turnstile,
    CaptchaFamily::AwsWaf,
    CaptchaFamily::ArkoseLabs,
    CaptchaFamily::YandexSmartCaptcha,
    CaptchaFamily::Geetest,
    CaptchaFamily::Funcaptcha,
    CaptchaFamily::Proton,
    CaptchaFamily::RecaptchaV3,
    CaptchaFamily::Datadome,
    CaptchaFamily::CloudflareChallenge,
];

use crate::telemetry::{RouteSpec, TelemetryProvider as P, Transport as T};
use CaptchaFamily as F;

pub(crate) struct NeedleRow {
    pub(crate) needle: &'static [u8],
    pub(crate) family: CaptchaFamily,
    pub(crate) family_scan: bool,
    pub(crate) challenge: bool,
    pub(crate) route: Option<RouteSpec>,
}

const fn rt(p: P, endpoint: &'static str, t: T, field: &'static str) -> Option<RouteSpec> {
    Some(RouteSpec { provider: p, endpoint, transport: t, field })
}

const ARKOSE_ROUTE: Option<RouteSpec> = rt(P::Arkose, "https://client-api.arkoselabs.com/v2", T::CdnPost, "");

pub(crate) const NEEDLE_ROWS: [NeedleRow; 15] = [
    NeedleRow { needle: b"hcaptcha.com", family: F::Hcaptcha, family_scan: true, challenge: true, route: rt(P::HCaptcha, "https://api.hcaptcha.com/checkcaptcha", T::CdnPost, "") },
    NeedleRow { needle: b"recaptcha/api.js", family: F::RecaptchaV2, family_scan: true, challenge: true, route: rt(P::ReCaptcha, "https://www.google.com/recaptcha/api2/userverify", T::CdnPost, "") },
    NeedleRow { needle: b"recaptcha/api2", family: F::RecaptchaV2, family_scan: true, challenge: false, route: None },
    NeedleRow { needle: b"challenges.cloudflare.com/turnstile", family: F::Turnstile, family_scan: true, challenge: true, route: rt(P::Turnstile, "https://challenges.cloudflare.com/turnstile/v0/telemetry", T::CdnPost, "") },
    NeedleRow { needle: b"cdn-cgi/challenge-platform", family: F::CloudflareChallenge, family_scan: true, challenge: true, route: rt(P::Turnstile, "https://challenges.cloudflare.com/cdn-cgi/telemetry", T::CustomHeader, "cf-chl-telemetry") },
    NeedleRow { needle: b"geetest.com", family: F::Geetest, family_scan: true, challenge: false, route: None },
    NeedleRow { needle: b"funcaptcha.com", family: F::Funcaptcha, family_scan: true, challenge: false, route: ARKOSE_ROUTE },
    NeedleRow { needle: b"awswaf.com", family: F::AwsWaf, family_scan: true, challenge: false, route: None },
    NeedleRow { needle: b"smartcaptcha", family: F::YandexSmartCaptcha, family_scan: true, challenge: false, route: None },
    NeedleRow { needle: b"proton-captcha", family: F::Proton, family_scan: true, challenge: false, route: None },
    NeedleRow { needle: b"captcha-delivery.com", family: F::Datadome, family_scan: true, challenge: true, route: rt(P::DataDome, "https://geo.captcha-delivery.com/telemetry", T::CdnPost, "datadome") },
    NeedleRow { needle: b"arkoselabs.com", family: F::ArkoseLabs, family_scan: true, challenge: false, route: ARKOSE_ROUTE },
    NeedleRow { needle: b"challenges.cloudflare.com/managed", family: F::CloudflareChallenge, family_scan: false, challenge: true, route: None },
    NeedleRow { needle: b"arkoselabs.com/v2", family: F::ArkoseLabs, family_scan: false, challenge: false, route: ARKOSE_ROUTE },
    NeedleRow { needle: b"datadome", family: F::Datadome, family_scan: false, challenge: false, route: rt(P::DataDome, "https://geo.captcha-delivery.com/telemetry", T::FormField, "datadome") },
];

pub(crate) fn build_ac(
    needles: impl IntoIterator<Item = &'static [u8]>,
    ci: bool,
) -> AhoCorasick {
    let mut builder = aho_corasick::AhoCorasick::builder();
    builder.match_kind(aho_corasick::MatchKind::LeftmostFirst);
    if ci {
        builder.ascii_case_insensitive(true);
    }
    builder.build(needles).expect("static needles")
}

static FAMILY_ROWS: LazyLock<Vec<&'static NeedleRow>> =
    LazyLock::new(|| NEEDLE_ROWS.iter().filter(|r| r.family_scan).collect());

static FAMILY_AC: LazyLock<AhoCorasick> =
    LazyLock::new(|| build_ac(FAMILY_ROWS.iter().map(|r| r.needle), false));

static CHALLENGE_AC: LazyLock<AhoCorasick> = LazyLock::new(|| {
    build_ac(
        NEEDLE_ROWS.iter().filter(|r| r.challenge).map(|r| r.needle),
        false,
    )
});

pub(crate) fn challenge_match(src: &str) -> bool {
    CHALLENGE_AC.is_match(src.as_bytes())
}

#[derive(Debug, Clone)]
pub struct WidgetHit {
    pub family: CaptchaFamily,
    pub sitekey: Option<CompactString>,
}

#[inline]
pub fn sitekey_passes(s: &str) -> bool {
    if s.is_empty() || s.len() > 256 {
        return false;
    }
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn extract_sitekey_from_url(url: &str) -> Option<&str> {
    let rest = &url.as_bytes()[url.find('?')? + 1..];
    const PREFIX_KEYS: &[&[u8]] = &[
        b"sitekey=",
        b"site_key=",
        b"site-key=",
        b"k=",
        b"captcha_id=",
        b"captchaId=",
        b"data-sitekey=",
    ];
    for &key in PREFIX_KEYS {
        let Some(raw) = rest.find_token(key, b'&', usize::MAX) else {
            continue;
        };
        let cut = raw
            .iter()
            .position(|&b| matches!(b, b'#' | b';'))
            .unwrap_or(raw.len());
        let val = raw[..cut].trim_ascii_extra(b"\"");
        if !val.is_empty() {
            return core::str::from_utf8(val).ok();
        }
    }
    None
}

pub(crate) fn detect_family_in_url(url: &str) -> Option<CaptchaFamily> {
    let mat = FAMILY_AC.find(url.as_bytes())?;
    let row = FAMILY_ROWS[mat.pattern().as_usize()];
    if matches!(row.family, CaptchaFamily::RecaptchaV2)
        && (url.contains("enterprise") || url.contains("v3"))
    {
        return Some(CaptchaFamily::RecaptchaV3);
    }
    Some(row.family)
}

pub fn detect_widget(srcs: &[&str], marker_urls: &[&str]) -> Option<WidgetHit> {
    let mut best: Option<(usize, CaptchaFamily, &str)> = None;
    for url in marker_urls.iter().chain(srcs.iter()) {
        let Some(family) = detect_family_in_url(url) else {
            continue;
        };
        let Some(rank) = PRIORITY.iter().position(|&p| p == family) else {
            continue;
        };
        if best.is_none_or(|(r, _, _)| rank < r) {
            best = Some((rank, family, url));
        }
    }
    let (_, family, url) = best?;
    let sitekey = extract_sitekey_from_url(url)
        .filter(|s| sitekey_passes(s))
        .map(CompactString::new);
    Some(WidgetHit { family, sitekey })
}

struct Probe {
    markers: AhoCorasick,
    libs: AhoCorasick,
}

const MARKERS: &[&str] = &[
    "eval(",
    "atob(",
    "btoa(",
    "Function(",
    "String.fromCharCode",
    "setTimeout(",
    "setInterval(",
    "document.write",
    "execScript",
    "charCodeAt",
];

const LIBS: &[&str] = &[
    "jQuery",
    "jquery",
    "React.createElement",
    "react-dom",
    "vue.runtime",
    "Vue.directive",
    "__NEXT_DATA__",
    "webpackChunk",
    "angular.module",
    "svelte",
];

static PROBE: OnceLock<Probe> = OnceLock::new();

fn probe() -> &'static Probe {
    PROBE.get_or_init(|| Probe {
        markers: build_ac(MARKERS.iter().map(|s| s.as_bytes()), false),
        libs: build_ac(LIBS.iter().map(|s| s.as_bytes()), false),
    })
}

impl Probe {
    fn scan(&self, script: &[u8]) -> (u32, bool) {
        let mut seen = [false; MARKERS.len()];
        for mat in self.markers.find_iter(script) {
            seen[mat.pattern().as_usize()] = true;
        }
        (
            seen.iter().map(|&b| u32::from(b)).sum(),
            self.libs.is_match(script),
        )
    }
}

pub(crate) fn is_challenge(script: &[u8]) -> bool {
    if script.len() < 384 {
        return false;
    }
    let (distinct, looks_like_lib) = probe().scan(script);
    distinct >= 3 && !looks_like_lib
}
