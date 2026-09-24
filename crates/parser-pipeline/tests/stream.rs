use core_utils::BytesExt as _;
use parser_pipeline::{Flow, StreamPipeline};

const PAGE: &str = r#"<html><head><title>Sign in</title>
<script id="__NEXT_DATA__" type="application/json">{"props":{"user":"x"},"page":"/login","buildId":"b77","query":"from=home"}</script>
<script src="/static/js/chunk-a91.js"></script></head><body>
<form action="/login" method="POST">
<input type="text" name="user" value="">
<input type="hidden" name="csrf_token" value="tok_9f21ab">
<input type="password" name="pass">
</form>
<script>
var _0x1a2b = 48879, _0x9f8e = "seed-one";
var mix = _0x1a2b + 7;
var entropy = [11, 23, 37, 41, 53, 67, 79, 83, 97, 101, 103];
var acc = 0;
for (var q = 0; q < 11; q++) { acc += entropy[q] * (q + 1); }
eval("1"), atob("YQ=="), setTimeout(function(){}, 0);
String.fromCharCode(65), charCodeAt(0);
if (acc > 0) { acc = acc ^ 0x5F3759DF; }
"challenge-final:" + mix + _0x9f8e + ":" + acc;
</script>
</body></html>"#;

fn feed_chunks(p: &mut StreamPipeline, html: &[u8], n: usize) {
    let step = html.len().div_ceil(n);
    for i in (0..html.len()).step_by(step) {
        let end = (i + step).min(html.len());
        if p.push(&html[i..end]).unwrap() == Flow::Stop {
            break;
        }
    }
}

#[test]
fn telemetry_route_detected_from_page_surface() {
    let turnstile = parser_pipeline::detect_route(
        &["https://challenges.cloudflare.com/turnstile/v0/api.js"],
        &[],
    )
    .expect("turnstile route");
    assert_eq!(
        turnstile.provider,
        parser_pipeline::TelemetryProvider::Turnstile
    );
    assert_eq!(turnstile.transport, parser_pipeline::Transport::CdnPost);

    let datadome = parser_pipeline::detect_route(&["https://js.datadome.co/tags.js"], &[])
        .expect("datadome route");
    assert_eq!(
        datadome.provider,
        parser_pipeline::TelemetryProvider::DataDome
    );
    assert_eq!(datadome.field.as_str(), "datadome");

    let inhouse = parser_pipeline::detect_route(
        &[],
        b"fetch('/telemetry', {method:'POST'})",
    )
    .expect("in-house route");
    assert_eq!(
        inhouse.provider,
        parser_pipeline::TelemetryProvider::InHouse
    );
    assert_eq!(inhouse.endpoint.as_str(), "/telemetry");

    assert!(parser_pipeline::detect_route(&["/static/app.js"], b"console.log(1)").is_none());
}

#[test]
fn whole_page_single_pass() {
    let mut p = StreamPipeline::new(Default::default());
    feed_chunks(&mut p, PAGE.as_bytes(), 1);
    let page = p.finish().unwrap();
    assert_eq!(page.title.as_deref(), Some("Sign in"));
    assert_eq!(page.next_data.as_ref().unwrap().build_id, "b77");
    assert_eq!(page.next_data.as_ref().unwrap().page, "/login");
    assert_eq!(page.tokens.first().map(|t| t.as_str()), Some("tok_9f21ab"));
    assert_eq!(page.forms.len(), 1);
    assert_eq!(page.forms[0].fields.len(), 3);
    assert_eq!(page.forms[0].method, "post");
    assert_eq!(page.script_srcs.len(), 1);
    assert!(
        page.challenge.is_some(),
        "obfuscated script must be flagged"
    );
    assert!(page.dom.scripts.len() >= 2, "script nodes indexed");
    assert_eq!(page.dom.forms.len(), 1, "form indexed");
    assert!(page.dom.inputs.len() >= 3, "input nodes indexed");
    assert!(
        page.dom
            .tag_name(page.dom.tag_id(page.dom.title_node.unwrap()))
            .is_some_and(|t| t == "title")
    );
    assert_eq!(page.dom.script_attr(0, "id"), Some("__NEXT_DATA__"));
    assert_eq!(
        page.dom.script_attr(1, "src"),
        Some("/static/js/chunk-a91.js")
    );
    assert_eq!(page.utf8_bad_chunks, 0);
    assert!(!page.truncated);
    assert!(!page.dom.truncated());
}

