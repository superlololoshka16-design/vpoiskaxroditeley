#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProxyKind {
    Datacenter,
    Vpn,
    Tor,
    Residential,
    Mobile,
}

impl ProxyKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ProxyKind::Datacenter => "datacenter",
            ProxyKind::Vpn => "vpn",
            ProxyKind::Tor => "tor",
            ProxyKind::Residential => "residential",
            ProxyKind::Mobile => "mobile",
        }
    }
}

pub const ISSUE_DATACENTER_ASN: u32 = 1 << 0;
pub const ISSUE_KNOWN_VPN_ASN: u32 = 1 << 1;
pub const ISSUE_TOR_EXIT: u32 = 1 << 2;
pub const ISSUE_TIMEZONE_MISMATCH: u32 = 1 << 3;
pub const ISSUE_LANGUAGE_MISMATCH: u32 = 1 << 4;
pub const ISSUE_GEO_LANG_MISMATCH: u32 = 1 << 5;

const SAFE_SCORE_MIN: f64 = 0.7;
const DATACENTER_PENALTY: f64 = 0.5;
const VPN_PENALTY: f64 = 0.35;
const TOR_PENALTY: f64 = 0.9;
const MOBILE_BONUS: f64 = 0.05;
const TZ_MISMATCH_PENALTY: f64 = 0.15;
const LANG_MISMATCH_PENALTY: f64 = 0.15;
const GEO_LANG_MISMATCH_PENALTY: f64 = 0.1;

const fn kind_adjust(kind: ProxyKind) -> (f64, u32) {
    match kind {
        ProxyKind::Datacenter => (-DATACENTER_PENALTY, ISSUE_DATACENTER_ASN),
        ProxyKind::Vpn => (-VPN_PENALTY, ISSUE_KNOWN_VPN_ASN),
        ProxyKind::Tor => (-TOR_PENALTY, ISSUE_TOR_EXIT),
        ProxyKind::Residential => (0.0, 0),
        ProxyKind::Mobile => (MOBILE_BONUS, 0),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProxyAssessment {
    pub score: f64,
    pub kind: ProxyKind,
    pub issues: u32,
}

pub fn assess_proxy(asn: u32, geo_country: &str, tz: &str, lang: &str) -> ProxyAssessment {
    let kind = super::asn::classify_asn(asn);
    let (adjust, mut issues) = kind_adjust(kind);
    let mut score = 1.0 + adjust;
    let tz_mismatch = !super::geo::tz_matches_geo(geo_country, tz);
    let lang_mismatch = !super::geo::lang_matches_geo(geo_country, lang);
    if tz_mismatch {
        score -= TZ_MISMATCH_PENALTY;
        issues |= ISSUE_TIMEZONE_MISMATCH;
    }
    if lang_mismatch {
        score -= LANG_MISMATCH_PENALTY;
        issues |= ISSUE_LANGUAGE_MISMATCH;
    }
    if tz_mismatch && lang_mismatch {
        score -= GEO_LANG_MISMATCH_PENALTY;
        issues |= ISSUE_GEO_LANG_MISMATCH;
    }
    ProxyAssessment {
        score: score.clamp(0.0, 1.0),
        kind,
        issues,
    }
}

pub fn is_safe_for_signup(assessment: &ProxyAssessment) -> bool {
    assessment.score >= SAFE_SCORE_MIN
        && !matches!(
            assessment.kind,
            ProxyKind::Datacenter | ProxyKind::Vpn | ProxyKind::Tor
        )
        && assessment.issues & (ISSUE_TIMEZONE_MISMATCH | ISSUE_LANGUAGE_MISMATCH) == 0
}
