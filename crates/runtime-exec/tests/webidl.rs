mod common;
use common::{page_doc};
use runtime_exec::{ExecKind, ExecReq, ProfileSnap, WorkerPool};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn test_profile() -> session_state::Profile {
    use session_state::{Family, NetKind, Platform};
    session_state::Profile {
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
        canvas_seed: 0x5EED_0001,
        asn: 7922,
        net: NetKind::Residential,
        family: Family::Chrome { major: 149 },
        display_hz: 60,
        ..session_state::Profile::shell()
    }
}

fn snap() -> ProfileSnap {
    ProfileSnap::from_parts(
        &std::sync::Arc::new(test_profile()),
        "https://unit.local/page?q=1",
        "",
    )
}

fn polyfill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../supervisor/assets/polyfill.js")
}

fn pool() -> WorkerPool {
    let bundle = Arc::new(runtime_exec::Bundle::open(polyfill_path()).expect("polyfill"));
    let (tx, _rx) = crossbeam_channel::bounded(64);
    WorkerPool::spawn(1, bundle, tx, 16).expect("pool")
}

fn req(script: &str, timeout_ms: u64) -> ExecReq {
    ExecReq {
        domain: 1,
        script: bytes::Bytes::from(script.as_bytes().to_vec()),
        snap: snap(),
        timeout: Duration::from_millis(timeout_ms),
        doc: None,
        input: None,
        net_slot: 0,
        script_node: None,

        kind: ExecKind::Js,
    }
}



async fn eval(pool: &WorkerPool, script: &str) -> String {
    let o = pool.exec(req(script, 3000)).await;
    o.token
        .unwrap_or_else(|| panic!("js error: {:?}", o.err))
        .as_str()
        .to_string()
}

async fn eval_doc(pool: &WorkerPool, script: &str) -> String {
    let mut r = req(script, 3000);
    r.doc = Some(page_doc());
    let o = pool.exec(r).await;
    o.token
        .unwrap_or_else(|| panic!("js error: {:?}", o.err))
        .as_str()
        .to_string()
}

#[tokio::test]
async fn prototype_chain_instanceof_is_exact() {
    let pool = pool();
    let out = eval(
        &pool,
        "var d = document.createElement('div');\
         [d instanceof HTMLDivElement, d instanceof HTMLElement, d instanceof Element,\
          d instanceof Node, d instanceof EventTarget, d instanceof Object,\
          document instanceof HTMLDocument, document instanceof Node,\
          document instanceof EventTarget, window instanceof Object].join('|');",
    )
    .await;
    assert_eq!(
        out, "true|true|true|true|true|true|true|true|true|true",
        "chain: {out}"
    );
}

#[tokio::test]
async fn descriptors_match_chrome_flags() {
    let pool = pool();
    let out = eval(
        &pool,
        "var d = document.createElement('div');\
         var m = Object.getOwnPropertyDescriptor(Navigator.prototype, 'userAgent');\
         var ap = Object.getOwnPropertyDescriptor(HTMLDivElement.prototype, 'constructor');\
         var p = Object.getOwnPropertyDescriptor(Node.prototype, 'appendChild');\
         var ta = Object.getOwnPropertyDescriptor(HTMLDivElement.prototype, Symbol.toStringTag);\
         [d.__proto__ === HTMLDivElement.prototype,\
          d.__proto__.__proto__ === HTMLElement.prototype,\
          HTMLDivElement.prototype.__proto__ === HTMLElement.prototype,\
          HTMLElement.prototype.__proto__ === Element.prototype,\
          Element.prototype.__proto__ === Node.prototype,\
          Node.prototype.__proto__ === EventTarget.prototype,\
          m.get !== undefined && m.enumerable === true && m.configurable === true,\
          m.get.name, m.set,\
          ap.writable === true && ap.enumerable === false && ap.configurable === true,\
          p.writable === true && p.enumerable === true && p.configurable === true,\
          ta.value, ta.writable === false && ta.enumerable === false && ta.configurable === true].join('§');",
    )
    .await;
    let parts: Vec<&str> = out.split('§').collect();
    assert_eq!(parts.len(), 13, "segments: {out}");
    for (i, part) in parts.iter().take(7).enumerate() {
        assert_eq!(*part, "true", "segment {i}: {out}");
    }
    assert_eq!(parts[7], "get userAgent", "getter name: {out}");
    assert_eq!(parts[8], "", "no setter: {out}");
    assert_eq!(parts[9], "true", "constructor flags: {out}");
    assert_eq!(parts[10], "true", "method flags: {out}");
    assert_eq!(parts[11], "HTMLDivElement", "toStringTag value: {out}");
    assert_eq!(parts[12], "true", "toStringTag flags: {out}");
}

