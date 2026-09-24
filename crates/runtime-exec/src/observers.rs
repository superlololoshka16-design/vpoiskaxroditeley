use crate::timer;
use crate::touch::{self, ApiKey};
use crate::webidl::{
    self, ObsOptions, RegCell, RegVec, chrome_exec_error, class_tag, define_method,
    install_host_ctor, node_of_value, param_not_type, this_id, view_node_type,
};
use crate::worker::viewport;
use compact_str::CompactString;
use rquickjs::function::Rest;
use rquickjs::function::This;
use rquickjs::{Ctx, Function, IntoJs, Object, Persistent, Value};
use smallvec::{SmallVec, smallvec};
use std::cell::RefCell;

const MO_CAP: usize = 256;
const RO_CAP: usize = 128;
const IO_CAP: usize = 128;
const OBS_TARGET_CAP: usize = 64;

struct MoState {
    cb: Persistent<Value<'static>>,
}

struct RoState {
    cb: Persistent<Value<'static>>,
    targets: SmallVec<[u32; 64]>,
    seen: SmallVec<[(u32, (f64, f64)); 64]>,
    scheduled: bool,
}

struct IoEntry {
    target: u32,
    rect: (f64, f64, f64, f64),
    root_rect: (f64, f64, f64, f64),
    ratio: f64,
    intersecting: bool,
    time: f64,
}

#[inline]
fn intersect_rect(b: (f64, f64, f64, f64), r: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let ix = b.0.max(r.0);
    let iy = b.1.max(r.1);
    let iw = ((b.0 + b.2).min(r.0 + r.2) - ix).max(0.0);
    let ih = ((b.1 + b.3).min(r.1 + r.3) - iy).max(0.0);
    (ix, iy, iw, ih)
}

struct IoState {
    cb: Persistent<Value<'static>>,
    targets: SmallVec<[u32; 64]>,
    root: Option<u32>,
    root_margin: [f64; 4],
    thresholds: SmallVec<[f64; 4]>,
    scheduled: bool,
    entries: Vec<IoEntry>,
}

thread_local! {
    static MO_REG: RegCell<MoState> = const { RefCell::new(RegVec::with_tag(3)) };
    static RO_REG: RegCell<RoState> = const { RefCell::new(RegVec::with_tag(4)) };
    static IO_REG: RegCell<IoState> = const { RefCell::new(RegVec::with_tag(5)) };
}

pub(crate) fn clear_registry() {
    MO_REG.with(|m| m.borrow_mut().clear());
    RO_REG.with(|m| m.borrow_mut().clear());
    IO_REG.with(|m| m.borrow_mut().clear());
}

use crate::webidl::with_reg;

fn with_mo<R>(id: u64, f: impl FnOnce(&mut MoState) -> R) -> Option<R> {
    with_reg(&MO_REG, id, f)
}

fn with_ro<R>(id: u64, f: impl FnOnce(&mut RoState) -> R) -> Option<R> {
    with_reg(&RO_REG, id, f)
}

fn with_io<R>(id: u64, f: impl FnOnce(&mut IoState) -> R) -> Option<R> {
    with_reg(&IO_REG, id, f)
}

fn reserve_target<S>(
    reg: &'static std::thread::LocalKey<RegCell<S>>,
    id: u64,
    node: u32,
    targets: fn(&mut S) -> &mut SmallVec<[u32; 64]>,
    scheduled: fn(&mut S) -> &mut bool,
) -> bool {
    reg.with(|m| {
        m.borrow_mut().with(id, |st| {
            let t = targets(st);
            if t.contains(&node) {
                return false;
            }
            if t.len() >= OBS_TARGET_CAP {
                return false;
            }
            t.push(node);
            let sch = scheduled(st);
            if *sch {
                return false;
            }
            *sch = true;
            true
        })
    })
    .unwrap_or(false)
}

