use bytes::Bytes;
use runtime_exec::{
    Bundle, ExecError, ExecKind, ExecReq, ProfileSnap, WorkerPool, WorkerPoolLimits,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const LOOP: &str = "while (true) {}";
const OK: &str = "21 + 21;";

fn polyfill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../supervisor/assets/polyfill.js")
}

fn fixture(bytes: usize) -> ExecReq {
    ExecReq {
        domain: 1,
        script: Bytes::from(vec![b'x'; bytes]),
        timeout: Duration::from_secs(60),
        doc: None,
        input: None,
        net_slot: 0,
        script_node: None,
        kind: ExecKind::Js,
        snap: ProfileSnap::from_parts(
            &Arc::new(session_state::Profile {
                ua: Arc::from("test"),
                sec_ch_ua: Arc::from(""),
                accept_language: Arc::from(""),
                platform: session_state::Platform::Windows,
                locale: "en".into(),
                tz: "UTC".into(),
                screen_w: 1,
                screen_h: 1,
                canvas_seed: 0,
                asn: 0,
                net: session_state::NetKind::Datacenter,
                family: session_state::Family::Chrome { major: 149 },
                display_hz: 60,
                ..session_state::Profile::shell()
            }),
            "",
            "",
        ),
    }
}

fn pool(limits: WorkerPoolLimits) -> WorkerPool {
    let bundle = Arc::new(Bundle::open(polyfill_path()).expect("polyfill present"));
    let (tx, _rx) = crossbeam_channel::bounded(8192);
    WorkerPool::spawn_with_limits(1, bundle, tx, limits).expect("pool spawns")
}

#[tokio::test]
async fn dropping_an_unpolled_future_cancels_its_task_and_pool_recovers() {
    let p = pool(WorkerPoolLimits {
        script_bytes: 1 << 20,
        queue_capacity: 8,
        cache_entries: 16,
        cache_bytes: 1 << 16,
    });
    let hang = p.exec(fixture(16));
    drop(hang);
    let mut ok = fixture(16);
    ok.script = Bytes::from_static(OK.as_bytes());
    ok.timeout = Duration::from_millis(2000);
    let out = p.exec(ok).await;
    assert!(
        out.token.is_some(),
        "worker survives cancellation: {:?}",
        out.err
    );
}

#[tokio::test]
async fn script_byte_limit_rejects_before_queueing() {
    let p = pool(WorkerPoolLimits {
        script_bytes: 8,
        queue_capacity: 4,
        cache_entries: 16,
        cache_bytes: 1 << 16,
    });
    let out = p.exec(fixture(9)).await;
    assert!(matches!(out.err, Some(ExecError::Backpressure)));
}

#[tokio::test]
async fn zero_timeout_does_not_enqueue() {
    let p = pool(WorkerPoolLimits {
        script_bytes: 1 << 20,
        queue_capacity: 4,
        cache_entries: 16,
        cache_bytes: 1 << 16,
    });
    let mut req = fixture(8);
    req.timeout = Duration::ZERO;
    let out = p.exec(req).await;
    assert!(matches!(out.err, Some(ExecError::Timeout)));
}

#[tokio::test]
async fn full_queue_releases_failed_admission() {
    let p = pool(WorkerPoolLimits {
        script_bytes: 1 << 20,
        queue_capacity: 1,
        cache_entries: 16,
        cache_bytes: 1 << 16,
    });
    let mut hang = fixture(16);
    hang.script = Bytes::from_static(LOOP.as_bytes());
    hang.timeout = Duration::from_millis(300);
    let first = p.exec(hang);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let mut queued = fixture(16);
    queued.script = Bytes::from_static(OK.as_bytes());
    queued.timeout = Duration::from_millis(3000);
    let second = p.exec(queued);
    let mut third = fixture(16);
    third.script = Bytes::from_static(OK.as_bytes());
    third.timeout = Duration::from_millis(3000);
    let out = p.exec(third).await;
    assert!(matches!(out.err, Some(ExecError::Backpressure)));
    let hang_out = first.await;
    assert!(matches!(hang_out.err, Some(ExecError::Timeout)));
    let queued_out = second.await;
    assert!(
        queued_out.token.is_some(),
        "queued task runs after hang: {:?}",
        queued_out.err
    );
}
