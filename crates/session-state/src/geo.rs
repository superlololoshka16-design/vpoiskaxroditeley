use core_utils::tz;

pub const DEFAULT_TZ: &str = "America/New_York";
pub const DEFAULT_LOCALE: &str = "en-US";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoEntry {
    pub lat: f64,
    pub lon: f64,
    pub tz: &'static str,
}

macro_rules! g {
    ($tld:literal, $lat:expr, $lon:expr, $tz:literal) => {
        ($tld, GeoEntry { lat: $lat, lon: $lon, tz: $tz })
    };
}

#[rustfmt::skip]
pub static GEO_TABLE: [(&str, GeoEntry); 26] = [
    g!("de", 50.1109, 8.6821, "Europe/Berlin"),
    g!("fr", 48.8566, 2.3522, "Europe/Paris"),
    g!("nl", 52.3676, 4.9041, "Europe/Amsterdam"),
    g!("uk", 51.5074, -0.1278, "Europe/London"),
    g!("ie", 53.3498, -6.2603, "Europe/Dublin"),
    g!("ru", 55.7558, 37.6173, "Europe/Moscow"),
    g!("ua", 50.4501, 30.5234, "Europe/Kyiv"),
    g!("tr", 41.0082, 28.9784, "Europe/Istanbul"),
    g!("us", 40.7128, -74.006, "America/New_York"),
    g!("ca", 43.6532, -79.3832, "America/Toronto"),
    g!("mx", 19.4326, -99.1332, "America/Mexico_City"),
    g!("jp", 35.6762, 139.6503, "Asia/Tokyo"),
    g!("kr", 37.5665, 126.978, "Asia/Seoul"),
    g!("cn", 39.9042, 116.4074, "Asia/Shanghai"),
    g!("in", 19.076, 72.8777, "Asia/Kolkata"),
    g!("au", -33.8688, 151.2093, "Australia/Sydney"),
    g!("br", -23.5505, -46.6333, "America/Sao_Paulo"),
    g!("ar", -34.6037, -58.3816, "America/Argentina/Buenos_Aires"),
    g!("es", 40.4168, -3.7038, "Europe/Madrid"),
    g!("it", 41.9028, 12.4964, "Europe/Rome"),
    g!("pl", 52.2297, 21.0122, "Europe/Warsaw"),
    g!("se", 59.3293, 18.0686, "Europe/Stockholm"),
    g!("ch", 47.3769, 8.5417, "Europe/Zurich"),
    g!("at", 48.2082, 16.3738, "Europe/Vienna"),
    g!("ae", 25.2048, 55.2708, "Asia/Dubai"),
    g!("il", 32.0853, 34.7818, "Asia/Jerusalem"),
];

fn tld_of_host(host: &str) -> Option<&str> {
    let last = host.rsplit('.').next()?;
    if last.is_empty() || !last.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    Some(last)
}

pub fn geo_for_host(host: &str) -> Option<GeoEntry> {
    let tld = tld_of_host(host)?;
    if let Some((_, e)) = GEO_TABLE.iter().find(|(t, _)| t.eq_ignore_ascii_case(tld)) {
        return Some(*e);
    }
    if tld.eq_ignore_ascii_case("com")
        || tld.eq_ignore_ascii_case("net")
        || tld.eq_ignore_ascii_case("org")
    {
        return GEO_TABLE.iter().find(|(t, _)| *t == "us").map(|(_, e)| *e);
    }
    None
}

#[inline]
pub fn tz_matches_geo(geo_country: &str, tz_name: &str) -> bool {
    geo_country.is_empty() || tz::country_of(tz_name).is_none_or(|c| c == geo_country)
}

#[rustfmt::skip]
const LANG_COUNTRY: [(&str, &str); 14] = [
    ("en", "US"), ("de", "DE"), ("fr", "FR"), ("es", "ES"), ("it", "IT"), ("nl", "NL"),
    ("pl", "PL"), ("ru", "RU"), ("ja", "JP"), ("ko", "KR"), ("zh", "CN"), ("pt", "BR"),
    ("sv", "SE"), ("hi", "IN"),
];

#[inline]
pub fn lang_matches_geo(geo_country: &str, lang: &str) -> bool {
    if geo_country.is_empty() {
        return true;
    }
    let mut split = lang.splitn(2, '-');
    let primary = split.next().unwrap_or("");
    if let Some(region) = split.next() {
        return region.eq_ignore_ascii_case(geo_country);
    }
    for (p, c) in LANG_COUNTRY {
        if p == primary {
            return c == geo_country;
        }
    }
    true
}

pub fn check_tz_ip_consistency(proxy_host: &str, utc_offset: i32) -> Option<(i32, i32)> {
    let geo = geo_for_host(proxy_host)?;
    let expected = tz::tz_offset_for(geo.tz, core_utils::unix_ms() as i64);
    if utc_offset != expected {
        return Some((expected, utc_offset));
    }
    None
}