fn observe_element<'js>(
    c: &Ctx<'js>,
    this: &This<Value<'js>>,
    target: Option<Value<'js>>,
    reg_key: usize,
    iface: &'static str,
    reserve: fn(u64, u32) -> bool,
    schedule: fn(&Ctx<'js>, u64) -> rquickjs::Result<()>,
) -> rquickjs::Result<()> {
    let target = target.ok_or_else(|| param_not_type(c, "observe", iface, 1, "Element"))?;
    let id = this_id(c, &this.0, reg_key)?;
    let Some(node) = node_of_value(&target) else {
        return Err(param_not_type(c, "observe", iface, 1, "Element"));
    };
    if view_node_type(node) != 1 {
        return Err(param_not_type(c, "observe", iface, 1, "Element"));
    }
    if reserve(id, node) {
        schedule(c, id)?;
    }
    Ok(())
}

fn unobserve_element<'js>(
    c: &Ctx<'js>,
    this: &This<Value<'js>>,
    target: Option<Value<'js>>,
    reg_key: usize,
    forget: fn(u64, u32),
) -> rquickjs::Result<()> {
    let target = target.unwrap_or_else(|| Value::new_undefined(c.clone()));
    let id = this_id(c, &this.0, reg_key)?;
    if let Some(node) = node_of_value(&target) {
        forget(id, node);
    }
    Ok(())
}

fn disconnect_observer<'js>(
    c: &Ctx<'js>,
    this: &This<Value<'js>>,
    reg_key: usize,
    reset: fn(u64),
) -> rquickjs::Result<()> {
    let id = this_id(c, &this.0, reg_key)?;
    reset(id);
    Ok(())
}

fn registry_ctor<'js, S: 'static>(
    c: &Ctx<'js>,
    cb: &Value<'js>,
    tag: &'static str,
    reg_key: usize,
    reg: &'static std::thread::LocalKey<RegCell<S>>,
    cap: usize,
    err: &'static str,
    api: u32,
    fresh: impl FnOnce(u64, Persistent<Value<'static>>) -> S,
) -> rquickjs::Result<Value<'js>> {
    require_ctor_cb(c, cb, tag)?;
    let saved = Persistent::save(c, cb.clone());
    webidl::registry_ctor(
        c,
        reg,
        cap,
        err,
        api,
        |id| fresh(id, saved),
        |cc, id| {
            let ctor: Function = cc.globals().get(tag)?;
            let proto: Object = ctor.get("prototype")?;
            webidl::registry_instance(cc, tag, reg_key, id, &proto)
        },
    )
}

fn mo_throw<'js>(c: &Ctx<'js>, detail: &str) -> rquickjs::Error {
    chrome_exec_error(c, "observe", "MutationObserver", detail)
}

fn bool_opt(opts: &Object<'_>, key: &str) -> rquickjs::Result<bool> {
    let v: Value = opts.get(key)?;
    if v.is_undefined() || v.is_null() {
        return Ok(false);
    }
    v.as_bool()
        .ok_or_else(|| mo_throw(opts.ctx(), "The provided value is not of type '(boolean or MutationObserverInit)'."))
}

fn filter_opt(ctx: &Ctx<'_>, opts: &Object<'_>) -> rquickjs::Result<SmallVec<[CompactString; 4]>> {
    let v: Value = opts.get("attributeFilter")?;
    if v.is_undefined() || v.is_null() {
        return Ok(SmallVec::new());
    }
    let arr = v
        .as_array()
        .ok_or_else(|| mo_throw(ctx, "member attributeFilter is not of type 'FrozenArray'."))?;
    let mut out = SmallVec::new();
    for item in arr.iter::<String>() {
        let s = item.map_err(|_| mo_throw(ctx, "member attributeFilter is not of type 'DOMString'."))?;
        out.push(CompactString::new(s));
    }
    Ok(out)
}

fn require_ctor_cb<'js>(c: &Ctx<'js>, cb: &Value<'js>, tag: &str) -> rquickjs::Result<()> {
    if !cb.is_function() {
        let mut msg = CompactString::with_capacity(64 + tag.len());
        msg.push_str("Failed to construct '");
        msg.push_str(tag);
        msg.push_str("': parameter 1 is not of type 'Function'.");
        return Err(rquickjs::Exception::throw_type(c, msg.as_str()));
    }
    Ok(())
}

