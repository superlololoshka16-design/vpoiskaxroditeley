use runtime_exec::WorkerPool;
mod common;
use common::{page_doc, pool, req};



async fn eval(pool: &WorkerPool, script: &str) -> String {
    let o = pool.exec(req(script, 3000)).await;
    o.token
        .unwrap_or_else(|| panic!("js error: {:?}", o.err))
        .as_str()
        .to_string()
}

async fn eval_doc(pool: &WorkerPool, script: &str) -> String {
    let mut r = req(script, 3000);
    r.doc = Some(page_doc());
    let o = pool.exec(r).await;
    o.token
        .unwrap_or_else(|| panic!("js error: {:?}", o.err))
        .as_str()
        .to_string()
}

#[tokio::test]
async fn mo_records_carry_full_chrome_fields() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "window.__snap = null;\
         var d = document.createElement('div');\
         var mo = new MutationObserver(function (recs) {\
             window.__snap = [recs.length,\
              recs[0].type, recs[0].attributeName, String(recs[0].oldValue),\
              recs[1].type, recs[1].attributeName, String(recs[1].oldValue),\
              recs[2].type, recs[2].addedNodes.length, recs[2].addedNodes[0] === child ? 'same' : typeof recs[2].addedNodes[0],\
              String(recs[2].previousSibling), String(recs[2].nextSibling), recs[2].target === d,\
              Object.prototype.toString.call(recs[0])].join('|');\
         });\
         mo.observe(d, { childList: true, attributes: true, attributeOldValue: true });\
         d.setAttribute('data-a', '1');\
         d.setAttribute('data-a', '2');\
         var child = document.createElement('span');\
         d.appendChild(child);\
         'kick';",
    )
    .await;
    assert_eq!(out, "kick");
    let check = eval(&pool, "window.__snap;").await;
    assert_eq!(
        check,
        "3|attributes|data-a|null|attributes|data-a|1|childList|1|same|null|null|true|[object MutationRecord]",
        "records: {check}"
    );
}

#[tokio::test]
async fn mo_take_records_drains_and_silences_callback() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "window.__fired = 0;\
         window.__taken = -1;\
         var d = document.createElement('div');\
         var mo = new MutationObserver(function (recs) { window.__fired += recs.length; });\
         mo.observe(d, { childList: true });\
         d.appendChild(document.createElement('p'));\
         var t = mo.takeRecords();\
         window.__taken = t.length;\
         'drain';",
    )
    .await;
    assert_eq!(out, "drain");
    let check = eval(&pool, "[window.__taken, window.__fired].join('|');").await;
    assert_eq!(check, "1|0", "take: {check}");
}

#[tokio::test]
async fn mo_dirty_inputs_throw_chrome_exact_errors() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "var out = [];\
         var d = document.createElement('div');\
         var mo = new MutationObserver(function () {});\
         try { mo.observe(d, {}); } catch (e) { out.push('empty:' + e.name + ':' + (e.message.indexOf(\"The options object must set at least one of\") !== -1)); }\
         try { mo.observe(123, { childList: true }); } catch (e) { out.push('badtarget:' + e.name + ':' + (e.message.indexOf(\"parameter 1 is not of type 'Node'\") !== -1)); }\
         try { mo.observe(d, { attributeOldValue: true }); } catch (e) { out.push('oldflag:' + e.name); }\
         try { mo.observe(d, { characterDataOldValue: true }); } catch (e) { out.push('charflag:' + e.name); }\
         try { new MutationObserver('nope'); } catch (e) { out.push('ctor:' + e.name + ':' + (e.message.indexOf(\"parameter 1 is not of type 'Function'\") !== -1)); }\
         out.push('len:' + MutationObserver.length);\
         out.push('str:' + (String(MutationObserver).indexOf('native code') !== -1));\
         out.join('|');",
    )
    .await;
    assert_eq!(
        out,
        "empty:TypeError:true|badtarget:TypeError:true|oldflag:TypeError|charflag:TypeError|ctor:TypeError:true|len:1|str:true",
        "mo dirty: {out}"
    );
}