#[tokio::test]
async fn native_getter_tostring_is_v8_single_line() {
    let pool = pool();
    let out = eval(
        &pool,
        "var d = Object.getOwnPropertyDescriptor(Navigator.prototype, 'userAgent').get;\
         var ds = Object.getOwnPropertyDescriptor(Node.prototype, 'textContent');\
         [d.toString(), ds.get.toString(), ds.set !== undefined ? ds.set.name : 'none'].join('§');",
    )
    .await;
    let parts: Vec<&str> = out.split('§').collect();
    assert_eq!(
        parts[0], "function get userAgent() { [native code] }",
        "getter toString: {}",
        parts[0]
    );
    assert_eq!(
        parts[1], "function get textContent() { [native code] }",
        "node getter: {}",
        parts[1]
    );
    assert_eq!(parts[2], "set textContent", "setter name: {}", parts[2]);
}

#[tokio::test]
async fn illegal_constructors_and_type_errors_are_native() {
    let pool = pool();
    let out = eval(
        &pool,
        "var a, b, c, d, e;\
         try { new HTMLDivElement(); a = 'no'; } catch (err) { a = err.message; }\
         try { new Node(); b = 'no'; } catch (err) { b = err.message; }\
         try { document.createElement('div').appendChild({}); c = 'no'; } catch (err) { c = err.message; }\
         try { Node.prototype.appendChild.call({}, document.createElement('div')); d = 'no'; } catch (err) { d = err.message; }\
         try { new HTMLCanvasElement(); e = 'no'; } catch (err) { e = err.message; }\
         [a, b, c, d, e].join('§');",
    )
    .await;
    let parts: Vec<&str> = out.split('§').collect();
    for p in &parts {
        assert!(
            p.starts_with("TypeError") || p.starts_with("Illegal"),
            "native error: {p}"
        );
    }
    assert_eq!(
        parts[0], "TypeError: Illegal constructor",
        "div ctor: {}",
        parts[0]
    );
    assert_eq!(
        parts[1], "TypeError: Illegal constructor",
        "node ctor: {}",
        parts[1]
    );
    assert_eq!(
        parts[2],
        "TypeError: Failed to execute 'appendChild' on 'Node': parameter 1 is not of type 'Node'.",
        "bad arg: {}",
        parts[2]
    );
    assert_eq!(
        parts[3], "TypeError: Illegal invocation",
        "bad this: {}",
        parts[3]
    );
    assert_eq!(
        parts[4], "TypeError: Illegal constructor",
        "canvas ctor: {}",
        parts[4]
    );
}

#[tokio::test]
async fn dom_mutations_are_real_and_queryable() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "var d = document.createElement('div');\
         d.id = 'zone';\
         var s = document.createElement('span');\
         s.setAttribute('class', 'tag');\
         var t = document.createTextNode('hi');\
         s.appendChild(t);\
         d.appendChild(s);\
         document.body ? 1 : 1;\
         var body = document.body || document.documentElement;\
         body.appendChild(d);\
         var found = document.getElementById('zone');\
         var q = document.querySelector('#zone span.tag');\
         var kids = d.childNodes.length;\
         var txt = s.textContent;\
         var html = d.innerHTML;\
         s.className = 'tag2';\
         var cls = s.getAttribute('class');\
         s.removeAttribute('class');\
         var gone = s.getAttribute('class') === null;\
         d.removeChild(s);\
         var empty = d.childNodes.length === 0;\
         var back = found === d;\
         [found !== null, q === s, kids, txt, html.indexOf('tag') >= 0 && html.indexOf('hi') >= 0,\
          cls, gone, empty, back].join('|');",
    )
    .await;
    assert_eq!(
        out, "true|true|1|hi|true|tag2|true|true|true",
        "mutations: {out}"
    );
}