fn install_mutation_observer<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let proto = Object::new(ctx.clone())?;
    class_tag(ctx, &proto, "MutationObserver")?;
    let real_ctor = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, cb: rquickjs::function::Opt<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::MUTATION_OBSERVER);
            let cb = cb.0.unwrap_or_else(|| Value::new_undefined(c.clone()));
            registry_ctor(
                &c,
                &cb,
                "MutationObserver",
                crate::webidl::REG_MO,
                &MO_REG,
                MO_CAP,
                "mutation observer registry saturated",
                ApiKey::MUTATION_OBSERVER,
                |_id, cb| MoState { cb },
            )
        },
    )?
    .with_constructor(true);
    install_host_ctor(ctx, &real_ctor, &proto, "MutationObserver", 1, true)?;

    let observe_f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         target: rquickjs::function::Opt<Value<'js>>,
         opts: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<()> {
            touch::touch_log_record(ApiKey::MUTATION_OBSERVER);
            let target = target
                .0
                .ok_or_else(|| param_not_type(&c, "observe", "MutationObserver", 1, "Node"))?;
            let opts = opts.0.unwrap_or_else(|| Value::new_undefined(c.clone()));
            let obs_id = this_id(&c, &this.0, crate::webidl::REG_MO)?;
            let Some(cb) = with_mo(obs_id, |s| s.cb.clone())
                .and_then(|p| p.restore(&c).ok())
                .filter(|v| v.is_function())
            else {
                return Ok(());
            };
            let Some(node) = node_of_value(&target) else {
                return Err(param_not_type(&c, "observe", "MutationObserver", 1, "Node"));
            };
            let opts_obj = if opts.is_undefined() || opts.is_null() {
                Object::new(c.clone())?
            } else {
                opts.as_object().cloned().ok_or_else(|| {
                    mo_throw(&c, "The provided value is not of type '(boolean or MutationObserverInit)'.")
                })?
            };
            let attrs = bool_opt(&opts_obj, "attributes")?;
            let char_data = bool_opt(&opts_obj, "characterData")?;
            let child_list = bool_opt(&opts_obj, "childList")?;
            if !child_list && !attrs && !char_data {
                return Err(mo_throw(
                    &c,
                    "The options object must set at least one of 'attributes', 'characterData', or 'childList' to true.",
                ));
            }
            let attr_old = bool_opt(&opts_obj, "attributeOldValue")?;
            let char_old = bool_opt(&opts_obj, "characterDataOldValue")?;
            let subtree = bool_opt(&opts_obj, "subtree")?;
            let filter = filter_opt(&c, &opts_obj)?;
            if attr_old && !attrs {
                return Err(mo_throw(
                    &c,
                    "The 'attributeOldValue' option requires 'attributes' or 'attributeFilter' to be true.",
                ));
            }
            if char_old && !char_data {
                return Err(mo_throw(
                    &c,
                    "The 'characterDataOldValue' option requires 'characterData' to be true.",
                ));
            }
            webidl::observe_reg(
                &c,
                obs_id,
                cb,
                node,
                ObsOptions::new(subtree, attrs, child_list, attr_old, char_data, char_old)
                    .with_filter(filter),
            );
            Ok(())
        },
    )?;
    define_method(ctx, &proto, "observe", observe_f)?;

    let disconnect_f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<()> {
            touch::touch_log_record(ApiKey::MUTATION_OBSERVER);
            let id = this_id(&c, &this.0, crate::webidl::REG_MO)?;
            webidl::disconnect_obs(id);
            MO_REG.with(|m| {
                m.borrow_mut().remove(id);
            });
            Ok(())
        },
    )?;
    define_method(ctx, &proto, "disconnect", disconnect_f)?;

    let take_f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::MUTATION_OBSERVER);
            let id = this_id(&c, &this.0, crate::webidl::REG_MO)?;
            let (recs, ids, attr_old, char_old) = webidl::take_records_for(id).unwrap_or_default();
            let tag = webidl::symbol_tostring_tag(&c).ok();
            let empty = webidl::empty_array_value(&c).ok();
            let arr = rquickjs::Array::new(c.clone())?;
            for (i, r) in recs.iter().enumerate() {
                let v = webidl::mutation_record_value(
                    &c,
                    r,
                    &ids,
                    empty.as_ref(),
                    tag.as_ref(),
                    attr_old,
                    char_old,
                )?;
                let _ = arr.set(i, v);
            }
            Ok(arr.into_value())
        },
    )?;
    define_method(ctx, &proto, "takeRecords", take_f)?;
    Ok(())
}

