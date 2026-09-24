use std::sync::Arc;
use std::time::{Duration, Instant};

use net_client::EngineSet;
use parser_pipeline::{TelemetryProvider, TelemetryRoute, Transport};
use session_state::{CookieJar, Family, NetKind, Platform, Profile};

use supervisor::fleet::Fleet;

fn profile() -> Arc<Profile> {
    Arc::new(Profile {
        ua: Arc::from("test"),
        sec_ch_ua: Arc::from("test"),
        accept_language: Arc::from("en"),
        platform: Platform::Windows,
        locale: "en".into(),
        tz: "UTC".into(),
        screen_w: 800,
        screen_h: 600,
        canvas_seed: 1,
        asn: 0,
        net: NetKind::Residential,
        family: Family::Chrome { major: 149 },
        display_hz: 60,
        ..session_state::Profile::shell()
    })
}

fn route() -> TelemetryRoute {
    TelemetryRoute {
        provider: TelemetryProvider::InHouse,
        endpoint: "/events".into(),
        transport: Transport::CdnPost,
        field: "".into(),
    }
}

fn fleet(max_tabs: usize) -> Fleet {
    Fleet::with_limits(
        Arc::new(EngineSet::build(&[]).expect("engines")),
        max_tabs,
        Duration::from_secs(60),
    )
}

#[test]
fn capacity_is_enforced_and_released_on_detach() {
    let mut fleet = fleet(1);
    let first = fleet.attach(
        profile(),
        "https://example.test",
        &CookieJar::new(),
        route(),
        0,
        1,
    );
    assert_ne!(first, u32::MAX);
    assert_eq!(
        fleet.attach(
            profile(),
            "https://example.test",
            &CookieJar::new(),
            route(),
            0,
            1
        ),
        u32::MAX
    );
    assert!(fleet.detach(first));
    let second = fleet.attach(
        profile(),
        "https://example.test",
        &CookieJar::new(),
        route(),
        0,
        1,
    );
    assert_ne!(second, u32::MAX);
    assert_ne!(first, second);
    assert!(!fleet.detach(first));
    assert_eq!(fleet.live_tabs(), 1);
}

#[test]
fn expiry_releases_tab_site_and_scheduler_work() {
    let mut fleet = fleet(2);
    fleet.attach(
        profile(),
        "https://example.test",
        &CookieJar::new(),
        route(),
        0,
        1,
    );
    assert_eq!(fleet.expire(Instant::now() + Duration::from_secs(61)), 1);
    assert_eq!(fleet.live_tabs(), 0);
    assert_eq!(fleet.next_due_us(), u64::MAX);
}

#[test]
fn closed_handles_do_not_resolve_after_slot_reuse() {
    let mut fleet = fleet(1);
    let first = fleet.attach(
        profile(),
        "https://example.test",
        &CookieJar::new(),
        route(),
        0,
        1,
    );
    assert!(fleet.detach(first));
    let second = fleet.attach(
        profile(),
        "https://example.test",
        &CookieJar::new(),
        route(),
        0,
        1,
    );
    assert!(!fleet.detach(first));
    assert!(fleet.detach(second));
    assert_eq!(fleet.live_tabs(), 0);
}
