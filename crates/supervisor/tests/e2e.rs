use bytes::Bytes;
use core_utils::sha256_hex_into;
use net_client::{EngineSet, engine_catalog, fetch_page};
use runtime_exec::{Bundle, ExecKind, ExecReq, ProfileSnap, WorkerPool};
use session_state::{Family, NetKind, Platform, Profile, Session};
use smallvec::SmallVec;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

const PAGE: &str = r#"<!doctype html><html><head><title>Gate</title>
<script id="__NEXT_DATA__" type="application/json">{"page":"/gate","buildId":"e42","query":"next=1"}</script>
</head><body>
<form action="/submit" method="POST">
<input type="hidden" name="csrf" value="ct_8842aa">
<input type="text" name="login" value="">
</form>
<script>
var _pQ7 = 4242, _rT9 = 1717;
var entropy = [3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5, 8, 9, 7, 9, 3, 2, 3, 8, 4, 6, 2, 6, 4, 3, 3, 8, 3, 2, 7, 9, 5];
var acc = 0;
for (var i = 0; i < 32; i++) { acc += entropy[i] * (i + 2); acc = acc ^ (acc << 2); acc = acc >>> 1; }
eval("1"); atob("YQ=="); setTimeout(function(){}, 1);
fetch("/telemetry", {method: "POST"});
String.fromCharCode(75), "gate".charCodeAt(0);
__silo_sha256("gate:" + _pQ7 + ":" + _rT9 + ":" + acc);
</script>
</body></html>"#;

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[derive(Clone, Default)]
struct TelemetryStats {
    batches: Arc<AtomicU32>,
    bytes: Arc<AtomicU64>,
}

async fn mock_gate(listener: TcpListener, stats: TelemetryStats) {
    loop {
        let (mut sock, _) =
            match tokio::time::timeout(Duration::from_millis(750), listener.accept()).await {
                Ok(Ok(conn)) => conn,
                _ => break,
            };
        let mut buf = vec![0u8; 8192];
        let mut raw = Vec::new();
        loop {
            let n = sock.read(&mut buf).await.expect("read");
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
            let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n");
            if let Some(h) = head_end {
                if !raw.starts_with(b"POST") {
                    break;
                }
                let headers = String::from_utf8_lossy(&raw[..h]).to_uppercase();
                let len: usize = headers
                    .lines()
                    .find(|l| l.starts_with("CONTENT-LENGTH:"))
                    .and_then(|l| l.split(':').nth(1))
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                if raw.len() >= h + 4 + len {
                    break;
                }
            }
        }
        let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n");
        let path = head_end
            .map(|h| String::from_utf8_lossy(&raw[..h]).to_string())
            .and_then(|head| head.split_whitespace().nth(1).map(str::to_string))
            .unwrap_or_default();
        if raw.starts_with(b"POST") && path.starts_with("/telemetry") {
            let head_end = head_end.expect("head");
            let body = &raw[head_end + 4..];
            stats.batches.fetch_add(1, Ordering::Relaxed);
            stats.bytes.fetch_add(body.len() as u64, Ordering::Relaxed);
            let resp = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            sock.write_all(resp.as_bytes())
                .await
                .expect("write telemetry");
        } else if raw.starts_with(b"POST") {
            let head_end = head_end.expect("head");
            let body = &raw[head_end + 4..];
            let text = String::from_utf8_lossy(body);
            let head = String::from_utf8_lossy(&raw[..head_end]).to_uppercase();
            let ok = text.contains("csrf=ct_8842aa")
                && text.contains("sig=")
                && head.contains("COOKIE: SID=E2X");
            let status = if ok { "200 OK" } else { "403 Forbidden" };
            let payload = if ok { "accepted" } else { "rejected" };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            sock.write_all(resp.as_bytes()).await.expect("write post");
        } else if raw.starts_with(b"GET") && path.starts_with("/api/data") {
            let body = "{\"v\":1}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: API=9; Path=/\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.expect("write api");
        } else {
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nSet-Cookie: SID=E2X; Path=/\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                PAGE.len(),
                PAGE
            );
            sock.write_all(resp.as_bytes()).await.expect("write page");
        }
        let _ = sock.shutdown().await;
    }
}