fn parse_root_margin(v: &Value<'_>) -> [f64; 4] {
    let sc = v.as_string().and_then(|s| s.clone().to_cstring().ok());
    let raw = sc.as_deref().unwrap_or("0px");
    let mut out = [0.0f64; 4];
    let mut idx = 0usize;
    let mut num = 0.0f64;
    let mut num_set = false;
    let b = raw.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        let ch = b[i];
        if ch.is_ascii_digit() || ch == b'.' || ch == b'-' || ch == b'+' {
            let start = i;
            while i < b.len()
                && (b[i].is_ascii_digit() || b[i] == b'.' || b[i] == b'-' || b[i] == b'+')
            {
                i += 1;
            }
            num = raw[start..i].parse().unwrap_or(0.0);
            num_set = true;
            continue;
        }
        if ch == b'%' {
            if num_set {
                out[idx % 4] = num * 0.01 * viewport().1;
                idx += 1;
                num_set = false;
            }
            i += 1;
            continue;
        }
        if ch == b',' || ch == b' ' {
            if num_set {
                out[idx % 4] = num;
                idx += 1;
                num_set = false;
            }
            i += 1;
            continue;
        }
        i += 1;
    }
    if num_set && idx < 4 {
        out[idx] = num;
    }
    out
}

fn install_resize_observer<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let proto = Object::new(ctx.clone())?;
    class_tag(ctx, &proto, "ResizeObserver")?;
    let real_ctor = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, cb: rquickjs::function::Opt<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::RESIZE_OBSERVER);
            let cb = cb.0.unwrap_or_else(|| Value::new_undefined(c.clone()));
            registry_ctor(
                &c,
                &cb,
                "ResizeObserver",
                crate::webidl::REG_RO,
                &RO_REG,
                RO_CAP,
                "resize observer registry saturated",
                ApiKey::RESIZE_OBSERVER,
                |_id, cb| RoState {
                    cb,
                    targets: SmallVec::new(),
                    seen: SmallVec::new(),
                    scheduled: false,
                },
            )
        },
    )?
    .with_constructor(true);
    install_host_ctor(ctx, &real_ctor, &proto, "ResizeObserver", 1, true)?;

    let observe_f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         target: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<()> {
            touch::touch_log_record(ApiKey::RESIZE_OBSERVER);
            observe_element(
                &c,
                &this,
                target.0,
                crate::webidl::REG_RO,
                "ResizeObserver",
                |id, node| {
                    reserve_target(&RO_REG, id, node, |s| &mut s.targets, |s| &mut s.scheduled)
                },
                |c, id| timer::defer_frame(c, move |cc| deliver_resize(cc, id)),
            )
        },
    )?;
    define_method(ctx, &proto, "observe", observe_f)?;

    let unobserve_f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         target: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<()> {
            touch::touch_log_record(ApiKey::RESIZE_OBSERVER);
            unobserve_element(&c, &this, target.0, crate::webidl::REG_RO, |id, node| {
                with_ro(id, |st| {
                    st.targets.retain(|t| *t != node);
                    st.seen.retain(|(n, _)| *n != node);
                });
            })
        },
    )?;
    define_method(ctx, &proto, "unobserve", unobserve_f)?;

    let disconnect_f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<()> {
            touch::touch_log_record(ApiKey::RESIZE_OBSERVER);
            disconnect_observer(&c, &this, crate::webidl::REG_RO, |id| {
                with_ro(id, |st| {
                    st.targets.clear();
                    st.seen.clear();
                    st.scheduled = false;
                });
            })
        },
    )?;
    define_method(ctx, &proto, "disconnect", disconnect_f)?;

    let take_f = Function::new(
        ctx.clone(),
        |_c: Ctx<'js>, _this: This<Value<'js>>| -> Vec<Value<'js>> { Vec::new() },
    )?;
    define_method(ctx, &proto, "takeRecords", take_f)?;
    Ok(())
}

