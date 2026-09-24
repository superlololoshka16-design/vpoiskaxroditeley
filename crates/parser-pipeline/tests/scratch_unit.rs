use parser_pipeline::{Flow, Limits, StreamPipeline};

#[test]
fn scratch_pool_overflow_is_visible_in_parse_errors() {
    let mut html = Vec::with_capacity(700 * 1024);
    html.extend_from_slice(b"<html><body><div id=\"m\">");
    html.extend(std::iter::repeat(b'a').take(600 * 1024));
    html.extend_from_slice(b"</div></body></html>");
    let mut p = StreamPipeline::new(Limits::default());
    let _ = p.push(&html).expect("push");
    let page = p.finish().expect("finish");
    assert!(
        page.parse_errors >= 1,
        "потеря контента на pool-overflow обязана быть видна в parse_errors"
    );
}

#[test]
fn empty_stream_yields_empty_page() {
    let mut p = StreamPipeline::new(Limits::default());
    assert_eq!(p.push(b"").unwrap(), Flow::Continue);
    let page = p.finish().unwrap();
    assert_eq!(page.title, None);
    assert_eq!(page.parse_errors, 0);
    assert_eq!(page.bytes_fed, 0);
    assert!(!page.truncated);
}

#[test]
fn binary_garbage_never_panics() {
    let mut p = StreamPipeline::new(Limits::default());
    let garbage: Vec<u8> = (0..256u32)
        .map(|i| (i % 256) as u8)
        .cycle()
        .take(8192)
        .collect();
    let _ = p.push(&garbage).unwrap();
    let page = p.finish().unwrap();
    assert_eq!(page.bytes_fed, 8192);
}

#[test]
fn cut_mid_document_keeps_collected_data() {
    let mut p = StreamPipeline::new(Limits::default());
    p.push(b"<html><head><title>Half-open</title><div id=\"x\" cla")
        .unwrap();
    let page = p.finish().unwrap();
    assert_eq!(page.title.as_deref(), Some("Half-open"));
}
