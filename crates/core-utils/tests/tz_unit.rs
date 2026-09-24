use core_utils::days_from_civil;
use core_utils::tz::{
    all_zone_names, civil_of, country_of, tz_offset_for, tz_offset_min, zone_name, zone_of,
};

fn noon_utc_ms(y: i64, m: u32, d: u32) -> i64 {
    days_from_civil(y, m, d) * 86_400_000 + 43_200_000
}

#[test]
fn zones_sorted_for_binary_search() {
    let mut prev = "";
    for z in all_zone_names() {
        assert!(
            z.as_bytes() > prev.as_bytes(),
            "zone {} out of order after {}",
            z,
            prev
        );
        prev = z;
    }
}

#[test]
fn zone_lookup_edges() {
    let berlin = zone_of("Europe/Berlin").expect("berlin");
    assert_eq!(zone_name(berlin), "Europe/Berlin");
    assert_eq!(tz_offset_min(berlin, noon_utc_ms(2026, 1, 15)), 60);
    assert_eq!(zone_of(""), None);
    assert_eq!(zone_of("europe/berlin"), None);
    assert_eq!(zone_of("Europe/Berlin "), None);
    assert_eq!(zone_of("Nonexistent/Zone"), None);
    let names: Vec<&str> = all_zone_names().collect();
    assert_eq!(names.len(), 436);
    assert!(names.iter().all(|n| !n.is_empty()));
    assert_eq!(names.windows(2).filter(|w| w[0] >= w[1]).count(), 0);
}

#[test]
fn country_semantics_matches_old_country_for_tz() {
    assert_eq!(country_of("Europe/Berlin"), Some("DE"));
    assert_eq!(country_of("Australia/Sydney"), Some("AU"));
    assert_eq!(country_of("Australia/Perth"), Some("AU"));
    assert_eq!(country_of("America/New_York"), Some("US"));
    assert_eq!(country_of("Asia/Hong_Kong"), Some("CN"));
    assert_eq!(country_of("Europe/Dublin"), None);
    assert_eq!(country_of("Africa/Cairo"), None);
    assert_eq!(country_of("Nonexistent/Zone"), None);
}

#[test]
fn dst_offsets_winter_summer() {
    assert_eq!(tz_offset_for("Europe/Berlin", 1_768_478_400_000), 60);
    assert_eq!(tz_offset_for("Europe/Berlin", 1_784_107_200_000), 120);
    assert_eq!(tz_offset_for("Nonexistent", 0), 0);
    assert_eq!(tz_offset_for("Europe/Berlin", noon_utc_ms(2026, 1, 15)), 60);
    assert_eq!(
        tz_offset_for("Europe/Berlin", noon_utc_ms(2026, 7, 15)),
        120
    );
}

#[test]
fn dst_southern_hemisphere_flipped() {
    let sydney = zone_of("Australia/Sydney").expect("sydney");
    assert_eq!(tz_offset_min(sydney, noon_utc_ms(2026, 1, 15)), 660);
    assert_eq!(tz_offset_min(sydney, noon_utc_ms(2026, 7, 15)), 600);
}

#[test]
fn us_dst_march_forward_november_back() {
    let ny = zone_of("America/New_York").expect("ny");
    assert_eq!(tz_offset_min(ny, noon_utc_ms(2026, 3, 1)), -300);
    assert_eq!(tz_offset_min(ny, noon_utc_ms(2026, 3, 20)), -240);
    assert_eq!(tz_offset_min(ny, noon_utc_ms(2026, 11, 3)), -300);
}

#[test]
fn fixed_offset_zone_never_dst() {
    let kolkata = zone_of("Asia/Kolkata").expect("kolkata");
    assert_eq!(tz_offset_min(kolkata, noon_utc_ms(2026, 1, 15)), 330);
    assert_eq!(tz_offset_min(kolkata, noon_utc_ms(2026, 7, 15)), 330);
    assert_eq!(tz_offset_min(kolkata, 0), 330);
}

#[test]
fn civil_of_anchors_offsets_and_dow() {
    let c = civil_of(0, 0);
    assert_eq!(
        (c.year, c.month, c.day, c.hour, c.minute, c.second, c.dow),
        (1970, 1, 1, 0, 0, 0, 4)
    );
    let c = civil_of(0, 330);
    assert_eq!(
        (c.year, c.month, c.day, c.hour, c.minute, c.second, c.dow),
        (1970, 1, 1, 5, 30, 0, 4)
    );
    let c = civil_of(0, -300);
    assert_eq!(
        (c.year, c.month, c.day, c.hour, c.minute, c.second, c.dow),
        (1969, 12, 31, 19, 0, 0, 3)
    );
    let c = civil_of(-1, 0);
    assert_eq!(
        (c.year, c.month, c.day, c.hour, c.minute, c.second, c.dow),
        (1969, 12, 31, 23, 59, 59, 3)
    );
    let c = civil_of(86_400_000, 60);
    assert_eq!(
        (c.year, c.month, c.day, c.hour, c.minute, c.second, c.dow),
        (1970, 1, 2, 1, 0, 0, 5)
    );
}