fn ro_entry_value<'js>(
    ctx: &Ctx<'js>,
    target: u32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> rquickjs::Result<Object<'js>> {
    let o = Object::new(ctx.clone())?;
    class_tag(ctx, &o, "ResizeObserverEntry")?;
    let t = crate::worker::handle_value(ctx, target)?;
    o.set("target", t)?;
    let content = webidl::dom_rect(ctx, x, y, w, h, "DOMRectReadOnly")?;
    o.set("contentRect", content)?;
    let bbs = Object::new(ctx.clone())?;
    let size = Object::new(ctx.clone())?;
    size.set("inlineSize", w)?;
    size.set("blockSize", h)?;
    bbs.set("0", size.clone())?;
    bbs.set("length", 1u32)?;
    o.set("borderBoxSize", bbs.clone())?;
    o.set("devicePixelContentBoxSize", bbs)?;
    Ok(o)
}

fn deliver_resize(ctx: &Ctx<'_>, rid: u64) {
    type RoEntry = (u32, f64, f64, f64, f64);
    let Some(cb) = with_ro(rid, |s| s.cb.clone()) else { return };
    let targets: SmallVec<[u32; 64]> = with_ro(rid, |s| s.targets.clone()).unwrap_or_default();
    if targets.is_empty() {
        return;
    }
    let vw = viewport().0;
    let vh = viewport().1;
    let mut entries: Vec<RoEntry> = Vec::with_capacity(targets.len());
    with_ro(rid, |s| {
        s.scheduled = false;
        for &t in &targets {
            let r = crate::layout::rect_for(t);
            let x = r.x;
            let y = r.y;
            let width = r.w.max(0.0);
            let height = r.h.max(0.0);
            let mut changed = true;
            for (n, _) in s.seen.iter_mut() {
                if *n == t {
                    *n = t;
                    changed = false;
                    break;
                }
            }
            if changed {
                s.seen.push((t, (width, height)));
            }
            entries.push((t, x, y, width, height));
        }
    });
    let _ = webidl::call_cb_with_array(ctx, &cb, |arr| {
        for (i, e) in entries.iter().enumerate() {
            let v = ro_entry_value(ctx, e.0, e.1, e.2, e.3, e.4)?;
            let _ = arr.set(i, v);
        }
        Ok(())
    });
}

