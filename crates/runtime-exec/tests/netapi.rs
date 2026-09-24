use compact_str::CompactString;
use runtime_exec::{
    FetchReply, install as bridge_install,
};
use session_state::{Family, NetKind, Platform, Profile};
use std::sync::Arc;
mod common;
use common::{chrome_profile, pool, req_with_profile as req};

fn firefox_profile() -> std::sync::Arc<Profile> {
    std::sync::Arc::new(Profile {
        ua: Arc::from(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:151.0) Gecko/20100101 Firefox/151.0",
        ),
        sec_ch_ua: Arc::from(""),
        accept_language: Arc::from("en-US,en;q=0.5"),
        platform: Platform::Windows,
        locale: "en-US".into(),
        tz: "America/New_York".into(),
        screen_w: 1920,
        screen_h: 1080,
        canvas_seed: 0xFA1E,
        asn: 7922,
        net: NetKind::Residential,
        family: Family::Firefox { major: 151 },
        display_hz: 60,
        ..session_state::Profile::shell()
    })
}

fn spawn_bridge() {
    let (tx, rx) = crossbeam_channel::bounded(8);
    bridge_install(tx);
    std::thread::spawn(move || {
        for job in rx {
            let body = bytes::Bytes::from_static(b"bridge-body");
            let _ = job.reply.send(FetchReply {
                status: 200,
                headers: smallvec::smallvec![
                    (
                        CompactString::const_new("Content-Type"),
                        CompactString::const_new("text/plain")
                    ),
                    (
                        CompactString::const_new("X-Probe"),
                        CompactString::const_new("yes")
                    ),
                ],
                set_cookie: smallvec::smallvec![CompactString::const_new("bridge=1; Path=/")],
                body,
            });
        }
    });
}

