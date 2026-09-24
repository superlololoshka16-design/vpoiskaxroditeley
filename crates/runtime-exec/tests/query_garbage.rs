mod common;
use common::{pool, test_profile as profile};
use bytes::Bytes;
use parser_pipeline::StreamPipeline;
use runtime_exec::{ExecKind, ExecReq, ProfileSnap};
use std::sync::Arc;
use std::time::Duration;



fn doc(html: &[u8]) -> Arc<parser_pipeline::PageData> {
    let mut p = StreamPipeline::new(Default::default());
    p.push(html).expect("doc parses");
    Arc::new(p.finish().expect("doc finish"))
}

async fn run(script: &str, page: Arc<parser_pipeline::PageData>) -> Result<Option<String>, String> {
    let pool = pool();
    let req = ExecReq {
        domain: 0xBEEF,
        script: Bytes::copy_from_slice(script.as_bytes()),
        snap: ProfileSnap::from_parts(
            &std::sync::Arc::new(profile()),
            "https://mock.local/page",
            "sid=1",
        ),
        timeout: Duration::from_millis(4000),
        doc: Some(page),
        input: None,
        net_slot: 0,
        script_node: None,
        kind: ExecKind::Js,
    };
    let out = pool.exec(req).await;
    match out.err {
        None | Some(runtime_exec::ExecError::NoResult) => Ok(out.token.map(|t| t.to_string())),
        Some(e) => Err(e.to_string()),
    }
}

#[tokio::test]
async fn garbage_selectors_return_null_not_crash() {
    let pool = pool();
    let page = doc(b"<html><body><div id=\"ok\" class=\"c\" data-x=\"v\">t</div></body></html>");
    let bad = [
        "", "   ", "#", ".", "[", "div[", "[a=", "a::", ",", ">", "a >", "*",
    ];
    for q in bad {
        let esc = q.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "(function () {{ var r = document.querySelector(\"{esc}\"); return r === null ? 'null' : 'hit'; }})()"
        );
        let req = ExecReq {
            domain: 0xBEEF,
            script: Bytes::copy_from_slice(script.as_bytes()),
            snap: ProfileSnap::from_parts(
                &std::sync::Arc::new(profile()),
                "https://mock.local/page",
                "sid=1",
            ),
            timeout: Duration::from_millis(4000),
            doc: Some(Arc::clone(&page)),
            input: None,
            net_slot: 0,
            script_node: None,
            kind: ExecKind::Js,
        };
        let out = pool.exec(req).await;
        assert!(
            out.err.is_none() || matches!(out.err, Some(runtime_exec::ExecError::NoResult)),
            "selector {q:?} crashed: {:?}",
            out.err
        );
        let got = out.token.map(|t| t.to_string()).unwrap_or_default();
        assert_eq!(got, "null", "selector {q:?} must yield null, got {got}");
    }
}

#[tokio::test]
async fn oversized_selector_is_rejected_not_walked() {
    let page = doc(b"<html><body><div id=\"ok\">t</div></body></html>");
    let huge = "div.class ".repeat(60);
    let script = format!(
        "(function () {{ var r = document.querySelector(\"{huge}\"); return r === null ? 'null' : 'hit'; }})()"
    );
    let out = run(&script, page).await.expect("no crash");
    assert_eq!(out.as_deref(), Some("null"));
}

#[tokio::test]
async fn attr_selector_garbage_and_valid_both_behave() {
    let page = doc(
        b"<html><body><input type=\"hidden\" name=\"csrf\" value=\"tok1\"><input type=\"text\"></body></html>",
    );
    let script = r#"(function () {
        var bad = document.querySelector('input[novalue=');
        var hit = document.querySelector('input[type="hidden"]');
        var miss = document.querySelector('input[type="checkbox"]');
        return bad === null && hit !== null && miss === null ? 'ok' : 'broken';
    })()"#;
    let out = run(script, page).await.expect("no crash");
    assert_eq!(out.as_deref(), Some("ok"));
}

#[tokio::test]
async fn zero_and_giant_canvas_dims_do_not_panic() {
    let page = doc(b"<html><body></body></html>");
    let script = r#"(function () {
        var c = document.createElement('canvas');
        c.width = 0; c.height = 0;
        var a = c.toDataURL();
        c.width = 5; c.height = 3;
        var ctx = c.getContext('2d');
        ctx.fillRect(0, 0, 2, 2);
        var b = c.toDataURL();
        var b2 = c.toDataURL();
        c.width = 4000000; c.height = 4000000;
        var d = c.toDataURL();
        return a.indexOf('data:image/png;base64,') === 0 && b === b2 && d.indexOf('data:image/png;base64,') === 0 ? 'ok' : 'broken';
    })()"#;
    let out = run(script, page).await.expect("no crash");
    assert_eq!(out.as_deref(), Some("ok"));
}

#[tokio::test]
async fn get_image_data_rejects_garbage_geometry() {
    let page = doc(b"<html><body></body></html>");
    let script = r#"(function () {
        var c = document.createElement('canvas');
        c.width = 8; c.height = 8;
        var ctx = c.getContext('2d');
        var bad = 0, good = null;
        try { ctx.getImageData(0, 0, 0, 0); } catch (e) { bad++; }
        try { ctx.getImageData(0, 0, -1, 4); } catch (e) { bad++; }
        try { ctx.getImageData(0, 0, NaN, 4); } catch (e) { bad++; }
        try { good = ctx.getImageData(0, 0, 4, 4); } catch (e) { bad++; }
        return bad === 3 && good && good.width === 4 && good.height === 4 && good.data.length === 64 ? 'ok' : 'broken:' + bad + ':' + (good && good.width);
    })()"#;
    let out = run(script, page).await.expect("no crash");
    assert_eq!(out.as_deref(), Some("ok"));
}