fn entropy_accum() -> i64 {
    let e = [
        3i64, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5, 8, 9, 7, 9, 3, 2, 3, 8, 4, 6, 2, 6, 4, 3, 3, 8, 3, 2,
        7, 9, 5,
    ];
    let mut acc: i64 = 0;
    for (i, val) in e.iter().enumerate() {
        acc += val * (i as i64 + 2);
        let a32 = acc as u32 as i32;
        let x = a32 ^ a32.wrapping_shl(2);
        acc = ((x as u32) >> 1) as i64;
    }
    acc
}

fn profile() -> Profile {
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
        canvas_seed: 0xABC,
        asn: 7922,
        net: NetKind::Residential,
        family: Family::Chrome { major: 149 },
        display_hz: 60,
        ..session_state::Profile::shell()
    }
}

#[tokio::test]
async fn full_cycle_gate_page_to_token_to_submit() {
    unsafe {
        std::env::set_var("SILO_ALLOW_PRIVATE_NETWORK", "1");
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let stats = TelemetryStats::default();
    let server = tokio::spawn(mock_gate(listener, stats.clone()));
    let url = format!("http://{addr}/gate");

    let catalog = engine_catalog().expect("catalog");
    let engines = Arc::new(EngineSet::build(&catalog).expect("engines"));
    let mut session = Session::new(Arc::new(profile()), url.as_str());

    let f = tokio::time::timeout(
        Duration::from_secs(10),
        fetch_page(&engines, 0, &mut session, &url),
    )
    .await
    .expect("deadline")
    .expect("fetch gate");
    assert_eq!(f.status, 200);
    assert_eq!(f.page.title.as_deref(), Some("Gate"));
    assert_eq!(f.page.tokens.first().map(|t| t.as_str()), Some("ct_8842aa"));
    assert_eq!(f.page.next_data.as_ref().unwrap().build_id, "e42");
    assert_eq!(session.jar.get("SID"), Some("E2X"));

    let challenge: Bytes = f.page.challenge.clone().expect("challenge captured");
    let text = std::str::from_utf8(challenge.as_ref()).unwrap_or("");
    assert!(text.contains("gate:"));

    let (tx, rx) = crossbeam_channel::bounded(64);
    let drain = std::thread::spawn(move || for _ in rx.try_iter() {});
    let bundle = Arc::new(Bundle::from_source(Arc::from(include_str!(
        "../assets/polyfill.js"
    ))));
    let pool = WorkerPool::spawn(1, bundle, tx, 32).expect("pool");
    let mut cookie_buf: SmallVec<[u8; 256]> = SmallVec::new();
    session.jar.header_into(&mut cookie_buf);
    let snap = ProfileSnap::from_parts(
        &session.profile,
        f.uri.as_str(),
        std::str::from_utf8(&cookie_buf).unwrap_or(""),
    );
    let outcome = pool
        .exec(ExecReq {
            domain: core_utils::xxh3::hash(b"gate.local"),
            script: challenge.clone(),
            snap,
            timeout: Duration::from_millis(500),
            doc: Some(Arc::clone(&f.page)),
            input: None,
            net_slot: 0,
            script_node: None,
            kind: ExecKind::Js,
        })
        .await;
    let token = outcome.token.expect("gate token");
    assert_eq!(outcome.path, runtime_exec::ExecPath::Compile);

    let acc: i64 = entropy_accum();
    let mut expect = [0u8; 64];
    let mut input = String::with_capacity(64);
    use std::fmt::Write as _;
    let _ = write!(&mut input, "gate:4242:1717:{acc}");
    sha256_hex_into(input.as_bytes(), &mut expect);
    assert_eq!(token.as_str(), std::str::from_utf8(&expect).unwrap());

    let replay = pool
        .exec(ExecReq {
            domain: core_utils::xxh3::hash(b"gate.local"),
            script: challenge.clone(),
            snap: ProfileSnap::from_parts(&session.profile, f.uri.as_str(), ""),
            timeout: Duration::from_millis(500),
            doc: Some(Arc::clone(&f.page)),
            input: None,
            net_slot: 0,
            script_node: None,
            kind: ExecKind::Js,
        })
        .await;
    assert_eq!(replay.path, runtime_exec::ExecPath::RawHit);
    assert_eq!(replay.token.expect("replay token").as_str(), token.as_str());

    let mut submit = String::with_capacity(64);
    let _ = write!(submit, "login=neo&csrf=ct_8842aa&sig={token}");
    let client = wreq::Client::builder().build().expect("client");
    let resp = client
        .post(format!("http://{addr}/submit"))
        .header("Cookie", "SID=E2X")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(submit)
        .send()
        .await
        .expect("submit");
    assert_eq!(resp.status().as_u16(), 200);
    let body = resp.text().await.expect("body");
    assert_eq!(body, "accepted");

    let route = f
        .page
        .telemetry_route
        .clone()
        .expect("in-house route from inline hint");
    assert_eq!(route.provider, parser_pipeline::TelemetryProvider::InHouse);
    let mut fleet = supervisor::fleet::Fleet::new(engines.clone());
    let prof = session.profile.clone();
    let tab = fleet.attach(prof, url.as_str(), &session.jar, route, 0, 8);
    assert_ne!(tab, u32::MAX);
    let base = core_utils::unix_us();
    let mut pushed = 0u32;
    let mut jobs: smallvec::SmallVec<[supervisor::fleet::PushJob; 8]> = smallvec::SmallVec::new();
    for step in 1..=40u64 {
        fleet.pump(base + step * 700_000, &mut jobs);
        for job in &jobs {
            let (status, _set_cookies) =
                supervisor::fleet::Fleet::push_job(&engines, job).await;
            assert_eq!(status, 204);
            fleet.calibrate(job.handle, true);
            pushed += 1;
        }
        jobs.clear();
        if pushed >= 2 {
            break;
        }
    }
    assert!(pushed >= 2, "флот обязан лить точки на сайт: {pushed}");
    assert!(
        stats.bytes.load(Ordering::Relaxed) >= pushed as u64 * 8,
        "байты событий реально получены сайтом: {}",
        stats.bytes.load(Ordering::Relaxed)
    );
    assert!(stats.batches.load(Ordering::Relaxed) >= 2);
    assert!(fleet.live_tabs() >= 1);
    let st = fleet.stats();
    assert!(st.events > 0, "флот живёт: {st:?}");

    server.await.expect("server join");
    drop(drain);
}

#[tokio::test]
async fn fetch_bridge_serves_real_http_with_cookies() {
    unsafe {
        std::env::set_var("SILO_ALLOW_PRIVATE_NETWORK", "1");
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(mock_gate(listener, TelemetryStats::default()));
    let url = format!("http://{addr}/api/data");

    let catalog = engine_catalog().expect("catalog");
    let engines = Arc::new(EngineSet::build(&catalog).expect("engines"));
    supervisor::bridge::spawn_fetch_daemon(engines);

    let (tx, rx) = crossbeam_channel::bounded(64);
    let drain = std::thread::spawn(move || for _ in rx.try_iter() {});
    let bundle = Arc::new(Bundle::from_source(Arc::from(include_str!(
        "../assets/polyfill.js"
    ))));
    let pool = WorkerPool::spawn(1, bundle, tx, 32).expect("pool");

    let script = format!(
        "var out = \"none\";\
         fetch(\"{url}\").then(function (r) {{ r.text().then(function (t) {{ globalThis.__fr = r.status + \"|\" + t; }}); }});\
         \"sent\";"
    );
    let outcome = pool
        .exec(ExecReq {
            domain: 1,
            script: bytes::Bytes::from(script.into_bytes()),
            snap: ProfileSnap::from_parts(&std::sync::Arc::new(profile()), &url, ""),
            timeout: Duration::from_millis(2000),
            doc: None,
            input: None,
            net_slot: 0,
            script_node: None,
            kind: ExecKind::Js,
        })
        .await;
    assert_eq!(
        outcome.token.as_deref(),
        Some("sent"),
        "bridge: {:?}",
        outcome.err
    );
    assert!(
        outcome
            .cookie_out
            .as_deref()
            .is_some_and(|c| c.contains("API=9")),
        "set-cookie must reach cookie_out: {:?}",
        outcome.cookie_out
    );

    let check = pool
        .exec(ExecReq {
            domain: 1,
            script: bytes::Bytes::from_static(
                b"globalThis.__fr === \"200|{\\\"v\\\":1}\" ? \"real\" : String(globalThis.__fr);",
            ),
            snap: ProfileSnap::from_parts(&std::sync::Arc::new(profile()), &url, ""),
            timeout: Duration::from_millis(2000),
            doc: None,
            input: None,
            net_slot: 0,
            script_node: None,
            kind: ExecKind::Js,
        })
        .await;
    assert_eq!(
        check.token.as_deref(),
        Some("real"),
        "fetch bridge body: {:?} => {:?}",
        check.err,
        check.token
    );

    let _ = server.await;
    drop(drain);
}
