#![allow(dead_code)]

use bytes::Bytes;
use runtime_exec::{Bundle, ExecKind, ExecReq, ProfileSnap, WorkerPool};
use session_state::{Family, NetKind, Platform, Profile};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub fn chrome_profile() -> Profile {
    Profile {
        ua: Arc::from(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36",
        ),
        sec_ch_ua: Arc::from(
            r#""Google Chrome";v="149", "Chromium";v="149", "Not)A;Brand";v="24""#,
        ),
        accept_language: Arc::from("en-US,en;q=0.9"),
        platform: Platform::Windows,
        locale: "en-US".into(),
        tz: "America/New_York".into(),
        screen_w: 1920,
        screen_h: 1080,
        canvas_seed: 0x5EED,
        asn: 7922,
        net: NetKind::Residential,
        family: Family::Chrome { major: 149 },
        display_hz: 60,
        ..session_state::Profile::shell()
    }
}

pub fn polyfill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../supervisor/assets/polyfill.js")
}

pub fn pool() -> WorkerPool {
    let bundle = Arc::new(Bundle::open(polyfill_path()).expect("polyfill present"));
    let (tx, _rx) = crossbeam_channel::bounded(8192);
    WorkerPool::spawn(1, bundle, tx, 128).expect("pool spawns")
}

pub fn req(script: &str, timeout_ms: u64) -> ExecReq {
    req_with_profile(script, &chrome_profile(), timeout_ms)
}

pub fn req_with_profile(script: &str, profile: &Profile, timeout_ms: u64) -> ExecReq {
    let profile = Arc::new(profile.clone());
    ExecReq {
        domain: 0xBEEF,
        script: Bytes::copy_from_slice(script.as_bytes()),
        snap: ProfileSnap::from_parts(&profile, "https://mock.local/page", "sid=1"),
        timeout: Duration::from_millis(timeout_ms),
        doc: None,
        input: None,
        net_slot: 0,
        script_node: None,
        kind: ExecKind::Js,
    }
}

pub fn test_profile() -> Profile {
    Profile {
        ua: Arc::from(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36",
        ),
        sec_ch_ua: Arc::from(r#""Google Chrome";v="149", "Chromium";v="149""#),
        accept_language: Arc::from("en-US,en;q=0.9"),
        platform: Platform::Windows,
        locale: "en-US".into(),
        tz: "America/New_York".into(),
        screen_w: 1920,
        screen_h: 1080,
        canvas_seed: 0x5EED,
        asn: 7922,
        net: NetKind::Residential,
        family: Family::Chrome { major: 149 },
        display_hz: 60,
        ..session_state::Profile::shell()
    }
}

pub fn snap() -> runtime_exec::ProfileSnap {
    runtime_exec::ProfileSnap::from_parts(
        &Arc::new(test_profile()),
        "https://mock.local/page",
        "sid=1",
    )
}

pub fn page_doc() -> Arc<parser_pipeline::PageData> {
    let html = b"<html><head><title>T</title><script src=\"https://cdn.local/a.js\"></script></head><body><div id=\"gate\" class=\"box\">gate label<input type=\"hidden\" name=\"csrf\" value=\"tok123\"></div><form action=\"/go\" method=\"POST\"></form></body></html>";
    let mut p = parser_pipeline::StreamPipeline::new(Default::default());
    p.push(html).expect("doc parses");
    Arc::new(p.finish().expect("doc finish"))
}
