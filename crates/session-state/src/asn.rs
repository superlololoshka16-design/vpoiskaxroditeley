use super::assess::ProxyKind;

pub const NET_DATACENTER: u8 = 0;
pub const NET_RESIDENTIAL: u8 = 1;
pub const NET_MOBILE: u8 = 2;

pub const ASN_CLASS_RESIDENTIAL: u8 = 0;
pub const ASN_CLASS_DATACENTER: u8 = 1;
pub const ASN_CLASS_VPN: u8 = 2;
pub const ASN_CLASS_TOR: u8 = 3;
pub const ASN_CLASS_MOBILE: u8 = 4;

#[derive(Debug, Clone, Copy)]
pub struct AsnRow {
    pub asn: u32,
    pub carrier: &'static str,
    pub net: u8,
    pub tz: &'static str,
    pub locale: &'static str,
    pub class: u8,
}

const D: u8 = NET_DATACENTER;
const R: u8 = NET_RESIDENTIAL;
const M: u8 = NET_MOBILE;
const CR: u8 = ASN_CLASS_RESIDENTIAL;
const CD: u8 = ASN_CLASS_DATACENTER;
const CV: u8 = ASN_CLASS_VPN;
const CT: u8 = ASN_CLASS_TOR;
const CM: u8 = ASN_CLASS_MOBILE;

macro_rules! r {
    ($asn:expr, $net:expr, $class:expr) => {
        AsnRow { asn: $asn, carrier: "", net: $net, tz: "", locale: "", class: $class }
    };
    ($asn:expr, $net:expr, $class:expr, $carrier:literal, $tz:literal, $locale:literal) => {
        AsnRow { asn: $asn, carrier: $carrier, net: $net, tz: $tz, locale: $locale, class: $class }
    };
}

#[rustfmt::skip]
pub static ASN_REG: &[AsnRow] = &[
    r!(209, D, CD),
    r!(701, M, CM, "Verizon Wireless", "America/New_York", "en-US"),
    r!(1257, R, CM),
    r!(2635, D, CD),
    r!(3209, M, CM, "Vodafone GmbH", "Europe/Berlin", "de-DE"),
    r!(3301, R, CM),
    r!(3320, R, CM, "Deutsche Telekom AG", "Europe/Berlin", "de-DE"),
    r!(4400, M, CM, "KDDI Corporation", "Asia/Tokyo", "ja-JP"),
    r!(5588, R, CM),
    r!(7018, M, CM, "AT&T Mobility", "America/Chicago", "en-US"),
    r!(7843, R, CR, "Charter Communications", "America/Los_Angeles", "en-US"),
    r!(7922, R, CR, "Comcast Cable", "America/New_York", "en-US"),
    r!(8068, D, CD),
    r!(8069, D, CD),
    r!(8075, D, CD, "Microsoft Corporation", "America/Chicago", "en-US"),
    r!(8402, R, CM),
    r!(9009, D, CD, "M247 Europe SRL", "Europe/Bucharest", "ro-RO"),
    r!(13030, R, CT),
    r!(13213, R, CT),
    r!(13335, D, CD),
    r!(14061, D, CD, "DigitalOcean LLC", "America/New_York", "en-US"),
    r!(14618, D, CD),
    r!(15169, D, CD, "Google LLC", "America/Los_Angeles", "en-US"),
    r!(15179, D, CD),
    r!(15412, R, CM),
    r!(16509, D, CD, "Amazon.com", "America/New_York", "en-US"),
    r!(20057, R, CR, "AT&T Internet", "America/Chicago", "en-US"),
    r!(20829, R, CT),
    r!(21412, R, CM),
    r!(21930, M, CM, "T-Mobile USA", "America/New_York", "en-US"),
    r!(22201, M, CM, "Vodafone Italia", "Europe/Rome", "it-IT"),
    r!(22394, R, CR, "Comcast Business", "America/New_York", "en-US"),
    r!(24940, D, CD, "Hetzner Online GmbH", "Europe/Berlin", "de-DE"),
    r!(35540, D, CV),
    r!(35913, R, CV),
    r!(36384, D, CD),
    r!(36385, D, CD),
    r!(39351, D, CD),
    r!(40676, D, CD),
    r!(41231, R, CV),
    r!(46562, D, CV),
    r!(49335, R, CV),
    r!(49505, R, CV),
    r!(49981, R, CV),
    r!(50266, R, CM),
    r!(51167, R, CV),
    r!(62567, D, CD),
    r!(63949, D, CD),
    r!(197540, R, CV),
    r!(197669, R, CT),
    r!(197727, R, CT),
    r!(198335, R, CT),
    r!(396982, D, CD),
    r!(398101, D, CD),
];

pub fn asn_lookup(asn: u32) -> Option<&'static AsnRow> {
    ASN_REG
        .binary_search_by(|r| r.asn.cmp(&asn))
        .ok()
        .map(|i| &ASN_REG[i])
}

pub fn net_kind_of(code: u8) -> crate::NetKind {
    match code {
        NET_DATACENTER => crate::NetKind::Datacenter,
        NET_MOBILE => crate::NetKind::Mobile,
        _ => crate::NetKind::Residential,
    }
}

pub fn classify_asn(asn: u32) -> ProxyKind {
    match asn_lookup(asn).map_or(ASN_CLASS_RESIDENTIAL, |r| r.class) {
        ASN_CLASS_DATACENTER => ProxyKind::Datacenter,
        ASN_CLASS_VPN => ProxyKind::Vpn,
        ASN_CLASS_TOR => ProxyKind::Tor,
        ASN_CLASS_MOBILE => ProxyKind::Mobile,
        _ => ProxyKind::Residential,
    }
}
