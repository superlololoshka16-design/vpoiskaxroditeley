use runtime_exec::NAV_RELOAD;
mod common;
use common::{pool, req};





#[tokio::test]
async fn ready_state_transitions_to_complete() {
    let pool = pool();
    let probe = r#"
if (document.readyState !== "loading") { throw new Error("must start loading"); }
(function () { return "ok"; })();
"#;
    let outcome = pool.exec(req(probe, 4000)).await;
    assert!(outcome.err.is_none(), "err: {:?}", outcome.err);
}

#[tokio::test]
async fn domcontentloaded_and_load_fire_in_order() {
    let pool = pool();
    let probe = r#"
var order = [];
document.addEventListener("DOMContentLoaded", function () { order.push("dcl"); });
window.addEventListener("load", function () { order.push("load"); });
setTimeout(function () {
    if (order.join(",") !== "dcl,load") { throw new Error("order was: " + order.join(",")); }
    if (document.readyState !== "complete") { throw new Error("readyState not complete: " + document.readyState); }
    "done";
}, 30);
(function () { return "ok"; })();
"#;
    let outcome = pool.exec(req(probe, 4000)).await;
    assert!(outcome.err.is_none(), "err: {:?}", outcome.err);
}

#[tokio::test]
async fn location_reload_sets_pending_navigation() {
    let pool = pool();
    let script = r#"
setTimeout(function () { location.reload(); }, 5);
(function () { return "reload-scheduled"; })();
"#;
    let outcome = pool.exec(req(script, 4000)).await;
    assert!(outcome.err.is_none(), "err: {:?}", outcome.err);
    assert_eq!(outcome.nav.as_deref(), Some(NAV_RELOAD));
}

#[tokio::test]
async fn location_assign_sets_pending_navigation() {
    let pool = pool();
    let script = r#"
setTimeout(function () { location.assign("https://mock.local/next"); }, 5);
(function () { return "assign-scheduled"; })();
"#;
    let outcome = pool.exec(req(script, 4000)).await;
    assert!(outcome.err.is_none(), "err: {:?}", outcome.err);
    assert_eq!(outcome.nav.as_deref(), Some("https://mock.local/next"));
}

#[tokio::test]
async fn string_callback_timer_runs_via_eval() {
    let pool = pool();
    let script = r#"
window.__cb_hit = 0;
setTimeout("__cb_hit = 42;", 5);
setTimeout(function () {
    if (window.__cb_hit !== 42) { throw new Error("string callback did not run"); }
    "done";
}, 40);
(function () { return "ok"; })();
"#;
    let outcome = pool.exec(req(script, 4000)).await;
    assert!(outcome.err.is_none(), "err: {:?}", outcome.err);
}

#[tokio::test]
async fn canvas_cost_jitter_is_deterministic_per_profile() {
    let pool = pool();
    let script = r#"
var c = document.createElement("canvas");
c.width = 200; c.height = 50;
var ctx = c.getContext("2d");
var a = c.toDataURL();
var b = c.toDataURL();
if (a !== b) { throw new Error("toDataURL not stable"); }
a.length;
"#;
    let o1 = pool.exec(req(script, 4000)).await;
    let o2 = pool.exec(req(script, 4000)).await;
    assert_eq!(
        o1.token, o2.token,
        "same profile + same draw must give same png"
    );
}
