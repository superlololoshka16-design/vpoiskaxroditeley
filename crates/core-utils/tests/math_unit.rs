use compact_str::CompactString;
use core_utils::math::*;

#[test]
fn truncate_str_never_splits_chars() {
    assert_eq!(truncate_str("héllo wörld", 3), "hé");
    assert_eq!(truncate_str("héllo", 2), "h");
    assert_eq!(truncate_str("abc", 10), "abc");
    assert_eq!(truncate_str("日本語", 3), "日");
}

#[test]
fn padded_int_matches_itoa_year() {
    let mut s = CompactString::new("");
    push_int_padded_into(&mut s, 2026, 4);
    assert_eq!(s.as_str(), "2026");
    let mut s = CompactString::new("");
    push_int_padded_into(&mut s, 7, 4);
    assert_eq!(s.as_str(), "0007");
    let mut s = CompactString::new("");
    push_int_padded_into(&mut s, -12, 4);
    assert_eq!(s.as_str(), "-012");
    let mut s = CompactString::new("");
    push_int_padded_into(&mut s, 12345, 4);
    assert_eq!(s.as_str(), "+12345");
}

#[test]
fn civil_roundtrip_all_days_1970_2100() {
    for d in 0..47_482i64 {
        let (y, m, day) = civil_from_days(d);
        assert_eq!(
            days_from_civil(i64::from(y), u32::from(m), u32::from(day)),
            d,
            "roundtrip {d}"
        );
    }
}

#[test]
fn dow_known_anchors() {
    assert_eq!(dow_from_days(0), 4);
    assert_eq!(dow_from_days(days_from_civil(2000, 2, 29)), 2);
    assert_eq!(dow_from_days(days_from_civil(2026, 9, 15)), 2);
}

#[test]
fn leap_century_rules() {
    assert_eq!(days_in_month(1900, 2), 28);
    assert_eq!(days_in_month(2000, 2), 29);
    assert_eq!(days_in_month(2100, 2), 28);
    assert_eq!(days_in_month(2024, 2), 29);
    assert_eq!(days_in_month(2023, 2), 28);

    assert_eq!(
        days_from_civil(2025, 1, 1) - days_from_civil(2024, 1, 1),
        366
    );
}

#[test]
fn push_int_padded_fmt_compat() {
    let mut s = CompactString::new("");
    push_int_padded_into(&mut s, -1234, 5);
    assert_eq!(s.as_str(), "-1234");
    let mut s = CompactString::new("");
    push_int_padded_into(&mut s, -12, 6);
    assert_eq!(s.as_str(), "-00012");
}

#[test]
fn bench_scale_is_single_law() {
    assert!((bench::bench_scale(1.0, 0.0) - 1.0).abs() < 1e-12);
    assert!((bench::bench_scale(2.0, 0.03) - 2.06).abs() < 1e-12);
    assert_eq!(bench::scale_clamp(f64::NAN), 1.0);
    assert_eq!(bench::scale_clamp(9.0), bench::CPU_SCALE_MAX);
    assert_eq!(bench::scale_clamp(0.1), bench::CPU_SCALE_MIN);
    assert!((bench::CPU_SCALE_MIN..=bench::CPU_SCALE_MAX).contains(&bench::scale_clamp(1.7)));
}
