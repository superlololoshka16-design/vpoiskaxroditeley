use common::req;

mod common;

fn script(body: &str) -> String {
    format!("JSON.stringify((function(){{ {body} }})())")
}

fn raw(body: &str) -> String {
    format!("(function(){{ {body} }})()")
}

#[tokio::test]
async fn global_surface_count_probe() {
    let pool = common::pool();
    let probe = script(
        "var names = Object.getOwnPropertyNames(globalThis);\
         var fns = 0, objs = 0, strs = 0;\
         for (var i = 0; i < names.length; i++) {\
             try {\
                 var v = globalThis[names[i]];\
                 var t = typeof v;\
                 if (t === 'function') { fns++; } else if (t === 'object' && v !== null) { objs++; } else { strs++; }\
             } catch (e) { }\
         }\
         return { total: names.length, fns: fns, objs: objs, other: strs };",
    );
    let o = pool.exec(req(&probe, 2000)).await;
    let tok = o.token.as_deref().unwrap_or_default();
    let total: u64 = tok.split_once("\"total\":").and_then(|(_, r)| r.split(",").next()).and_then(|r| r.trim().parse().ok()).unwrap_or(0);
    println!("globalThis own props: {tok}");
    assert!(
        total >= 170,
        "chrome-mimicking surface too small: {total} own props (chrome stable ~180)"
    );
}

#[tokio::test]
async fn webgl_family_reflective() {
    let pool = common::pool();
    let probe = raw(
        "var ok = typeof WebGLRenderingContext === 'function'\
             && typeof WebGLShader === 'function'\
             && typeof WebGLBuffer === 'function'\
             && typeof WebGLUniformLocation === 'function';\
         try { new WebGLRenderingContext(); return 'constructible-bad'; } catch (e) { }\
         return ok ? 'true' : 'false';",
    );
    let o = pool.exec(req(&probe, 2000)).await;
    assert_eq!(o.token.as_deref(), Some("true"));
}

#[tokio::test]
async fn dom_ctor_family_reflective() {
    let pool = common::pool();
    let probe = raw(
        "var ok = typeof Range === 'function'\
             && typeof NodeList === 'function'\
             && typeof Storage === 'function'\
             && typeof History === 'function'\
             && typeof DOMParser === 'function'\
             && typeof XMLSerializer === 'function';\
         try { new Range(); return 'constructible-bad'; } catch (e) { }\
         return ok ? 'true' : 'false';",
    );
    let o = pool.exec(req(&probe, 2000)).await;
    assert_eq!(o.token.as_deref(), Some("true"));
}

#[tokio::test]
async fn event_ctor_family_functional() {
    let pool = common::pool();
    let probe = raw(
        "var e = new MouseEvent('click', { clientX: 11, clientY: 22 });\
         var c = new CustomEvent('boom', { detail: 7 });\
         var ev = new Event('plain');\
         var ok = e instanceof MouseEvent && e instanceof UIEvent && e instanceof Event\
             && e.clientX === 11 && e.clientY === 22\
             && c instanceof CustomEvent && c.detail === 7\
             && ev instanceof Event && ev.type === 'plain';\
         return ok ? 'true' : 'false';",
    );
    let o = pool.exec(req(&probe, 2000)).await;
    assert_eq!(o.token.as_deref(), Some("true"), "event ctors must be functional");
}

#[tokio::test]
async fn abort_controller_functional() {
    let pool = common::pool();
    let probe = raw(
        "var ac = new AbortController();\
         var s = ac.signal;\
         var fired = false;\
         s.addEventListener('abort', function () { fired = true; });\
         var ok = s instanceof AbortSignal && s.aborted === false;\
         ac.abort();\
         return (ok && fired && s.aborted === true) ? 'true' : 'false';",
    );
    let o = pool.exec(req(&probe, 2000)).await;
    assert_eq!(o.token.as_deref(), Some("true"));
}

#[tokio::test]
async fn shared_array_buffer_absent_like_non_isolated() {
    let pool = common::pool();
    let probe = raw("return typeof SharedArrayBuffer;");
    let o = pool.exec(req(&probe, 2000)).await;
    assert_eq!(o.token.as_deref(), Some("undefined"));
}

#[tokio::test]
async fn enumerable_surface_probe() {
    let pool = common::pool();
    let probe = script(
        "var keys = Object.keys(globalThis).length;         var forin = 0; for (var k in globalThis) { forin++; }         return { keys: keys, forin: forin };",
    );
    let o = pool.exec(req(&probe, 2000)).await;
    println!("enumerable surface: {}", o.token.as_deref().unwrap_or_default());
    assert!(
        o.err.is_none(),
        "probe failed: {:?}",
        o.err.map(|e| e.to_string())
    );
}

#[tokio::test]
async fn list_all_globals_probe() {
    let pool = common::pool();
    let probe = script(
        "return Object.getOwnPropertyNames(globalThis).sort().join(',');",
    );
    let o = pool.exec(req(&probe, 2000)).await;
    println!("ALLGLOBALS:{}", o.token.as_deref().unwrap_or_default());
}
