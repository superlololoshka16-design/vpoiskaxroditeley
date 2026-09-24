use net_client::{engine_catalog, engine_catalog_with_proxies, reslot_with_asn};
use session_state::Profile;

#[test]
fn catalog_is_coherent_per_entry() {
    let catalog = engine_catalog().expect("catalog");
    assert!(catalog.len() >= 6);
    for entry in &catalog {
        let p: &Profile = entry.profile.as_ref();
        assert!(p.validate().is_ok(), "entry must be coherent");
        assert!(p.ua.starts_with("Mozilla/5.0"));
    }
    let mut seen = std::collections::HashSet::new();
    for entry in &catalog {
        assert!(seen.insert(entry.profile.ua.to_string()), "uas must differ");
    }
}

#[test]
fn reslot_rebuilds_locale_tz_from_asn() {
    let catalog = engine_catalog().expect("catalog");
    let base = &catalog[0].profile;
    let reslotted = reslot_with_asn(base, 24940);
    assert_eq!(reslotted.tz.as_str(), "Europe/Berlin");
    assert_eq!(reslotted.locale.as_str(), "de-DE");
    assert_eq!(reslotted.asn, 24940);
    let keep = reslot_with_asn(base, 7922);
    assert_eq!(keep.tz.as_str(), "America/New_York");
    assert!(reslotted.validate().is_ok());
}

#[test]
fn catalog_rejects_wreq_incompatible_proxy() {
    let err = match engine_catalog_with_proxies(&[
        "socks5h://user:pass@proxy.example:1080".to_string()
    ]) {
        Err(e) => e,
        Ok(_) => panic!("socks5h обязан отказывать, а не строить DIRECT-клиента"),
    };
    assert!(err.contains("refusing to go direct"), "ошибка: {err}");

    assert!(
        engine_catalog_with_proxies(&[
            "socks5://proxy.example:1080".to_string(),
            "http://proxy.example:3128".to_string(),
        ])
        .is_ok()
    );
}