#[test]
fn script_split_across_many_chunk_boundaries() {
    let mut p = StreamPipeline::new(Default::default());
    feed_chunks(&mut p, PAGE.as_bytes(), 37);
    let page = p.finish().unwrap();
    let ch = page.challenge.expect("challenge survives chunk splits");
    let text = String::from_utf8_lossy(&ch);
    assert!(text.contains("challenge-final"));
    assert_eq!(page.tokens.first().map(|t| t.as_str()), Some("tok_9f21ab"));
    assert_eq!(page.next_data.as_ref().unwrap().build_id, "b77");
}

#[test]
fn broken_utf8_chunk_does_not_panic() {
    let mut p = StreamPipeline::new(Default::default());
    p.push(PAGE.as_bytes()).unwrap();
    let garbage = [0xFF, 0xFE, 0x80, b'<', b'd', b'i', 0xC0, b'>'];
    let flow = p.push(&garbage).unwrap();
    assert_eq!(flow, Flow::Continue);
    p.push(b"</html>").unwrap();
    let page = p.finish().unwrap();
    assert_eq!(page.utf8_bad_chunks, 1);
    assert_eq!(page.title.as_deref(), Some("Sign in"));
}

#[test]
fn empty_and_garbage_inputs_do_not_panic() {
    let empty = StreamPipeline::new(Default::default()).finish().unwrap();
    assert!(empty.forms.is_empty());
    let mut p = StreamPipeline::new(Default::default());
    p.push(b"\x00\x01\x02random\xff\xffgarbage").unwrap();
    let page = p.finish().unwrap();
    assert!(page.utf8_bad_chunks >= 1 || page.bytes_fed > 0);
}

#[test]
fn byte_brake_truncates_gracefully() {
    let mut p = StreamPipeline::new(parser_pipeline::Limits {
        byte_brake: 2048,
        ..Default::default()
    });
    let flow = p.push(&[b'x'; 4096]).unwrap();
    assert_eq!(flow, Flow::Stop);
    let page = p.finish().unwrap();
    assert!(page.truncated);
    assert!(page.bytes_fed >= 2048);
}

#[test]
fn cut_next_data_json_is_not_a_crash() {
    let html = r#"<html><script id="__NEXT_DATA__">{"props":{"user":"x"},"page":"/lo"#;
    let mut p = StreamPipeline::new(Default::default());
    p.push(html.as_bytes()).unwrap();
    let page = p.finish().unwrap();
    assert!(page.next_data.is_none());
    assert_eq!(page.parse_errors, 1);
}

#[test]
fn oversized_script_is_not_flagged_as_challenge() {
    let mut big = String::with_capacity(600 * 1024);
    big.push_str("<script>");
    for i in 0..60_000 {
        big.push_str(&format!("var x{i}=i;"));
    }
    big.push_str("</script>");
    let mut p = StreamPipeline::new(Default::default());
    p.push(big.as_bytes()).unwrap();
    let page = p.finish().unwrap();
    assert!(page.challenge.is_none());
    assert!(page.next_data.is_none());
}

#[test]
fn plain_script_below_probe_threshold_is_ignored() {
    let html = r#"<script>var a = 1; a + 2;</script>"#;
    let mut p = StreamPipeline::new(Default::default());
    p.push(html.as_bytes()).unwrap();
    let page = p.finish().unwrap();
    assert!(page.challenge.is_none());
    assert_eq!(page.inline_count, 1);
}

