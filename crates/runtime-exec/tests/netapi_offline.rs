mod common;
use common::{pool, req};

#[tokio::test]
async fn xhr_offline_falls_back_to_204_without_bridge() {
    let pool = pool();
    let script = "var x = new XMLHttpRequest();\
        var out = \"pre:\" + x.readyState;\
        x.open(\"GET\", \"https://mock.local/off\");\
        out += \":\" + x.readyState;\
        x.onload = function () { globalThis.__off = \"off:\" + x.status + \":\" + x.readyState + \":\" + (typeof x.responseText); };\
        x.onerror = function () { globalThis.__off = \"err\"; };\
        x.send();\
        out;";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("pre:0:1"),
        "offline sync phase: {:?}",
        o.err
    );
    let read = "globalThis.__off || \"pending\";";
    let o = pool.exec(req(read, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("off:204:4:string"),
        "offline fallback: {:?}",
        o.err
    );
}

#[tokio::test]
async fn xhr_send_before_open_throws_chrome_invalid_state() {
    let pool = pool();
    let script = "var x = new XMLHttpRequest(); var out;\
        try { x.send(); out = \"no\"; }\
        catch (e) { out = e.message.indexOf(\"InvalidStateError\") === 0 ? \"state\" : \"other:\" + e.message; }\
        out;";
    let o = pool.exec(req(script, 2000)).await;
    assert_eq!(
        o.token.as_deref(),
        Some("state"),
        "invalid state error: {:?}",
        o.err
    );
}