#[tokio::test]
async fn mutation_observer_receives_child_list_records() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "window.__obs = 0;\
         var d = document.createElement('div');\
         var mo = new MutationObserver(function (recs) { window.__obs += recs.length; });\
         mo.observe(d, { childList: true, subtree: true });\
         var inner = document.createElement('p');\
         d.appendChild(inner);\
         inner.setAttribute('data-x', '1');\
         'wait';",
    )
    .await;
    assert_eq!(out, "wait", "observer exec: {out}");
    let out2 = eval(&pool, "window.__obs;").await;
    assert_eq!(out2, "1", "observer records: {out2}");
}

#[tokio::test]
async fn node_constants_and_to_string_tag() {
    let pool = pool();
    let out = eval(
        &pool,
        "var d = document.createElement('div');\
         [Node.ELEMENT_NODE, Node.TEXT_NODE, Node.DOCUMENT_NODE, Node.DOCUMENT_FRAGMENT_NODE,\
          Object.prototype.toString.call(d),\
          Object.prototype.toString.call(document),\
          Object.prototype.toString.call(window),\
          Object.prototype.toString.call(document.createElement('canvas'))].join('|');",
    )
    .await;
    assert_eq!(
        out,
        "1|3|9|11|[object HTMLDivElement]|[object HTMLDocument]|[object Window]|[object HTMLCanvasElement]",
        "constants/tags: {out}"
    );
}

#[tokio::test]
async fn canvas_class_identity_and_no_canvasel_leak() {
    let pool = pool();
    let out = eval(
        &pool,
        "var c = document.createElement('canvas');\
         var ctx = c.getContext('2d');\
         [c instanceof HTMLCanvasElement, c instanceof HTMLElement, c instanceof Node,\
          c.constructor.name, ctx.constructor.name,\
          Object.getPrototypeOf(c).constructor.name].join('|');",
    )
    .await;
    assert_eq!(
        out, "true|true|true|HTMLCanvasElement|CanvasRenderingContext2D|HTMLCanvasElement",
        "canvas id: {out}"
    );
}

#[tokio::test]
async fn constructor_names_and_globals_present() {
    let pool = pool();
    let out = eval(
        &pool,
        "var names = [typeof EventTarget, typeof Node, typeof Element, typeof HTMLElement,\
          typeof HTMLDivElement, typeof HTMLUnknownElement, typeof MutationObserver,\
          typeof console, typeof WorkerNavigator, typeof DocumentFragment];\
         var w = new EventTarget();\
         names.push(typeof w.addEventListener);\
         var u = document.createElement('custom-tag');\
         names.push(u instanceof HTMLUnknownElement, u instanceof HTMLElement);\
         var f = document.createDocumentFragment();\
         names.push(f instanceof DocumentFragment, f instanceof Node);\
         names.join('|');",
    )
    .await;
    assert_eq!(
        out,
        "function|function|function|function|function|function|function|object|function|function|function|true|true|true|true",
        "globals: {out}"
    );
}

#[tokio::test]
async fn console_is_native_and_hidden_from_silo() {
    let pool = pool();
    let out = eval(
        &pool,
        "var names = [typeof console.log, typeof console.warn, typeof console.error,\
          typeof console.memory, typeof console.debug];\
         var s = console.log.toString();\
         names.push(s === 'function log() { [native code] }');\
         names.push('__silo_profile' in globalThis, 'NodeHandle' in globalThis, 'CanvasEl' in globalThis);\
         names.push(typeof window.Node, window === self, window === globalThis);\
         names.push(typeof document.defaultView, typeof document.activeElement);\
         names.join('|');",
    )
    .await;
    assert_eq!(
        out,
        "function|function|function|undefined|function|true|false|false|false|function|true|true|object|object",
        "console/surface: {out}"
    );
}