#[test]
fn challenge_marker_in_script_src_is_detected() {
    let html = r#"<html><head>
<script src="https://challenges.cloudflare.com/turnstile/v0/api.js"></script>
</head><body></body></html>"#;
    let mut p = StreamPipeline::new(Default::default());
    p.push(html.as_bytes()).unwrap();
    let page = p.finish().unwrap();
    assert_eq!(page.challenge_markers.len(), 1);
    assert_eq!(
        page.challenge_script_url.as_deref(),
        Some("https://challenges.cloudflare.com/turnstile/v0/api.js")
    );
}

#[test]
fn adaptive_filter_keeps_only_target_tree() {
    let mut html = String::from("<html><head><title>T</title></head><body>");
    for i in 0..64 {
        html.push_str(&format!("<div><p>filler {i}</p><span>x</span></div>"));
    }
    html.push_str("<form action=\"/go\" method=\"post\"><input type=\"hidden\" name=\"t\" value=\"v\"></form>");
    html.push_str("</body></html>");
    let mut p = StreamPipeline::new(Default::default());
    p.push(html.as_bytes()).unwrap();
    let page = p.finish().unwrap();
    assert_eq!(page.forms.len(), 1);
    assert_eq!(page.dom.inputs.len(), 1);
    assert!(
        page.dom.len() < 24,
        "junk must be filtered out, got {} nodes",
        page.dom.len()
    );
    assert_eq!(page.dom.forms.len(), 1);
}

#[test]
fn container_with_id_stays_in_tree() {
    let html =
        r#"<html><body><div id="app"><p>inner</p></div><div><p>junk</p></div></body></html>"#;
    let mut p = StreamPipeline::new(Default::default());
    p.push(html.as_bytes()).unwrap();
    let page = p.finish().unwrap();
    let dom = &page.dom;
    let app = find_by_tag(dom, "div").expect("app div");
    assert_eq!(
        dom.attr(app, parser_pipeline::ATTR_NAMES["id"]),
        Some("app")
    );
    let mut count = 0;
    let mut q: std::collections::VecDeque<u32> = dom.children(u32::MAX).collect();
    while let Some(n) = q.pop_front() {
        if dom.tag_name(dom.tag_id(n)) == Some("div") {
            count += 1;
        }
        q.extend(dom.children(n));
    }
    assert_eq!(count, 1, "only the marked div survives");
}

#[test]
fn target_inside_skipped_subtree_still_captured() {
    let html = r#"<div><section><p>noise</p><form action="/x"><input name="csrf" type="hidden" value="z"></form></section></div>"#;
    let mut p = StreamPipeline::new(Default::default());
    p.push(html.as_bytes()).unwrap();
    let page = p.finish().unwrap();
    assert_eq!(page.forms.len(), 1);
    assert_eq!(page.forms[0].fields.len(), 1);
    assert_eq!(page.tokens.first().map(|t| t.as_str()), Some("z"));
    assert_eq!(page.dom.forms.len(), 1);
}

#[test]
fn text_whitelist_and_truncation() {
    let long = "x".repeat(400);
    let html = format!(
        r##"<html><body>
<div id="m">{long}</div>
<button>btext</button>
<a href="#">atext</a>
<div>plain-no-marker-text</div>
</body></html>"##
    );
    let mut p = StreamPipeline::new(Default::default());
    p.push(html.as_bytes()).unwrap();
    let page = p.finish().unwrap();
    let dom = &page.dom;
    let div = find_by_tag(dom, "div").expect("marker div");
    let dt = dom.children(div).next().expect("div text");
    let t = dom.text(dt).unwrap();
    assert!(
        t.len() <= 128,
        "marker container text truncated, got {}",
        t.len()
    );
    let a = find_by_tag(dom, "a").expect("a");
    let at = dom.children(a).next().expect("a text");
    assert_eq!(dom.text(at), Some("atext"));
    let btn = find_by_tag(dom, "button").expect("button");
    let bt = dom.children(btn).next().expect("btn text");
    assert_eq!(dom.text(bt), Some("btext"));
    let mut plain = None;
    let mut q: std::collections::VecDeque<u32> = dom.children(u32::MAX).collect();
    while let Some(n) = q.pop_front() {
        if dom.tag_name(dom.tag_id(n)) == Some("div")
            && dom.attr(n, parser_pipeline::ATTR_NAMES["id"]).is_none()
        {
            plain = Some(n);
            break;
        }
        q.extend(dom.children(n));
    }
    assert!(plain.is_none(), "plain div must be filtered");
}