#[tokio::test]
async fn xhr_routes_through_bridge_with_async_events_and_cookies() {
    spawn_bridge();
    let pool = pool();
    let script = "var x = new XMLHttpRequest();\n\
        var out = \"born:\" + x.readyState;\n\
        x.open(\"GET\", \"https://mock.local/api\");\n\
        out += \":open:\" + x.readyState;\n\
        x.setRequestHeader(\"X-Test\", \"1\");\n\
        x.onreadystatechange = function () { globalThis.__rs = (globalThis.__rs || 0) + 1; };\n\
        x.onload = function (e) {\n\
            globalThis.__out = \"done:\" + x.status + \":\" + x.responseText + \":\" + (x.getResponseHeader(\"x-probe\") || \"none\") + \":rs\" + x.readyState + \":\" + (e.isTrusted === true);\n\
        };\n\
        x.onerror = function () { globalThis.__out = \"err\"; };\n\
        x.send();\n\
        out;";
    let o = pool.exec(req(script, &chrome_profile(), 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("born:0:open:1"),
        "xhr sync phase: {:?}",
        o.err
    );
    assert!(
        o.cookie_out.is_some(),
        "set-cookie from bridge reply must reach outcome"
    );
    assert_eq!(
        o.cookie_out.as_deref(),
        Some("bridge=1"),
        "cookie value: {:?}",
        o.cookie_out
    );

    let read = "globalThis.__out || \"pending\";";
    let o = pool.exec(req(read, &chrome_profile(), 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("done:200:bridge-body:yes:rs4:true"),
        "xhr async completion: {:?}",
        o.err
    );
    let rs = "globalThis.__rs >= 1;";
    let o = pool.exec(req(rs, &chrome_profile(), 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true"),
        "readystatechange fired: {:?}",
        o.err
    );
}

#[tokio::test]
async fn xhr_prototype_surface_is_native_shaped() {
    spawn_bridge();
    let pool = pool();
    let script = "var x = new XMLHttpRequest();\
        var own = Object.getOwnPropertyNames(x).length;\
        var proto = Object.getPrototypeOf(x);\
        var ctor = proto.constructor === XMLHttpRequest;\
        var tag = Object.prototype.toString.call(x);\
        var ts = XMLHttpRequest.prototype.send.toString().indexOf(\"[native code]\") >= 0;\
        \"own:\" + own + \":ctor:\" + ctor + \":tag:\" + tag + \":native:\" + ts;";
    let o = pool.exec(req(script, &chrome_profile(), 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("own:0:ctor:true:tag:[object XMLHttpRequest]:native:true"),
        "xhr native surface: {:?}",
        o.err
    );
}

#[tokio::test]
async fn rtc_sdp_is_deterministic_and_leaks_no_ip() {
    let pool = pool();
    let script = "var pc = new RTCPeerConnection({});\
        var ice = \"\";\
        pc.onicecandidate = function (e) {\
            if (e.candidate && e.candidate.candidate) { ice = e.candidate.candidate; }\
        };\
        var offer = pc.createOffer();\
        offer.then(function (o) {\
            var sdp = o.sdp;\
            var has_mdns = sdp.indexOf(\".local\") >= 0;\
            var ip_zero = sdp.indexOf(\"c=IN IP4 0.0.0.0\") >= 0;\
            var fp = sdp.indexOf(\"a=fingerprint:sha-256 00:00:00:00:00:00:00:00\") < 0;\
            window.__out = \"type:\" + o.type + \":mdns:\" + has_mdns + \":zero:\" + ip_zero + \":fp:\" + fp + \":ice:\" + (ice.indexOf(\".local\") >= 0) + \":state:\" + pc.connectionState;\
        });\
        \"\";";
    let o = pool.exec(req(script, &chrome_profile(), 2000)).await;
    assert!(o.err.is_none(), "rtc exec: {:?}", o.err);

    let probe = "window.__out || \"pending\";";
    let o = pool.exec(req(probe, &chrome_profile(), 2000)).await;
    assert_ne!(
        o.token.as_deref(),
        Some("pending"),
        "rtc promise did not resolve"
    );

    let again = "var pc2 = new RTCPeerConnection({});\
        var a = null, b = null;\
        var p1 = new Promise(function (res) { var pc = new RTCPeerConnection({}); pc.createOffer().then(function (o) { a = o.sdp; res(1); }); });\
        var p2 = new Promise(function (res) { var pc = new RTCPeerConnection({}); pc.createOffer().then(function (o) { b = o.sdp; res(1); }); });\
        Promise.all([p1, p2]).then(function () { window.__cmp = (a === b); });\
        \"\";";
    let o = pool.exec(req(again, &chrome_profile(), 2000)).await;
    assert!(o.err.is_none(), "rtc cmp exec: {:?}", o.err);
}

#[tokio::test]
async fn ua_data_follows_profile_family() {
    let pool = pool();
    let chrome = "var out = navigator.userAgentData;\
        var brand0 = out && out.brands && out.brands.length;\
        var plat = out && out.platform;\
        out ? \"chrome:\" + brand0 + \":\" + plat : \"none\";";
    let o = pool.exec(req(chrome, &chrome_profile(), 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("chrome:3:Windows"),
        "chrome uaData: {:?}",
        o.err
    );

    let ff = "typeof navigator.userAgentData + \":\" + (typeof window.chrome);";
    let o = pool.exec(req(ff, &firefox_profile(), 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("undefined:undefined"),
        "firefox must hide chrome surface: {:?}",
        o.err
    );

    let hentropy = "navigator.userAgentData.getHighEntropyValues([\"platformVersion\", \"uaFullVersion\"]).then(function (v) { window.__h = v.platformVersion + \"|\" + v.uaFullVersion; }); \"\";";
    let o = pool.exec(req(hentropy, &chrome_profile(), 2000)).await;
    assert!(o.err.is_none(), "hentropy exec: {:?}", o.err);
    let read = "window.__h || \"none\";";
    let o = pool.exec(req(read, &chrome_profile(), 2000)).await;
    let tok = o.token.clone();
    assert!(
        matches!(tok.as_deref(), Some(t) if t.starts_with("15.0.0|149.0.")),
        "high entropy values: {:?} token={:?}",
        o.err,
        tok
    );
}

#[tokio::test]
async fn screen_orientation_follows_profile_geometry() {
    let pool = pool();
    let script = "screen.orientation.type + \":\" + screen.orientation.angle;";
    let o = pool.exec(req(script, &chrome_profile(), 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("landscape-primary:0"),
        "orientation: {:?}",
        o.err
    );
}

#[tokio::test]
async fn current_script_null_without_doc_binding() {
    let pool = pool();
    let script = "document.currentScript === null;";
    let o = pool.exec(req(script, &chrome_profile(), 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true"),
        "currentScript null without binding: {:?}",
        o.err
    );
}

#[tokio::test]
async fn current_script_resolves_inline_challenge_node() {
    let pool = pool();
    let padding = "var q".to_string() + &"1".repeat(420) + " = 1;";
    let html = format!(
        "<html><head><title>T</title><script id=\"gate-script\">var a = atob(\"eA==\"); eval(\"1\"); setTimeout(function () {{}}, 0); var s = \"x\".charCodeAt(0); {padding}</script></head><body><div id=\"gate\"></div></body></html>"
    );
    let mut p = parser_pipeline::StreamPipeline::new(Default::default());
    p.push(html.as_bytes()).expect("doc parses");
    let doc = Arc::new(p.finish().expect("doc finish"));
    assert!(doc.challenge.is_some(), "inline challenge must be detected");
    assert_eq!(
        doc.challenge_node, doc.challenge_node,
        "node binding stable"
    );
    let script = "(document.currentScript && document.currentScript.tagName) + \":\" + (document.currentScript && document.currentScript.id);";
    let mut r = req(script, &chrome_profile(), 2000);
    r.doc = Some(Arc::clone(&doc));
    r.script_node = doc.challenge_node;
    let o = pool.exec(r).await;
    assert_eq!(
        o.token.as_deref(),
        Some("SCRIPT:gate-script"),
        "currentScript node: {:?}",
        o.err
    );
}