#[tokio::test]
async fn inner_text_style_and_eventtarget_roundtrip() {
    let pool = pool();
    let out = eval(
        &pool,
        "var d = document.createElement('div');\
         d.innerText = 'hello';\
         var styleSeen = '';\
         d.style.display = 'none';\
         styleSeen = d.style.display;\
         var fired = 0;\
         d.addEventListener('ping', function () { fired++; });\
         var ev = { type: 'ping' };\
         d.dispatchEvent(ev);\
         d.dispatchEvent(ev);\
         d.removeEventListener('ping', undefined);\
         var txt = d.innerText;\
         [txt, styleSeen, fired, typeof d.style, d.style === d.style].join('|');",
    )
    .await;
    assert_eq!(out, "hello|none|2|object|true", "element surface: {out}");
}

#[tokio::test]
async fn subtle_hmac_and_pbkdf2_with_limits() {
    let pool = pool();
    let out = eval(
        &pool,
        "window.__sub = [0, 0];\
         crypto.subtle.digest('SHA-256', new Uint8Array([1,2,3])).then(function (b) { window.__sub[0] += b.byteLength === 32 ? 1 : 0; });\
         crypto.subtle.importKey('raw', new Uint8Array([9,9,9]), { name: 'HMAC' }, false, ['sign'])\
             .then(function (k) {\
                 return crypto.subtle.sign('HMAC', k, new Uint8Array([7]));\
             })\
             .then(function (mac) { window.__sub[0] += mac.byteLength === 32 ? 1 : 0; })\
             .catch(function () { window.__sub[1]++; });\
         crypto.subtle.deriveBits({ name: 'PBKDF2', salt: new Uint8Array([1,2]), iterations: 100 },\
             { _raw: new Uint8Array([1,2,3]) }, 64)\
             .then(function (bits) { window.__sub[0] += bits.byteLength === 8 ? 1 : 0; })\
             .catch(function () { window.__sub[1]++; });\
         'wait';",
    )
    .await;
    assert_eq!(out, "wait", "subtle exec: {out}");
    let out2 = eval(&pool, "window.__sub[0] + '/' + window.__sub[1];").await;
    assert_eq!(out2, "3/0", "subtle: {out2}");
}

#[tokio::test]
async fn outer_window_matches_chrome_offsets() {
    let pool = pool();
    let out = eval(
        &pool,
        "var iw = window.innerWidth, ih = window.innerHeight;\
         var ow = window.outerWidth, oh = window.outerHeight;\
         var aw = screen.availWidth, ah = screen.availHeight;\
         var chrome = oh - ih;\
         [ow === aw, oh === ah, iw === ow, oh >= ih, chrome >= 87 && chrome <= 111].join('|');",
    )
    .await;
    assert_eq!(out, "true|true|true|true|true", "outer geometry: {out}");
}

#[tokio::test]
async fn raf_cadence_carries_profile_jitter() {
    let pool = pool();
    let out = eval(
        &pool,
        "window.__stamps = [];\
         function frame(t) {\
             window.__stamps.push(t);\
             if (window.__stamps.length < 6) { requestAnimationFrame(frame); }\
         }\
         requestAnimationFrame(frame);\
         \'wait\';",
    )
    .await;
    assert_eq!(out, "wait", "raf exec: {out}");
    let out2 = eval(
        &pool,
        "var s = window.__stamps;\
         var deltas = [];\
         for (var i = 1; i < s.length; i++) { deltas.push(s[i] - s[i-1]); }\
         var uniform = deltas.every(function (d) { return d === 16 || d === 17; });\
         var strict = deltas.every(function (d) { return d === 16.7; });\
         s.length + '|' + uniform + '|' + strict;",
    )
    .await;
    let parts: Vec<&str> = out2.split('|').collect();
    assert_eq!(parts[0], "6", "raf frames: {out2}");
    assert_eq!(parts[1], "false", "cadence not stuck at 16/17: {out2}");
    assert_eq!(parts[2], "false", "not a fixed 16.7 template: {out2}");
}

