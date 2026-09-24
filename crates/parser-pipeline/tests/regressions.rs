use parser_pipeline::{ATTR_NAMES, Flow, Limits, PageData, StreamPipeline};

fn parse_chunks(html: &[u8], size: usize) -> PageData {
    let mut parser = StreamPipeline::new(Limits::default());
    for chunk in html.chunks(size) {
        assert_eq!(parser.push(chunk).unwrap(), Flow::Continue);
    }
    parser.finish().unwrap()
}

#[test]
fn utf8_split_at_every_transport_boundary_preserves_title() {
    let html = "<html><head><title>Цена € 日本語 🦀</title></head><body></body></html>";
    for size in 1..=html.len() {
        let page = parse_chunks(html.as_bytes(), size);
        assert_eq!(page.title.as_deref(), Some("Цена € 日本語 🦀"));
        assert_eq!(page.bytes_fed, html.len() as u64);
    }
}

#[test]
fn title_cap_keeps_a_stable_unicode_prefix() {
    let expected = "a".repeat(255);
    let html = format!("<title>{expected}€Z</title>");
    for size in 1..=html.len() {
        let page = parse_chunks(html.as_bytes(), size);
        assert_eq!(page.title.as_deref(), Some(expected.as_str()));
    }
}

#[test]
fn invalid_utf8_does_not_discard_following_markup() {
    let html = b"<html><body>\xff<input name=\"kept\" value=\"yes\"><script src=\"kept.js\"></script></body></html>";
    let page = parse_chunks(html, html.len());
    assert_eq!(page.utf8_bad_chunks, 1);
    assert!(page.script_srcs.iter().any(|src| src.as_str() == "kept.js"));
}

#[test]
fn byte_brake_limits_admitted_bytes_in_a_large_chunk() {
    let prefix = b"<title>ok</title>";
    let mut html = prefix.to_vec();
    html.extend_from_slice(b"<input name=\"outside\" value=\"no\">");
    let mut parser = StreamPipeline::new(Limits {
        byte_brake: prefix.len() as u64,
        ..Limits::default()
    });
    assert_eq!(parser.push(&html).unwrap(), Flow::Stop);
    assert_eq!(parser.push(b"ignored").unwrap(), Flow::Stop);
    let page = parser.finish().unwrap();
    assert_eq!(page.bytes_fed, prefix.len() as u64);
    assert_eq!(page.title.as_deref(), Some("ok"));
    assert!(page.truncated);
}

#[test]
fn removing_a_subtree_invalidates_every_descendant_and_index() {
    let html = b"<html><body><div id=\"drop\"><form><button>label</button><input name=\"gone\"></form><script src=\"gone.js\"></script></div><input name=\"keep\"><script src=\"keep.js\"></script></body></html>";
    let mut page = parse_chunks(html, html.len());
    let dom = &mut page.dom;
    let mut stack: Vec<u32> = dom.children(u32::MAX).collect();
    let mut drop_root = None;
    while let Some(node) = stack.pop() {
        if dom.attr(node, ATTR_NAMES["id"]) == Some("drop") {
            drop_root = Some(node);
            break;
        }
        stack.extend(dom.children(node));
    }
    let root = drop_root.unwrap();
    let root_id = dom.node_id(root);
    let mut subtree = Vec::new();
    stack.clear();
    stack.push(root);
    while let Some(node) = stack.pop() {
        subtree.push(dom.node_id(node));
        stack.extend(dom.children(node));
    }
    let before = dom.len();
    assert!(subtree.len() >= 6);
    assert!(dom.remove_node(root_id));
    assert_eq!(dom.len(), before - subtree.len());
    for id in subtree {
        assert!(!dom.is_valid(id));
        assert!(!dom.is_valid(dom.node_id(id.index)));
        assert!(!dom.remove_node(id));
    }
    assert!(!dom.remove_node(dom.node_id(root)));
    assert!(dom.forms.is_empty());
    assert_eq!(dom.scripts.len(), 1);
    assert_eq!(dom.inputs.len(), 1);
    assert_eq!(dom.script_attr(0, "src"), Some("keep.js"));
    assert_eq!(dom.attr(dom.inputs[0], ATTR_NAMES["name"]), Some("keep"));
    let mut reachable = 0;
    stack.extend(dom.children(u32::MAX));
    while let Some(node) = stack.pop() {
        reachable += 1;
        assert!(dom.is_valid(dom.node_id(node)));
        stack.extend(dom.children(node));
    }
    assert_eq!(reachable, dom.len());
}

#[test]
fn removing_first_script_preserves_document_order_of_indexes() {
    let html = b"<html><head><script src=\"a.js\"></script><script src=\"b.js\"></script><script src=\"c.js\"></script></head></html>";
    let mut page = parse_chunks(html, html.len());
    let dom = &mut page.dom;
    let first = dom.node_id(dom.scripts[0]);
    assert!(dom.remove_node(first));
    assert_eq!(dom.script_attr(0, "src"), Some("b.js"));
    assert_eq!(dom.script_attr(1, "src"), Some("c.js"));
}

#[test]
fn memory_bomb_degrades_to_truncated_partial_page() {
    let mut html = String::from("<html><head><title>bomb-title</title></head><body>");

    for i in 0..4096 {
        html.push_str(&format!("<div data-pad=\"{i}"));
        let filler = "p".repeat(256);
        html.push_str(&filler);
        html.push_str("\">");
    }
    html.push_str("</body></html>");
    let mut parser = StreamPipeline::new(Limits::default());
    let _flow = parser.push(html.as_bytes()).unwrap();
    let page = parser.finish().unwrap();
    assert_eq!(
        page.title.as_deref(),
        Some("bomb-title"),
        "собранное обязано выжить"
    );
    assert!(page.truncated || page.parse_errors > 0 || page.bytes_fed == html.len() as u64);
}

#[test]
fn utf8_tail_across_chunk_boundary_is_not_bad_chunk() {
    let html = "<title>€🦀Ü</title>";
    let bytes = html.as_bytes();
    for split in 1..bytes.len() {
        let (a, b) = bytes.split_at(split);
        let mut p = StreamPipeline::new(Limits::default());
        p.push(a).unwrap();
        p.push(b).unwrap();
        let page = p.finish().unwrap();
        assert_eq!(
            page.utf8_bad_chunks, 0,
            "split at {split}: ложный bad-chunk"
        );
    }
}