#[tokio::test]
async fn mo_character_data_records_with_old_value() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "window.__snap = null;\
         var t = document.createTextNode('before');\
         var mo = new MutationObserver(function (recs) {\
             window.__snap = [recs.length, recs[0].type, String(recs[0].oldValue), recs[0].target.textContent, String(recs[0].attributeName)].join('|');\
         });\
         mo.observe(t, { characterData: true, characterDataOldValue: true });\
         t.textContent = 'after';\
         'mut';",
    )
    .await;
    assert_eq!(out, "mut");
    let check = eval(&pool, "window.__snap;").await;
    assert_eq!(check, "1|characterData|before|after|null", "cdata: {check}");
}

#[tokio::test]
async fn mo_subtree_and_attribute_filter() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "window.__hits = 0;\
         window.__names = [];\
         var d = document.createElement('div');\
         var mo = new MutationObserver(function (recs) {\
             window.__hits += recs.length;\
             for (var i = 0; i < recs.length; i++) { window.__names.push(recs[i].attributeName); }\
         });\
         mo.observe(d, { attributes: true, attributeFilter: ['data-keep'], subtree: true });\
         var child = document.createElement('span');\
         d.appendChild(child);\
         child.setAttribute('data-skip', '1');\
         child.setAttribute('data-keep', '1');\
         'filter';",
    )
    .await;
    assert_eq!(out, "filter");
    let check = eval(
        &pool,
        "[window.__hits, window.__names.join(',')].join('|');",
    )
    .await;
    assert_eq!(check, "1|data-keep", "filter: {check}");
}

#[tokio::test]
async fn ro_fires_initial_entry_with_real_geometry() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "window.__entries = null;\
         var gate = document.getElementById('gate');\
         window.__gate = gate;\
         var ro = new ResizeObserver(function (entries) { window.__entries = entries; });\
         ro.observe(gate);\
         'obs';",
    )
    .await;
    assert_eq!(out, "obs");
    let check = eval(
        &pool,
        "var e = window.__entries[0];\
         [window.__entries.length, e.target === window.__gate, e.contentRect.width > 0, e.contentRect.height > 0,\
          e.borderBoxSize[0].inlineSize === e.contentRect.width,\
          Object.prototype.toString.call(e.contentRect)].join('|');",
    )
    .await;
    assert_eq!(
        check, "1|true|true|true|true|[object DOMRectReadOnly]",
        "ro: {check}"
    );
}

#[tokio::test]
async fn ro_rejects_non_element_targets() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "var out = [];\
         var ro = new ResizeObserver(function () {});\
         try { ro.observe(document.createTextNode('x')); } catch (e) { out.push('text:' + e.name + ':' + (e.message.indexOf(\"parameter 1 is not of type 'Element'\") !== -1)); }\
         try { ro.observe({}); } catch (e) { out.push('obj:' + e.name); }\
         try { ro.observe(); } catch (e) { out.push('none:' + e.name); }\
         try { new ResizeObserver('x'); } catch (e) { out.push('ctor:' + e.name); }\
         out.push('proto:' + (typeof ResizeObserver.prototype.disconnect));\
         out.join('|');",
    )
    .await;
    assert_eq!(
        out, "text:TypeError:true|obj:TypeError|none:TypeError|ctor:TypeError|proto:function",
        "ro dirty: {out}"
    );
}

