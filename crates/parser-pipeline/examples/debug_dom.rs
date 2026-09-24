use parser_pipeline::StreamPipeline;

fn main() {
    let html = r#"<html><head><title>T</title>
<script id="__NEXT_DATA__" type="application/json">{"page":"/x"}</script>
<script src="/a.js"></script></head><body><div id="app" class="root"><p>alpha</p></div></body></html>"#;
    let mut p = StreamPipeline::new(Default::default());
    p.push(html.as_bytes()).unwrap();
    let page = p.finish().unwrap();
    let dom = &page.dom;
    eprintln!(
        "nodes={} scripts={} attrs={} pool={}",
        dom.len(),
        dom.scripts.len(),
        dom.attr_count_total(),
        dom.pool_len()
    );
    for (i, &s) in dom.scripts.iter().enumerate().take(6) {
        eprintln!(
            "script[{i}] node={s} tag={:?} id={:?} src={:?} attrs={:?}",
            dom.tag_name(dom.tag_id(s)),
            dom.script_attr(i, "id"),
            dom.script_attr(i, "src"),
            dom.attrs_of(s).collect::<Vec<_>>()
        );
    }
}
