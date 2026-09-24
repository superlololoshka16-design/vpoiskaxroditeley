use session_state::Platform;
use session_state::Profile;
use supervisor::site_override::SiteOverrides;

#[test]
fn json_override_applies_fields() {
    let raw = r#"{"example.com":{"ua":"UA-X","tz":"Asia/Tokyo","locale":"ja-JP"}}"#;
    let v: sonic_rs::Value = sonic_rs::from_str(raw).expect("json");
    let ovs = SiteOverrides::from_json(&v);
    assert_eq!(ovs.len(), 1);
    let ov = ovs.for_host("example.com").expect("hit");
    let base = Profile::shell();
    let next = ov.apply(&base);
    assert_eq!(next.ua.as_ref(), "UA-X");
    assert_eq!(next.tz.as_str(), "Asia/Tokyo");
    assert_eq!(next.locale.as_str(), "ja-JP");
    assert!(ovs.for_host("other.com").is_none());
}

#[test]
fn garbage_json_yields_empty() {
    for bad in [
        "",
        "not json",
        "[]",
        "{\"a\":{}}",
        "{\"a\":{\"platform\":\"win3.1\"}}",
    ] {
        let v: Result<sonic_rs::Value, _> = sonic_rs::from_str(bad);
        match v {
            Ok(v) => {
                let ovs = SiteOverrides::from_json(&v);
                assert_eq!(ovs.len(), 0, "no usable overrides from {bad}");
            }
            Err(_) => {
                let ovs = SiteOverrides::empty();
                assert_eq!(ovs.len(), 0);
            }
        }
    }
}

#[test]
fn platform_override_changes_platform() {
    let raw = r#"{"x.test":{"platform":"macos"}}"#;
    let v: sonic_rs::Value = sonic_rs::from_str(raw).expect("json");
    let ovs = SiteOverrides::from_json(&v);
    let next = ovs
        .for_host("x.test")
        .expect("hit")
        .apply(&Profile::shell());
    assert_eq!(next.platform, Platform::MacOS);
}