fn install_intersection_observer<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let proto = Object::new(ctx.clone())?;
    class_tag(ctx, &proto, "IntersectionObserver")?;
    let real_ctor = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, cb: rquickjs::function::Opt<Value<'js>>, rest: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::INTERSECTION_OBSERVER);
            let cb = cb.0.unwrap_or_else(|| Value::new_undefined(c.clone()));
            let mut root = None;
            let mut root_margin = [0.0f64; 4];
            let mut thresholds: SmallVec<[f64; 4]> = smallvec![0.0f64];
            if let Some(opts) = rest.first().and_then(|v| v.as_object().cloned()) {
                let root_v: Value = opts.get("root")?;
                if !root_v.is_null() && !root_v.is_undefined() {
                    let node = node_of_value(&root_v).ok_or_else(|| {
                        chrome_exec_error(
                            &c,
                            "IntersectionObserver",
                            "IntersectionObserver",
                            "member root is not of type 'Element'.",
                        )
                    })?;
                    root = Some(node);
                }
                let rm_v: Value = opts.get("rootMargin")?;
                if !rm_v.is_undefined() && !rm_v.is_null() {
                    root_margin = parse_root_margin(&rm_v);
                }
                let th_v: Value = opts.get("threshold")?;
                if !th_v.is_undefined() && !th_v.is_null() {
                    if let Some(arr) = th_v.as_array() {
                        thresholds.clear();
                        for item in arr.iter::<f64>() {
                            let t = item.unwrap_or(0.0);
                            if !(0.0..=1.0).contains(&t) {
                                return Err(rquickjs::Exception::throw_range(&c, "Failed to construct 'IntersectionObserver': Threshold values must be numbers between 0 and 1"));
                            }
                            thresholds.push(t);
                        }
                        if thresholds.is_empty() {
                            thresholds.push(0.0);
                        }
                    } else if let Some(t) = th_v.as_float() {
                        if !(0.0..=1.0).contains(&t) {
                            return Err(rquickjs::Exception::throw_range(&c, "Failed to construct 'IntersectionObserver': Threshold values must be numbers between 0 and 1"));
                        }
                        thresholds = smallvec![t];
                    }
                }
            }
            registry_ctor(
                &c,
                &cb,
                "IntersectionObserver",
                crate::webidl::REG_IO,
                &IO_REG,
                IO_CAP,
                "intersection observer registry saturated",
                ApiKey::INTERSECTION_OBSERVER,
                move |_id, cb| IoState {
                    cb,
                    targets: SmallVec::new(),
                    root,
                    root_margin,
                    thresholds,
                    scheduled: false,
                    entries: Vec::new(),
                },
            )
        },
    )?
    .with_constructor(true);
    install_host_ctor(ctx, &real_ctor, &proto, "IntersectionObserver", 1, true)?;

    let observe_f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         target: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<()> {
            touch::touch_log_record(ApiKey::INTERSECTION_OBSERVER);
            observe_element(
                &c,
                &this,
                target.0,
                crate::webidl::REG_IO,
                "IntersectionObserver",
                |id, node| {
                    reserve_target(&IO_REG, id, node, |s| &mut s.targets, |s| &mut s.scheduled)
                },
                |c, id| timer::defer_frame(c, move |cc| deliver_intersection(cc, id)),
            )
        },
    )?;
    define_method(ctx, &proto, "observe", observe_f)?;

    let unobserve_f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         target: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<()> {
            touch::touch_log_record(ApiKey::INTERSECTION_OBSERVER);
            unobserve_element(&c, &this, target.0, crate::webidl::REG_IO, |id, node| {
                with_io(id, |st| {
                    st.targets.retain(|t| *t != node);
                    st.entries.retain(|e| e.target != node);
                });
            })
        },
    )?;
    define_method(ctx, &proto, "unobserve", unobserve_f)?;

    let disconnect_f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<()> {
            touch::touch_log_record(ApiKey::INTERSECTION_OBSERVER);
            disconnect_observer(&c, &this, crate::webidl::REG_IO, |id| {
                with_io(id, |st| {
                    st.targets.clear();
                    st.entries.clear();
                    st.scheduled = false;
                });
            })
        },
    )?;
    define_method(ctx, &proto, "disconnect", disconnect_f)?;

    let take_f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::INTERSECTION_OBSERVER);
            let id = this_id(&c, &this.0, crate::webidl::REG_IO)?;
            let entries = with_io(id, |st| std::mem::take(&mut st.entries)).unwrap_or_default();
            let arr = rquickjs::Array::new(c.clone())?;
            for (i, e) in entries.iter().enumerate() {
                let v = io_entry_value(&c, e)?;
                let _ = arr.set(i, v);
            }
            Ok(arr.into_value())
        },
    )?;
    define_method(ctx, &proto, "takeRecords", take_f)?;

    let root_get = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let id = this_id(&c, &this.0, crate::webidl::REG_IO)?;
            let root = with_io(id, |st| st.root).flatten();
            match root {
                Some(n) => crate::worker::handle_value(&c, n),
                None => Ok(Value::new_null(c)),
            }
        },
    )?;
    crate::webidl::named_accessor(ctx, &proto, "root", root_get, None)?;

    let root_margin_get = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let id = this_id(&c, &this.0, crate::webidl::REG_IO)?;
            let rm = with_io(id, |st| st.root_margin).unwrap_or([0.0; 4]);
            let mut s = CompactString::with_capacity(40);
            for side in rm {
                core_utils::push_px_into(&mut s, side);
                s.push_str("px ");
            }
            s.truncate(s.len() - 1);
            s.as_str().into_js(&c)
        },
    )?;
    crate::webidl::named_accessor(ctx, &proto, "rootMargin", root_margin_get, None)?;

    let thresholds_get = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let id = this_id(&c, &this.0, crate::webidl::REG_IO)?;
            let th = with_io(id, |st| st.thresholds.clone()).unwrap_or_else(|| smallvec![0.0]);
            let arr = rquickjs::Array::new(c.clone())?;
            for (i, t) in th.iter().enumerate() {
                arr.set(i, *t)?;
            }
            Ok(arr.into_value())
        },
    )?;

    crate::webidl::named_accessor(ctx, &proto, "thresholds", thresholds_get, None)?;
    Ok(())
}

