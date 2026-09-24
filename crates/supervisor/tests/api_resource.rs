use net_client::{EngineSet, RobotsCache, engine_catalog};
use runtime_exec::{Bundle, WorkerPool};
use sonic_rs::JsonValueTrait as _;
use std::sync::Arc;
use std::time::Duration;
use supervisor::api::{AppSpawn, AppState};
use supervisor::flow::VisitCtx;
use supervisor::site_override::SiteOverrides;
use supervisor::stats::StatBlock;
use supervisor::task::{TaskKind, TaskOutcome};

fn spawn_state(timeout: Duration) -> Arc<AppState> {
    let catalog = engine_catalog().expect("catalog");
    let engines = Arc::new(EngineSet::build(&catalog).expect("engines"));
    let (tx, _rx) = crossbeam_channel::bounded(64);
    let bundle = Arc::new(Bundle::from_source(Arc::from(include_str!(
        "../assets/polyfill.js"
    ))));
    let pool = Arc::new(WorkerPool::spawn(1, bundle, tx, 16).expect("pool"));
    AppState::spawn(AppSpawn {
        workers: 1,
        monitor: parser_pipeline::VersionMonitor::new(),
        ctx: VisitCtx {
            engines,
            profiles: catalog.iter().map(|c| c.profile.clone()).collect(),
            pool,
            stats: Arc::new(StatBlock::new()),
            timeout,
            asn: 0,
            overrides: Arc::new(SiteOverrides::empty()),
            robots: Arc::new(RobotsCache::new()),
        },
    })
}

#[tokio::test]
async fn expired_task_reports_failure_and_bad_url_is_rejected() {
    let state = spawn_state(Duration::ZERO);

    let err = state
        .create_task(TaskKind::HtmlFetch {
            proxy: None,
            url: "htp:/::bad".into(),
        })
        .await;
    assert!(err.is_err());

    let id = state
        .create_task(TaskKind::HtmlFetch {
            proxy: None,
            url: "https://example.com/".into(),
        })
        .await
        .expect("task registered");
    match state.task_outcome(id) {
        TaskOutcome::Failed { error } => assert_eq!(error, "expired"),
        other => panic!("expired task must fail, got {:?}", other),
    }
}

#[tokio::test]
async fn solve_with_unknown_profile_id_fails_with_profile_error() {
    let state = spawn_state(Duration::from_millis(2_000));
    let id = state
        .create_task(TaskKind::Solve {
            kind: "captcha".into(),
            script: None,
            payload_b64: None,
            deadline_ms: None,
            profile_id: Some(999_999),
            proxy: None,
        })
        .await
        .expect("task registered");
    let error = loop {
        match state.task_outcome(id) {
            TaskOutcome::Failed { error } => break error,
            TaskOutcome::Ready { .. } => panic!("solve must fail, got ready"),
            TaskOutcome::Processing { .. } => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    };
    assert_eq!(error, "profile not found");
}

#[tokio::test]
async fn http_routes_reject_garbage_and_report_unknown_tasks() {
    let state = spawn_state(Duration::from_millis(2_000));
    let app = supervisor::api::router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let client = wreq::Client::builder().build().expect("client");
    let base = format!("http://{addr}");

    let health = client
        .get(format!("{base}/health"))
        .send()
        .await
        .expect("health");
    assert_eq!(health.status().as_u16(), 200);
    assert_eq!(
        health.text().await.expect("health body"),
        "my-engine: alive"
    );

    let empty = client
        .post(format!("{base}/createTask"))
        .body("")
        .send()
        .await
        .expect("empty body");
    assert_eq!(empty.status().as_u16(), 400);
    let v: sonic_rs::Value =
        sonic_rs::from_str(&empty.text().await.expect("error body")).expect("error json");
    assert!(
        v["error"]
            .as_str()
            .is_some_and(|e| e.starts_with("bad json"))
    );

    let bad_url = client
        .post(format!("{base}/createTask"))
        .header("content-type", "application/json")
        .body(r#"{"task":{"type":"htmlFetch","url":"ftp://nope/"}}"#)
        .send()
        .await
        .expect("bad url");
    assert_eq!(bad_url.status().as_u16(), 400);
    let v: sonic_rs::Value =
        sonic_rs::from_str(&bad_url.text().await.expect("bad url body")).expect("error json");
    assert_eq!(v["error"].as_str(), Some("bad url"));

    let missing = client
        .post(format!("{base}/getTaskResult"))
        .header("content-type", "application/json")
        .body(r#"{"task_id":424242}"#)
        .send()
        .await
        .expect("missing task");
    assert_eq!(missing.status().as_u16(), 200);
    let v: sonic_rs::Value =
        sonic_rs::from_str(&missing.text().await.expect("missing body")).expect("result json");
    assert_eq!(v["status"].as_str(), Some("failed"));
    assert_eq!(v["error"].as_str(), Some("not found"));

    let stats = client
        .get(format!("{base}/stats"))
        .send()
        .await
        .expect("stats");
    assert_eq!(stats.status().as_u16(), 200);
    let v: sonic_rs::Value =
        sonic_rs::from_str(&stats.text().await.expect("stats body")).expect("stats json");
    assert!(v["tasksInRegistry"].is_number());
    assert!(v["uptimeSecs"].is_number());
    assert!(v["monitorAlerts"].is_number());
}