#[test]
fn dom_tree_structure_and_text() {
    let html = r##"<html><body><div id="app" class="root"><button>alpha</button><label>beta</label></div><a href="#">tail</a></body></html>"##;
    let mut p = StreamPipeline::new(Default::default());
    p.push(html.as_bytes()).unwrap();
    let page = p.finish().unwrap();
    let dom = &page.dom;
    let div = find_by_tag(dom, "div").expect("div");
    assert_eq!(
        dom.attr(div, parser_pipeline::ATTR_NAMES["id"]),
        Some("app")
    );
    assert_eq!(
        dom.attr(div, parser_pipeline::ATTR_NAMES["class"]),
        Some("root")
    );
    let mut kids = dom.children(div);
    let b1 = kids.next().expect("button1");
    let b2 = kids.next().expect("label1");
    let t1 = dom.children(b1).next().expect("text1");
    let t2 = dom.children(b2).next().expect("text2");
    assert_eq!(dom.text(t1), Some("alpha"));
    assert_eq!(dom.text(t2), Some("beta"));
    assert_eq!(dom.parent(t1), Some(b1));
    assert_eq!(dom.parent(b1), Some(div));
    assert_eq!(dom.parent(b2), Some(div));
    let body = dom.parent(div).expect("body");
    let a = dom.children(body).nth(1).expect("a");
    assert_eq!(dom.text(dom.children(a).next().unwrap()), Some("tail"));
}

fn find_by_tag(dom: &parser_pipeline::DomTree, tag: &str) -> Option<u32> {
    let mut queue: std::collections::VecDeque<u32> = dom.children(u32::MAX).collect();
    let mut guard = 0;
    while let Some(n) = queue.pop_front() {
        guard += 1;
        if guard > 100_000 {
            break;
        }
        if dom.tag_name(dom.tag_id(n)) == Some(tag) {
            return Some(n);
        }
        queue.extend(dom.children(n));
    }
    None
}

#[test]
fn dom_sibling_rules_and_generations() {
    let html = b"<select id=\"s\"><option>one<option>two</select>";
    let mut p = StreamPipeline::new(Default::default());
    p.push(html).unwrap();
    let mut page = p.finish().unwrap();
    let dom = &mut page.dom;
    let sel = find_by_tag(dom, "select").expect("select");
    let mut kids = dom.children(sel);
    let oa = kids.next().expect("option a");
    let ob = kids.next().expect("option b");
    assert_eq!(
        dom.parent(ob),
        Some(sel),
        "sibling rule: option closes option"
    );
    assert_ne!(oa, ob);
    let id = dom.node_id(oa);
    assert!(dom.is_valid(id));
    assert!(dom.remove_node(id));
    assert!(!dom.is_valid(id), "generation bump invalidates stale id");
    let first = dom.children(sel).next().expect("first child now");
    assert_ne!(first, oa);
}

#[test]
fn b64_field_decodes_and_rejects_garbage() {
    use smallvec::SmallVec;
    let mut out: SmallVec<[u8; 4096]> = SmallVec::new();
    let n = b"  U2lsbyByb2Nrcw== ".b64_field_decode(&mut out, 64 * 1024).unwrap();
    assert_eq!(&out[..n], b"Silo rocks");
    out.clear();
    assert!(b"!!!not-base64!!!".b64_field_decode(&mut out, 64 * 1024).is_err());
}
