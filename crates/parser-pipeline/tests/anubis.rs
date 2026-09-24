use parser_pipeline::{Limits, StreamPipeline};

const PAGE: &str = r#"<!DOCTYPE html>
<html><head>
<script id="anubis_version" type="application/json">"v1.25.0"</script>
<script id="anubis_challenge" type="application/json">
{"rules":{"algorithm":"fast","difficulty":4,"report_as":4},
 "challenge":{
   "issuedAt":"2026-09-06T17:25:44.720860281Z",
   "id":"01a07817-7a86-7e54-8fce-36e99de23289",
   "method":"fast",
   "randomData":"abcdefghij0123456789",
   "difficulty":4}}
</script>
<title>Making sure the internet works</title>
</head><body><h1>Checking your browser</h1></body></html>"#;

#[test]
fn anubis_scripts_captured() {
    let mut p = StreamPipeline::new(Limits::default());
    assert_eq!(
        p.push(PAGE.as_bytes()).unwrap(),
        parser_pipeline::Flow::Continue
    );
    let page = p.finish().unwrap();
    let anubis = page.anubis.as_ref().expect("anubis json");
    assert!(anubis.windows(10).any(|w| w == b"randomData"));
    assert!(
        anubis
            .windows(36)
            .any(|w| w == b"01a07817-7a86-7e54-8fce-36e99de23289")
    );
    assert_eq!(page.anubis_version.as_deref(), Some("v1.25.0"));
}

#[test]
fn anubis_absent_on_plain_page() {
    let html =
        b"<html><head><title>plain</title></head><body><script>var x = 1;</script></body></html>";
    let mut p = StreamPipeline::new(Limits::default());
    let _ = p.push(html).unwrap();
    let page = p.finish().unwrap();
    assert!(page.anubis.is_none());
    assert!(page.anubis_version.is_none());
}