fn io_entry_value<'js>(ctx: &Ctx<'js>, e: &IoEntry) -> rquickjs::Result<Value<'js>> {
    let o = Object::new(ctx.clone())?;
    let target = crate::worker::handle_value(ctx, e.target)?;
    o.set("target", target)?;
    let b = e.rect;
    let bcr = crate::webidl::dom_rect(ctx, b.0, b.1, b.2, b.3, "DOMRectReadOnly")?;
    o.set("boundingClientRect", bcr)?;
    let r = e.root_rect;
    let rb = crate::webidl::dom_rect(ctx, r.0, r.1, r.2, r.3, "DOMRectReadOnly")?;
    o.set("rootBounds", rb)?;
    let (ix, iy, iw, ih) = intersect_rect(b, r);
    let ir = crate::webidl::dom_rect(ctx, ix, iy, iw, ih, "DOMRectReadOnly")?;
    o.set("intersectionRect", ir)?;
    o.set("intersectionRatio", e.ratio)?;
    o.set("isIntersecting", e.intersecting)?;
    o.set("time", e.time)?;
    Ok(o.into_value())
}

fn deliver_intersection(ctx: &Ctx<'_>, iid: u64) {
    let Some(cb) = with_io(iid, |s| s.cb.clone()) else { return };
    let (targets, root, root_margin, thresholds) = with_io(iid, |s| {
        (
            s.targets.clone(),
            s.root,
            s.root_margin,
            s.thresholds.clone(),
        )
    })
    .unwrap_or((SmallVec::new(), None, [0.0; 4], smallvec![0.0]));
    with_io(iid, |s| s.scheduled = false);
    if targets.is_empty() {
        return;
    }
    let (vw, vh) = viewport();
    let root_rect = match root {
        Some(n) => {
            let r = crate::layout::rect_for(n);
            (r.x, r.y, r.w, r.h)
        }
        None => (0.0, 0.0, vw, vh),
    };
    let root_rect = (
        root_rect.0 - root_margin[3],
        root_rect.1 - root_margin[0],
        root_rect.2 + root_margin[1] + root_margin[3],
        root_rect.3 + root_margin[0] + root_margin[2],
    );
    let now = crate::timer::clock::now_ms_quantized();
    let mut entries: Vec<IoEntry> = Vec::with_capacity(targets.len());
    for &t in &targets {
        let r = crate::layout::rect_for(t);
        let rect = (r.x, r.y, r.w, r.h);
        let (ix, iy, iw, ih) = intersect_rect(rect, root_rect);
        let area = (rect.2 * rect.3).max(1e-9);
        let ratio = (iw * ih / area).clamp(0.0, 1.0);
        let intersecting = iw > 0.0 && ih > 0.0;
        let last_ratio = 0.0f64;
        let thresh = thresholds
            .iter()
            .copied()
            .chain([0.0f64])
            .find(|&th| {
                (ratio > th && last_ratio <= th) || (ratio <= th && last_ratio > th) || th == 0.0
            })
            .unwrap_or(0.0);
        let _ = thresh;
        entries.push(IoEntry {
            target: t,
            rect,
            root_rect,
            ratio,
            intersecting,
            time: now,
        });
    }
    let _ = webidl::call_cb_with_array(ctx, &cb, |arr| {
        for (i, e) in entries.iter().enumerate() {
            let v = io_entry_value(ctx, e)?;
            arr.set(i, v)?;
        }
        Ok(())
    });
}

pub(crate) fn install<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    install_mutation_observer(ctx)?;
    install_resize_observer(ctx)?;
    install_intersection_observer(ctx)?;
    Ok(())
}
