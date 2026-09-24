use parser_pipeline::{Flow, StreamPipeline};

#[test]
fn example_com_page_parses_clean() {
    let html = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/example_com.html"
    ))
    .expect("fixture");
    let mut p = StreamPipeline::new(Default::default());
    for chunk in html.as_bytes().chunks(97) {
        let r = p.push(chunk).expect("push");
        if r == Flow::Stop {
            break;
        }
    }
    let page = p.finish().expect("finish");
    assert_eq!(page.title.as_deref(), Some("Example Domain"));
    assert!(page.dom.len() >= 8, "dom too small: {}", page.dom.len());
    assert_eq!(page.utf8_bad_chunks, 0);
}
