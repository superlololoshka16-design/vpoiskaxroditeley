use net_client::{EngineSet, engine_catalog, fetch_page, reslot_with_asn};
use session_state::Session;
use std::time::Duration;

#[tokio::test]
#[ignore = "hits live network"]
async fn live_example_com_parses_in_single_pass() {
    let catalog = engine_catalog().expect("catalog");
    let engines = EngineSet::build(&catalog).expect("engines");
    let profile = reslot_with_asn(catalog[0].profile.as_ref(), 0);
    let mut session = Session::new(profile, "https://example.com");
    let f = tokio::time::timeout(
        Duration::from_secs(30),
        fetch_page(&engines, 0, &mut session, "https://example.com"),
    )
    .await
    .expect("deadline")
    .expect("fetch ok");
    assert_eq!(f.status, 200);
    assert_eq!(f.page.title.as_deref(), Some("Example Domain"));
    assert!(f.bytes_in > 500);
    assert_eq!(f.page.utf8_bad_chunks, 0);
    assert!(!f.page.truncated);
}
