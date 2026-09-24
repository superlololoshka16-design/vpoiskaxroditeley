use bytes::Bytes;
use core_utils::sha256_hex_into;
use runtime_exec::{Bundle, ExecError, ExecKind, ExecPath, WorkerPool};
use std::sync::Arc;
mod common;
use common::{page_doc, polyfill_path, pool, req};

const V1: &str = r#"var _aXq7 = 11, _zKp2 = 22;
var t = __silo_sha256("silo:" + _aXq7 + ":" + _zKp2);
t;"#;

const V2: &str = r#"var qq1w = 33, mn4r = 44;
var t = __silo_sha256("silo:" + qq1w + ":" + mn4r);
t;"#;

const LOOP: &str = "while (true) { }";

const OOM: &str = r#"var a = [];
for (var i = 0; i < 400000; i++) { a.push({ id: i, tag: "payload-payload-payload" }); }
a.length;"#;

const WASM_MOD: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7e, 0x03,
    0x02, 0x01, 0x00, 0x07, 0x0a, 0x01, 0x06, 0x61, 0x6e, 0x73, 0x77, 0x65, 0x72, 0x00, 0x00, 0x0a,
    0x06, 0x01, 0x04, 0x00, 0x42, 0x2a, 0x0b,
];







#[tokio::test]
async fn crypto_get_random_values_fills_in_place_with_chrome_errors() {
    let pool = pool();
    let fill = "var a = new Uint8Array(16); var r = crypto.getRandomValues(a); var acc = 0;\
        for (var i = 0; i < 16; i++) { acc += a[i]; }\
        (r === a) * 2 + (acc > 0 ? 1 : 0);";
    let o = pool.exec(req(fill, 1000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("3"),
        "identity + real fill: {:?}",
        o.err
    );

    let quota = "var out;\
        try { crypto.getRandomValues(new Uint8Array(65537)); out = \"no\"; }\
        catch (e) { out = e.message.indexOf(\"QuotaExceededError\") === 0 ? \"quota\" : \"other:\" + e.message; }\
        out;";
    let o = pool.exec(req(quota, 1000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("quota"),
        "chrome quota error: {:?}",
        o.err
    );

    let ty = "var out;\
        try { crypto.getRandomValues({}); out = \"no\"; }\
        catch (e) { out = e.message.indexOf(\"TypeMismatchError\") === 0 ? \"type\" : \"other:\" + e.message; }\
        out;";
    let o = pool.exec(req(ty, 1000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("type"),
        "chrome type error: {:?}",
        o.err
    );
}

#[tokio::test]
async fn dom_bridge_reads_soa_indexes_and_keeps_identity() {
    let pool = pool();
    let script = "var n = document.scripts.length;\
        var s = document.scripts.item(0).getAttribute(\"src\");\
        var el = document.getElementById(\"gate\");\
        var again = document.getElementById(\"gate\");\
        n + \"|\" + (s.indexOf(\"a.js\") >= 0) + \"|\" + el.tagName + \"|\" + el.className + \"|\" + (el === again);";
    let mut r = req(script, 1000);
    r.doc = Some(page_doc());
    let o = pool.exec(r).await;
    assert_eq!(
        o.token.as_deref(),
        Some("1|true|DIV|box|true"),
        "soa -> handles: {:?}",
        o.err
    );
}

#[tokio::test]
async fn dom_bridge_without_doc_degrades_to_empty() {
    let pool = pool();
    let script = "var l = document.scripts.length;\
        var i = document.scripts.item(0);\
        var e = document.getElementById(\"x\");\
        l + \"|\" + (i === null) + \"|\" + (e === null);";
    let o = pool.exec(req(script, 1000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("0|true|true"),
        "no doc: {:?}",
        o.err
    );
}

#[tokio::test]
async fn native_crypto_and_polymorphic_cache_hit() {
    let pool = pool();
    let r = tokio::runtime::Handle::current();
    let _ = r;
    let o1 = pool.exec(req(V1, 1000)).await;
    let tok1 = o1.token.clone().expect("v1 token");
    assert_eq!(o1.path, ExecPath::Compile);
    assert!(!o1.cache_hit);
    let mut expect = [0u8; 64];
    sha256_hex_into(b"silo:11:22", &mut expect);
    assert_eq!(tok1.as_str(), std::str::from_utf8(&expect).unwrap());

    let o2 = pool.exec(req(V2, 1000)).await;
    let tok2 = o2.token.expect("v2 token");
    let mut expect2 = [0u8; 64];
    sha256_hex_into(b"silo:33:44", &mut expect2);
    assert_eq!(tok2.as_str(), std::str::from_utf8(&expect2).unwrap());
    assert_eq!(o2.path, ExecPath::NormHit, "variant must reuse skeleton");
    assert!(o2.cache_hit);

    let o3 = pool.exec(req(V1, 1000)).await;
    assert_eq!(o3.path, ExecPath::RawHit, "exact replay hits raw cache");
    assert_eq!(o3.token.expect("v3 token").as_str(), tok1.as_str());
}

#[tokio::test]
async fn constant_folding_merges_computed_seeds() {
    let a = "var q = 100 + 15; __silo_md5(\"fold:\" + q);";
    let b = "var z = 90 + 25; __silo_md5(\"fold:\" + z);";
    let pool = pool();
    let o1 = pool.exec(req(a, 1000)).await;
    assert!(o1.token.is_some());
    let o2 = pool.exec(req(b, 1000)).await;
    assert_eq!(o2.path, ExecPath::NormHit, "folded literals converge");
    assert_eq!(o1.token, o2.token);
}

#[tokio::test]
async fn infinite_loop_hits_deadline_and_worker_survives() {
    let pool = pool();
    let o = pool.exec(req(LOOP, 120)).await;
    assert!(matches!(o.err, Some(ExecError::Timeout)));
    assert!(o.token.is_none());
    let after = pool.exec(req(V1, 1000)).await;
    assert!(after.token.is_some(), "worker must survive interrupt");
}

#[tokio::test]
async fn oom_rebuilds_engine_and_recovers() {
    let pool = pool();
    let o = pool.exec(req(OOM, 4000)).await;
    assert!(
        matches!(o.err, Some(ExecError::Oom)) || matches!(o.err, Some(ExecError::Timeout)),
        "oom path: {:?}",
        o.err
    );
    let after = pool.exec(req(V1, 1000)).await;
    assert!(after.token.is_some(), "engine must recover after oom");
}

#[tokio::test]
async fn garbage_script_is_graceful_parse_error() {
    let pool = pool();
    let junk = [
        0x01u8, 0x02, b' ', b'n', b'o', b't', b'-', b'j', b's', 0xFF, 0xFE,
    ];
    let mut r = req("x", 500);
    r.script = Bytes::copy_from_slice(&junk);
    let o = pool.exec(r).await;
    assert!(matches!(o.err, Some(ExecError::Parse)));
    let o2 = pool.exec(req(V2, 1000)).await;
    assert!(o2.token.is_some());
}

#[tokio::test]
async fn wasm_module_runs_with_fuel() {
    let pool = pool();
    let mut r = req("x", 1000);
    r.script = Bytes::copy_from_slice(WASM_MOD);
    r.kind = ExecKind::Wasm;
    let o = pool.exec(r).await;
    assert_eq!(o.path, ExecPath::Wasm);
    assert_eq!(o.token.expect("wasm answer").as_str(), "42");
}

#[tokio::test]
async fn math_random_seeded_per_profile() {
    let pool = pool();
    let o = pool
        .exec(req(
            "var a = Math.random(); var b = Math.random(); a + b;",
            1000,
        ))
        .await;
    assert!(o.token.is_some());
    let again = pool
        .exec(req(
            "var a = Math.random(); var b = Math.random(); a + b;",
            1000,
        ))
        .await;
    assert_eq!(o.token, again.token, "same seed reproduces sequence");
}

#[test]
fn zero_workers_rejected() {
    let bundle = Arc::new(Bundle::open(polyfill_path()).expect("polyfill"));
    let (tx, _rx) = crossbeam_channel::bounded(8);
    assert!(WorkerPool::spawn(0, bundle, tx, 16).is_err());
}

#[tokio::test]
async fn touch_instrumentation_reports_api_coverage() {
    let pool = pool();
    let o = pool
        .exec(req(
            "var u = navigator.userAgent; var w = screen.width; u.length + w;",
            1000,
        ))
        .await;
    assert!(o.token.is_some());
    assert!(
        o.touches >= 2,
        "navigator+screen reads must be instrumented, got {}",
        o.touches
    );
}

#[tokio::test]
async fn no_silo_names_leak_into_window() {
    let pool = pool();
    let probe = "var leak = 0;\
        var names = Object.getOwnPropertyNames(globalThis);\
        for (var i = 0; i < names.length; i++) { if (names[i].indexOf(\"__silo\") === 0) { leak++; } }\
        var probe2 = (\"__silo_sha256\" in globalThis) || (\"__silo_profile\" in globalThis) || (\"__silo_cache\" in globalThis);\
        leak * 2 + (probe2 ? 1 : 0);";
    let o = pool.exec(req(probe, 1000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("0"),
        "window must not expose a single __silo_* name, got {:?}",
        o.token
    );
}

#[tokio::test]
async fn polyfill_profile_reaches_canvas_stub() {
    let pool = pool();
    let probe = "var c = document.createElement(\"canvas\");\
        var gl = c.getContext(\"webgl\");\
        var v = gl.getParameter(gl.getExtension(\"WEBGL_debug_renderer_info\").UNMASKED_VENDOR_WEBGL);\
        v.indexOf(\"Intel\") >= 0;";
    let o = pool.exec(req(probe, 1000)).await;
    assert_eq!(o.token.as_deref(), Some("true"), "webgl vendor из профиля");
}

#[tokio::test]
async fn input_events_reach_js_listeners_as_trusted_dom_events() {
    let pool = pool();
    let script = "var moves = 0, clicks = 0, downs = 0, ups = 0, keys = 0, trusted = 0, lastX = -1;\
        document.addEventListener(\"mousemove\", function (e) { moves++; if (e.isTrusted) { trusted++; } lastX = e.clientX; });\
        document.addEventListener(\"click\", function (e) { if (e.isTrusted) { clicks++; } });\
        window.addEventListener(\"mousedown\", function (e) { downs++; });\
        window.addEventListener(\"mouseup\", function (e) { ups++; });\
        window.addEventListener(\"keydown\", function (e) { keys++; });\
        (moves > 10 && clicks >= 1 && downs === ups && keys === 1 && trusted === moves && lastX >= 0) ? \"ok\" : \"bad:\" + moves + \",\" + clicks + \",\" + downs + \",\" + ups + \",\" + keys + \",\" + trusted;";
    let mut events =
        payload_gen::input::interaction_events_for(&session_state::Profile::shell(), "https://x.example/", 0);
    events.push(payload_gen::input::RawEvent::new(b'q' as u16, 0, 40, 4, 0));
    events.push(payload_gen::input::RawEvent::new(b'q' as u16, 0, 60, 5, 0));
    let input: Arc<[payload_gen::input::RawEvent]> = Arc::from(events.into_vec());
    let mut r = req(script, 2000);
    r.input = Some(input);
    let o = pool.exec(r).await;
    assert_eq!(
        o.token.as_deref(),
        Some("ok"),
        "trusted input pipeline: {:?}",
        o.token
    );
}

#[tokio::test]
async fn visibility_events_flip_document_state() {
    let pool = pool();
    let script = "var seen = 0, hidden = null;\
        document.addEventListener(\"visibilitychange\", function (e) { seen++; hidden = document.hidden; });\
        seen + \"|\" + (hidden === true) + \"|\" + document.visibilityState;";
    let input: Arc<[payload_gen::input::RawEvent]> =
        Arc::from(vec![payload_gen::input::RawEvent::new(0, 0, 100, 8, 1)]);
    let mut r = req(script, 1000);
    r.input = Some(input);
    let o = pool.exec(r).await;
    assert_eq!(
        o.token.as_deref(),
        Some("1|true|hidden"),
        "visibility: {:?}",
        o.token
    );
}

#[tokio::test]
async fn polyfill_storage_context_canvas_and_clock_are_real() {
    let pool = pool();
    let script = "var a = typeof localStorage === \"object\" && typeof sessionStorage === \"object\";\
        localStorage.setItem(\"k\", \"v\");\
        var b = localStorage.getItem(\"k\") === \"v\" && localStorage.length === 1 && localStorage.key(0) === \"k\";\
        var c = crossOriginIsolated === false && isSecureContext === true;\
        var fresh = document.createElement(\"canvas\").toDataURL();\
        var drawn = document.createElement(\"canvas\");\
        var ctx = drawn.getContext(\"2d\");\
        ctx.fillStyle = \"#f60\"; ctx.fillRect(125, 1, 62, 20); ctx.fillText(\"probe\", 2, 15);\
        var d = drawn.toDataURL() !== fresh && drawn.toDataURL().indexOf(\"image/png\") > 0;\
        var t0 = performance.now(); var t1 = performance.now(); var t2 = performance.now();\
        var e = t1 >= t0 && t2 >= t1;\
        (a ? 1 : 0) + \"|\" + (b ? 1 : 0) + \"|\" + (c ? 1 : 0) + \"|\" + (d ? 1 : 0) + \"|\" + (e ? 1 : 0);";
    let o = pool.exec(req(script, 1000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("1|1|1|1|1"),
        "polyfill: {:?}",
        o.token
    );
}

#[tokio::test]
async fn timers_run_after_main_script_with_browser_order() {
    let pool = pool();
    let script =
        "globalThis.a = false; globalThis.b = 0; globalThis.c = false; globalThis.mic = false;
        queueMicrotask(function () { globalThis.mic = true; });
        setTimeout(function () { globalThis.a = true; }, 30);
        setTimeout(function (x) { globalThis.b = x; }, 0, 7);
        setTimeout(\"globalThis.c = true;\", 5);
        globalThis.c === false && globalThis.b === 0 ? \"deferred\" : \"sync\";
        ";
    let o = pool.exec(req(script, 3000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("deferred"),
        "main frame: {:?}",
        o.err
    );
    let after = pool
        .exec(req(
            "[globalThis.a, globalThis.b, globalThis.c, globalThis.mic].join(\"|\");",
            3000,
        ))
        .await;
    assert_eq!(
        after.token.as_deref(),
        Some("true|7|true|true"),
        "timers: {:?}",
        after.token
    );
}

#[tokio::test]
async fn interval_reschedules_and_clears() {
    let pool = pool();
    let script = "globalThis.n = 0;
        var id = setInterval(function () { globalThis.n++; if (globalThis.n >= 3) { clearInterval(id); } }, 10);
        \"started\";";
    let o = pool.exec(req(script, 3000)).await;
    assert_eq!(o.token.as_deref(), Some("started"));
    let check = pool
        .exec(req(
            "globalThis.n >= 3 ? \"cycled\" : \"n:\" + globalThis.n;",
            3000,
        ))
        .await;
    assert_eq!(
        check.token.as_deref(),
        Some("cycled"),
        "interval: {:?}",
        check.err
    );
}

#[tokio::test]
async fn raf_batches_with_single_timestamp_per_frame() {
    let pool = pool();
    let script = "globalThis.t1 = -1; globalThis.t2 = -2; globalThis.called = 0;
        requestAnimationFrame(function (t) { globalThis.t1 = t; globalThis.called++; });
        requestAnimationFrame(function (t) { globalThis.t2 = t; globalThis.called++; });
        \"ok\";";
    let o = pool.exec(req(script, 3000)).await;
    assert_eq!(o.token.as_deref(), Some("ok"));
    let check = pool.exec(req("globalThis.called === 2 && globalThis.t1 === globalThis.t2 && globalThis.t1 >= 0 ? \"batch\" : \"bad\";", 3000)).await;
    assert_eq!(
        check.token.as_deref(),
        Some("batch"),
        "raf: {:?}",
        check.err
    );
}

#[tokio::test]
async fn error_stack_is_v8_shaped_with_limit_and_capture() {
    let pool = pool();
    let script = "var out = [];
        out.push(Error.stackTraceLimit);
        var arrow = function () { return new Error(\"boom\").stack; };
        var st = arrow();
        out.push(st.indexOf(\"at anonymous\") === -1);
        out.push(st.split(\"\\n\").length <= 12);
        var target = {};
        Error.captureStackTrace(target);
        out.push(typeof target.stack === \"string\" && target.stack.indexOf(\"boom\") === -1);
        out.join(\"|\");";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("10|true|true|true"),
        "stack: {:?}",
        o.err
    );
}

#[tokio::test]
async fn native_tostring_is_single_line_v8() {
    let pool = pool();
    let script = "var a = navigator.javaEnabled.toString();
        var b = Math.random.toString();
        var c = performance.now.toString();
        var own = (function (x) { return x + 1; }).toString();
        (a === \"function javaEnabled() { [native code] }\") + \"|\" +
        (b === \"function random() { [native code] }\") + \"|\" +
        (c === \"function now() { [native code] }\") + \"|\" +
        (own.indexOf(\"[native code]\") === -1 && own.indexOf(\"function\") === 0);";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true|true|true"),
        "toString: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn canvas_png_is_valid_draw_sensitive_and_stable() {
    let pool = pool();
    let script = "var c = document.createElement(\"canvas\");
        var blank = c.toDataURL();
        var magic = blank.indexOf(\"iVBOR\") === 22;
        var ctx = c.getContext(\"2d\");
        ctx.fillStyle = \"#f60\";
        ctx.fillRect(125, 1, 62, 20);
        ctx.fillText(\"probe\", 2, 15);
        var drawn = c.toDataURL();
        var repeat = c.toDataURL();
        var c2 = document.createElement(\"canvas\");
        var ctx2 = c2.getContext(\"2d\");
        ctx2.fillRect(1, 2, 3, 4);
        var other = c2.toDataURL();
        c.width = 200;
        var reset = c.toDataURL();
        [magic, drawn !== blank, repeat === drawn, other !== drawn, reset !== drawn, c.getContext(\"2d\") === ctx].join(\"|\");";
    let o = pool.exec(req(script, 3000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true|true|true|true|true"),
        "canvas: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn canvas_imagedata_matches_and_is_deterministic() {
    let pool = pool();
    let script = "var c = document.createElement(\"canvas\");
        var ctx = c.getContext(\"2d\");
        ctx.fillRect(0, 0, 40, 30);
        var d1 = ctx.getImageData(0, 0, 4, 4);
        var d2 = ctx.getImageData(0, 0, 4, 4);
        var same = true;
        for (var i = 0; i < 16; i++) { if (d1.data[i * 4 + 3] !== d2.data[i * 4 + 3] || d1.data[i * 4] !== d2.data[i * 4]) { same = false; } }
        (d1.width === 4) + \"|\" + (d1.height === 4) + \"|\" + same + \"|\" + (d1.data[3] === 255);";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true|true|true"),
        "imageData: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn webgl_parameter_table_is_chrome_shaped() {
    let pool = pool();
    let script = "var gl = document.createElement(\"canvas\").getContext(\"webgl\");
        var ext = gl.getSupportedExtensions();
        var a = gl.getParameter(3379);
        var b = gl.getParameter(36349);
        var e = gl.getError();
        var vv = gl.getParameter(7936);
        ((a === 16384) || (a === 8192)) + \"|\" + (b >= 4087 && b <= 4095) + \"|\" + (e === 0) + \"|\" + (vv === \"WebKit\") + \"|\" + (ext.length > 10);";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true|true|true|true"),
        "webgl: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn dom_query_selector_walks_the_tree() {
    let pool = pool();
    let script = "var gate = document.querySelector(\"#gate\");
        var missing = document.querySelector(\"#nope\");
        var all = document.querySelectorAll(\"input\");
        var form = document.querySelector(\"form\");
        var body = document.body;
        var head = document.head;
        [gate !== null, missing === null, all.length >= 1, form !== null, body !== null, head !== null, gate.tagName === \"DIV\"].join(\"|\");";
    let mut r = req(script, 2000);
    r.doc = Some(page_doc());
    let o = pool.exec(r).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true|true|true|true|true|true"),
        "qsa: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn cookie_setter_surfaces_in_outcome() {
    let pool = pool();
    let o = pool
        .exec(req("document.cookie = \"sig=42; Path=/\"; \"set\";", 1000))
        .await;
    assert_eq!(o.token.as_deref(), Some("set"));
    assert_eq!(
        o.cookie_out.as_deref().map(|c| c.contains("sig=42")),
        Some(true)
    );
}

#[tokio::test]
async fn crypto_subtle_digest_sha256_known_vector() {
    let pool = pool();
    let script =
        "crypto.subtle.digest(\"SHA-256\", new TextEncoder().encode(\"abc\")).then(function (buf) {
        var b = new Uint8Array(buf);
        var h = \"\"; var hex = \"0123456789abcdef\";
        for (var i = 0; i < 32; i++) { h += hex[b[i] >> 4] + hex[b[i] & 15]; }
        globalThis.__hash = h;
    });
    \"pending\";";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(o.token.as_deref(), Some("pending"));
    let check = pool.exec(req("globalThis.__hash === \"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\" ? \"ok\" : \"bad:\" + globalThis.__hash;", 2000)).await;
    assert_eq!(
        check.token.as_deref(),
        Some("ok"),
        "subtle: {:?} => {:?}",
        check.err,
        check.token
    );
}

#[tokio::test]
async fn url_textencoder_stub_basics() {
    let pool = pool();
    let script = "var u = new URL(\"https://host.local/pa?x=1&y=2#f\");
        var q = new URLSearchParams(\"a=b&c=d\");
        var enc = new TextEncoder().encode(\"\\u00e9\");
        var dec = new TextDecoder().decode(new Uint8Array([104, 105]));
        [u.hostname === \"host.local\", u.pathname === \"/pa\", u.searchParams.get(\"y\") === \"2\", q.get(\"c\") === \"d\", q.has(\"a\"), enc.length === 2, dec === \"hi\"].join(\"|\");";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true|true|true|true|true|true"),
        "url: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn navigator_properties_live_on_prototype() {
    let pool = pool();
    let script = "var own = Object.getOwnPropertyDescriptor(navigator, \"userAgent\");
        var proto = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(navigator), \"userAgent\");
        var names = Object.getOwnPropertyNames(navigator);
        var inst = navigator instanceof Navigator;
        var screenInst = screen instanceof Screen;
        [own === undefined, proto !== undefined && typeof proto.get === \"function\", names.length === 0, inst, screenInst].join(\"|\");";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true|true|true|true"),
        "proto: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn stealth_surface_stubs_are_present() {
    let pool = pool();
    let script = "var fp = 0;
        var r1 = typeof navigator.plugins === \"object\" && navigator.plugins.length === 5;
        var r2 = typeof navigator.mimeTypes === \"object\" && navigator.mimeTypes.length === 2;
        var r3 = typeof window.chrome === \"object\" && typeof window.chrome.runtime.getManifest === \"function\";
        var r4 = Notification.permission === \"default\" && typeof Notification.requestPermission === \"function\";
        var r5 = screen.orientation.type === \"landscape-primary\" && screen.orientation.angle === 0;
        var r6 = typeof navigator.permissions.query === \"function\";
        var r7 = typeof navigator.mediaDevices.enumerateDevices === \"function\";
        var r8 = typeof AudioContext === \"function\" && typeof OfflineAudioContext === \"function\";
        var r9 = typeof Worker === \"function\" && typeof RTCPeerConnection === \"function\";
        var r10 = document.fonts.status === \"loaded\" && typeof document.featurePolicy.allowsFeature === \"function\";
        var r11 = typeof navigator.userAgentData === \"object\" && navigator.userAgentData.platform === \"Windows\";
        [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10, r11].join(\"|\");";
    let o = pool.exec(req(script, 2500)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true|true|true|true|true|true|true|true|true|true"),
        "stubs: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn audio_fingerprint_is_per_profile_deterministic() {
    let pool = pool();
    let script = "var ctx = new OfflineAudioContext(1, 100, 44100);\
        var comp = ctx.createDynamicsCompressor();\
        var inBand = typeof comp.reduction === \"number\" && comp.reduction >= 124.0 && comp.reduction < 124.1;\
        var a = ctx.createBuffer(1, 100, 44100).getChannelData(0);\
        var b = ctx.createBuffer(1, 100, 44100).getChannelData(0);\
        var stable = true;\
        for (var i = 0; i < 100; i++) { if (a[i] !== b[i]) { stable = false; } }\
        var ctx2 = new AudioContext();\
        var c = ctx2.createBuffer(1, 100, 44100).getChannelData(0);\
        var online = true;\
        for (var j = 0; j < 100; j++) { if (c[j] !== a[j]) { online = false; } }\
        [inBand, stable, online].join(\"|\");";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true|true"),
        "audio: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn performance_clock_is_quantized_and_monotonic() {
    let pool = pool();
    let script = "var originNear = Math.abs(performance.timeOrigin - Date.now()) < 60000;
        var a = performance.now();
        var b = performance.now();
        var quantized = (a * 10) === Math.round(a * 10);
        [originNear, b >= a, quantized].join(\"|\");";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true|true"),
        "clock: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn domcontentloaded_fires_after_timers() {
    let pool = pool();
    let script = "globalThis.seen = \"none\";
        document.addEventListener(\"DOMContentLoaded\", function () { globalThis.seen = \"dom\"; });
        \"reg\";";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(o.token.as_deref(), Some("reg"));
    let check = pool
        .exec(req(
            "globalThis.seen === \"dom\" ? \"fired\" : globalThis.seen;",
            2000,
        ))
        .await;
    assert_eq!(
        check.token.as_deref(),
        Some("fired"),
        "domready: {:?}",
        check.err
    );
}

#[tokio::test]
async fn crossorigin_isolated_is_frozen() {
    let pool = pool();
    let script = "var out = \"initial\";
        try { crossOriginIsolated = true; out = \"mutated\"; } catch (e) { out = \"frozen\"; }
        out + \"|\" + crossOriginIsolated + \"|\" + isSecureContext;";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("frozen|false|true"),
        "coop: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn location_reports_real_path_and_navigation_is_captured() {
    let pool = pool();
    let script = "location.assign(\"https://next.local/x\");
        [location.pathname, location.search].join(\"|\");";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("/page|"),
        "loc: {:?} => {:?}",
        o.err,
        o.token
    );
    assert_eq!(o.nav.as_deref(), Some("https://next.local/x"));
}

#[tokio::test]
async fn firefox_only_fields_are_absent_for_chrome_profiles() {
    let pool = pool();
    let script = "[typeof navigator.oscpu, typeof navigator.buildID].join(\"|\");";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("undefined|undefined"),
        "ff: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn layout_boxes_are_real_and_element_from_point_hits() {
    let pool = pool();
    let script = "var gate = document.getElementById(\"gate\");
        var r = gate.getBoundingClientRect();
        var de = document.documentElement;
        var w = gate.offsetWidth;
        var hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
        var cs = window.getComputedStyle ? window.getComputedStyle(gate) : document.getComputedStyle(gate);
        [r.width > 0, r.height > 0, w === r.width, hit !== null && hit === gate, de !== null, cs.display === \"block\", document.documentElement.clientWidth === screen.width].join(\"|\");";
    let mut r = req(script, 2000);
    r.doc = Some(page_doc());
    let o = pool.exec(r).await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true|true|true|true|true|true"),
        "layout: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn intl_observers_event_and_battery_surface() {
    let pool = pool();
    let script = "var tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
        var mo = typeof MutationObserver === \"function\" && typeof ResizeObserver === \"function\" && typeof IntersectionObserver === \"function\";
        var seen = 0;
        var el = document.createElement(\"div\");
        el.addEventListener(\"ping\", function () { seen++; });
        el.dispatchEvent(new Event(\"ping\"));
        var beacon = typeof navigator.sendBeacon === \"function\";
        var battery = typeof navigator.getBattery === \"function\";
        [tz, mo, seen === 1, typeof Event === \"function\", beacon, battery].join(\"|\");";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("America/New_York|true|true|true|true|true"),
        "surface: {:?} => {:?}",
        o.err,
        o.token
    );
}

#[tokio::test]
async fn fetch_without_bridge_falls_back_to_stub() {
    let pool = pool();
    let script = "var out = \"none\";
        fetch(\"https://stub.local/x\").then(function (r) { out = r.status + \"|\" + r.ok; });
        setTimeout(function () { globalThis.__r = out; }, 5);
        \"kicked\";";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("kicked"),
        "fetch-stub: {:?}",
        o.err
    );
    let check = pool
        .exec(req(
            "globalThis.__r === \"204|true\" ? \"stub\" : globalThis.__r;",
            2000,
        ))
        .await;
    assert_eq!(
        check.token.as_deref(),
        Some("stub"),
        "stub fetch: {:?}",
        check.token
    );
}