#[tokio::test]
async fn io_threshold_validation_and_initial_delivery() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "window.__entries = null;\
         var bad = '';\
         try { new IntersectionObserver(function () {}, { threshold: 2 }); } catch (e) { bad = e.name + ':' + (e.message.indexOf('Threshold values must be numbers between 0 and 1') !== -1); }\
         var gate = document.getElementById('gate');\
         window.__gate = gate;\
         var io = new IntersectionObserver(function (entries) { window.__entries = entries; });\
         io.observe(gate);\
         window.__bad = bad;\
         'io';",
    )
    .await;
    assert_eq!(out, "io");
    let check = eval(
        &pool,
        "var e = window.__entries[0];\
         [window.__entries.length, e.target === window.__gate, e.isIntersecting, e.intersectionRatio > 0,\
          e.rootBounds.width === 1920, e.time > 0, e.boundingClientRect.width > 0].join('|');",
    )
    .await;
    assert_eq!(check, "1|true|true|true|true|true|true", "io: {check}");
    let bad_check = eval_doc(
        &pool,
        "var bad = '';\
         try { new IntersectionObserver(function () {}, { threshold: 1.5 }); } catch (e) { bad = e.name; }\
         bad;",
    )
    .await;
    assert_eq!(bad_check, "RangeError", "threshold: {bad_check}");
    let _ = out;
}

#[tokio::test]
async fn io_surface_properties_match_chrome() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "var io = new IntersectionObserver(function () {});\
         var io2 = new IntersectionObserver(function () {}, { threshold: [0, 0.5, 1] });\
         [String(io.root), io.rootMargin, io.thresholds.join(','), io2.thresholds.join(','),\
          Object.prototype.toString.call(io)].join('|');",
    )
    .await;
    assert_eq!(
        out, "null|0px 0px 0px 0px|0|0,0.5,1|[object IntersectionObserver]",
        "io surface: {out}"
    );
}

#[tokio::test]
async fn observer_prototypes_and_instances_link() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "var mo = new MutationObserver(function () {});\
         var ro = new ResizeObserver(function () {});\
         var io = new IntersectionObserver(function () {});\
         [mo instanceof MutationObserver, ro instanceof ResizeObserver, io instanceof IntersectionObserver,\
          typeof mo.observe, typeof ro.unobserve, typeof io.takeRecords,\
          Object.prototype.toString.call(mo), Object.prototype.toString.call(ro), Object.prototype.toString.call(io)].join('|');",
    )
    .await;
    assert_eq!(
        out,
        "true|true|true|function|function|function|[object MutationObserver]|[object ResizeObserver]|[object IntersectionObserver]",
        "inst: {out}"
    );
}

#[tokio::test]
async fn mo_mass_mutation_above_cap_no_silent_drop() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "window.__n = 0;\
         var target = document.body;\
         var mo = new MutationObserver(function (recs) { window.__n += recs.length; });\
         mo.observe(target, { childList: true, subtree: true });\
         for (var i = 0; i < 3000; i++) {\
             var d = document.createElement('div');\
             d.setAttribute('data-i', String(i));\
             target.appendChild(d);\
         }\
         'done'",
    )
    .await;
    assert_eq!(out, "done");
    let n = eval_doc(&pool, "String(window.__n)").await;
    let count: usize = n.trim().parse().expect("numeric");
    assert!(
        count >= 3000,
        "records silently dropped: delivered {count} of 3000"
    );
}

#[tokio::test]
async fn mo_take_records_after_mass_mutation() {
    let pool = pool();
    let out = eval_doc(
        &pool,
        "window.__cb = 0;\
         var target = document.body;\
         var mo = new MutationObserver(function () { window.__cb += 1; });\
         mo.observe(target, { childList: true, subtree: true });\
         for (var i = 0; i < 1500; i++) {\
             target.appendChild(document.createElement('span'));\
         }\
         var taken = mo.takeRecords().length;\
         [String(taken), String(window.__cb)].join('|')",
    )
    .await;
    let (taken, cb): (usize, usize) = {
        let mut it = out.split('|');
        let t: usize = it.next().unwrap().trim().parse().unwrap();
        let c: usize = it.next().unwrap().trim().parse().unwrap();
        (t, c)
    };
    assert!(taken >= 1500, "takeRecords returned {taken} of 1500");
    assert_eq!(cb, 0, "callback fired before checkpoint");
}