#[tokio::test]
async fn sibling_navigation_composite_base_and_overlay() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "var body = document.body;\
         var a = document.createElement('div'); a.id = 'a';\
         var b = document.createElement('div'); b.id = 'b';\
         var c = document.createElement('div'); c.id = 'c';\
         body.appendChild(a); body.appendChild(b); body.appendChild(c);\
         [String(a.nextSibling === b), String(b.previousSibling === a),\
          String(b.nextSibling === c), String(c.nextSibling && c.nextSibling.id),\
          String(a.previousSibling === null),\
          String(body.lastChild && body.lastChild.tagName),\
          String(body.firstChild === a)].join('|')",
    )
    .await;
    assert_eq!(out, "true|true|true|gate|true|FORM|true", "sib: {out}");
}

#[tokio::test]
async fn sibling_navigation_moves_relink() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "var body = document.body;\
         var x = document.createElement('p');\
         var y = document.createElement('p');\
         var z = document.createElement('p');\
         body.appendChild(x); body.appendChild(y); body.appendChild(z);\
         body.removeChild(y);\
         [String(x.nextSibling === z), String(z.previousSibling === x),\
          String(y.nextSibling === null), String(y.parentNode === null)].join('|')",
    )
    .await;
    assert_eq!(out, "true|true|true|true", "relink: {out}");
}

#[tokio::test]
async fn get_element_by_id_uses_live_index() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "var el = document.createElement('div');\
         el.setAttribute('id', 'live-id');\
         [String(document.getElementById('live-id') === null),\
          (document.body.appendChild(el), 1),\
          String(document.getElementById('live-id') === el),\
          (el.setAttribute('id', 'renamed'), 1),\
          String(document.getElementById('live-id') === null),\
          String(document.getElementById('renamed') === el),\
          (el.removeAttribute('id'), 1),\
          String(document.getElementById('renamed') === null)].join('|')",
    )
    .await;
    assert_eq!(out, "true|1|true|1|true|true|1|true", "id: {out}");
}

#[tokio::test]
async fn tag_lookup_covers_overlay_and_dynamic() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "var body = document.body;\
         for (var i = 0; i < 5; i++) { body.appendChild(document.createElement('kartoshka-x')); }\
         body.appendChild(document.createElement('DIV'));\
         [String(document.querySelectorAll('kartoshka-x').length),\
          String(document.querySelectorAll('div').length > 0),\
          String(document.querySelectorAll('DIV').length > 0),\
          String(document.querySelector('kartoshka-x') !== null),\
          String(document.getElementsByTagName('div').length)].join('|')",
    )
    .await;
    assert_eq!(out, "10|true|true|true|0", "tag: {out}");
}

#[tokio::test]
async fn contains_is_subtree_wide() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "var body = document.body;\
         var wrap = document.createElement('section');\
         var leaf = document.createElement('span');\
         wrap.appendChild(leaf); body.appendChild(wrap);\
         [String(body.contains(leaf)), String(wrap.contains(leaf)),\
          String(leaf.contains(body)), String(document.documentElement.contains(leaf))].join('|')",
    )
    .await;
    assert_eq!(out, "true|true|false|true", "contains: {out}");
}

#[tokio::test]
async fn text_content_deep_and_inner_html_roundtrip() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "var body = document.body;\
         var w = document.createElement('div');\
         w.innerHTML = '<b>alpha</b>beta<i>gamma</i>';\
         body.appendChild(w);\
         var tc = w.textContent;\
         var ih = w.innerHTML;\
         [tc, ih].join('|')",
    )
    .await;
    assert_eq!(out, "alphabetagamma|<b>alpha</B>beta<i>gamma</I>", "tc: {out}");
}
