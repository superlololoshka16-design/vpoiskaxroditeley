use compact_str::CompactString;
use session_state::{CookieJar, Family, NetKind, Platform, Profile};
use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::atomic::Ordering;

fn chrome_profile(platform: Platform, ua: &str, sec: &str) -> Profile {
    Profile {
        ua: Arc::from(ua),
        sec_ch_ua: Arc::from(sec),
        accept_language: Arc::from("en-US,en;q=0.9"),
        platform,
        locale: "en-US".into(),
        tz: "America/New_York".into(),
        screen_w: 1920,
        screen_h: 1080,
        canvas_seed: 42,
        asn: 7922,
        net: NetKind::Residential,
        family: Family::Chrome { major: 149 },
        display_hz: 60,
        ..session_state::Profile::shell()
    }
}

const UA_WIN: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";
const UA_MAC: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";
const SEC_149: &str = r#""Google Chrome";v="149", "Chromium";v="149", "Not)A;Brand";v="24""#;

#[test]
fn profile_rejects_platform_ua_mismatch() {
    let mut p = chrome_profile(Platform::MacOS, UA_WIN, SEC_149);
    let err = p.validate().unwrap_err();
    assert!(err.to_string().contains("platform-token"));
    p.platform = Platform::Windows;
    assert!(p.validate().is_ok());
}

#[test]
fn profile_rejects_sec_ch_ua_major_mismatch() {
    let mut p = chrome_profile(
        Platform::Windows,
        UA_WIN,
        r#""Google Chrome";v="148", "Chromium";v="148""#,
    );
    let err = p.validate().unwrap_err();
    assert!(err.to_string().contains("sec-ch-ua-major"));
    p.sec_ch_ua = Arc::from(SEC_149);
    assert!(p.validate().is_ok());
}

#[test]
fn profile_rejects_headless_marks() {
    let mut p = chrome_profile(Platform::Windows, UA_WIN, SEC_149);
    p.ua = Arc::from("Mozilla/5.0 (Windows NT 10.0) HeadlessChrome/149.0.0.0");
    assert!(p.validate().is_err());
}

#[test]
fn profile_rejects_zero_screen_and_bad_locale() {
    let mut p = chrome_profile(Platform::Windows, UA_WIN, SEC_149);
    p.screen_w = 0;
    assert!(p.validate().is_err());
    p.screen_w = 1920;
    p.locale = CompactString::new("enus");
    assert!(p.validate().is_err());
}

#[test]
fn cookie_jar_dedups_and_keeps_order() {
    let mut jar = CookieJar::new();
    jar.ingest("sid=aaa; Path=/; HttpOnly");
    jar.ingest("csrf=bbb");
    jar.ingest("sid=zzz; Max-Age=3600");
    jar.ingest("");
    jar.ingest("novalue");
    jar.ingest("broken");
    assert_eq!(jar.len(), 2);
    let mut out: SmallVec<[u8; 256]> = SmallVec::new();
    jar.header_into(&mut out);
    assert_eq!(out.as_slice(), b"sid=zzz; csrf=bbb");
    assert_eq!(jar.get("sid"), Some("zzz"));
}

#[test]
fn cookie_jar_eats_hostile_garbage_without_panic() {
    let mut jar = CookieJar::new();
    jar.ingest("=nolang");
    jar.ingest("= = =");
    jar.ingest(";");
    jar.ingest(";;;");
    jar.ingest("a=");
    jar.ingest("=b");
    jar.ingest("k=v; ; ;; Path=");
    jar.ingest("  spaced = trimmed  ");
    jar.ingest("\u{0}\u{1}bad=weird\u{0}");
    jar.ingest(&"x".repeat(8192));
    let mut out: SmallVec<[u8; 256]> = SmallVec::new();
    jar.header_into(&mut out);
    assert!(!out.is_empty());
    assert!(jar.get("k").is_some());
}

#[test]
fn profile_rejects_gpu_platform_mismatch() {
    let mut p = chrome_profile(Platform::Windows, UA_WIN, SEC_149);
    p.preset = session_state::Preset::Ryzen3_2200U;
    assert!(p.validate().is_ok());
    p.platform = Platform::MacOS;
    p.ua = Arc::from(UA_MAC);
    let err = p.validate().unwrap_err();
    assert!(
        err.to_string().starts_with("screen/renderer"),
        "amd gpu on mac must fail at renderer check: {err}"
    );
}

#[test]
fn profile_rejects_mac_with_non_apple_gpu() {
    let mut p = chrome_profile(Platform::MacOS, UA_MAC, SEC_149);
    p.preset = session_state::Preset::I5_6300U;
    let err = p.validate().unwrap_err();
    assert!(
        err.to_string().starts_with("screen/renderer"),
        "intel ANGLE gpu on mac must fail at renderer check: {err}"
    );
    p.preset = session_state::Preset::Ryzen3_2200U;
    assert!(p.validate().is_err());
}

#[test]
fn profile_rejects_android_without_hardware_pool() {
    let p = chrome_profile(
        Platform::Android,
        "Mozilla/5.0 (Linux; Android 13; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Mobile Safari/537.36",
        SEC_149,
    );
    let err = p.validate().unwrap_err();
    assert!(
        err.to_string().starts_with("platform/preset"),
        "android has no hardware pool: {err}"
    );
}

#[test]
fn session_tab_ids_strictly_monotonic_and_touch_persists() {
    let p = Arc::new(chrome_profile(Platform::Windows, UA_WIN, SEC_149));
    let a = session_state::Session::new(Arc::clone(&p), "https://a.example");
    let b = session_state::Session::new(p, "https://b.example");
    assert!(b.id.as_raw() > a.id.as_raw());
    assert!(a.hot.last_seen_ms.load(Ordering::Acquire) > 0);
    let seen = a.hot.last_seen_ms.load(Ordering::Acquire);
    a.touch();
    assert!(a.hot.last_seen_ms.load(Ordering::Acquire) >= seen);
}
