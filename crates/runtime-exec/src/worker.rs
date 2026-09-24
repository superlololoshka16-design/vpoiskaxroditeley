pub(crate) mod dispatch {
    use rquickjs::function::Function;
    use rquickjs::{Context, Ctx, Persistent, Value};
    use std::sync::Arc;

    pub(crate) const READY_LOADING: u8 = 0;
    pub(crate) const READY_INTERACTIVE: u8 = 1;
    pub(crate) const READY_COMPLETE: u8 = 2;

    thread_local! {
        static READY: std::cell::Cell<u8> = const { std::cell::Cell::new(READY_LOADING) };
    }

    pub(crate) fn ready_state() -> u8 {
        READY.with(std::cell::Cell::get)
    }

    pub(crate) fn set_ready_state(v: u8) {
        READY.with(|r| r.set(v));
    }

    pub(crate) struct Dispatch {
        feed: Persistent<Function<'static>>,
        reset: Persistent<Function<'static>>,
        domready: Option<Persistent<Function<'static>>>,
        pageload: Option<Persistent<Function<'static>>>,
    }

    impl Dispatch {
        pub(crate) fn capture<'js>(ctx: &Ctx<'js>) -> Option<Self> {
            let feed: Function<'js> = ctx.globals().get("__silo_feed").ok()?;
            let reset: Function<'js> = ctx.globals().get("__silo_reset").ok()?;
            let domready: Option<Function<'js>> = ctx.globals().get("__silo_domready").ok();
            let pageload: Option<Function<'js>> = ctx.globals().get("__silo_pageload").ok();
            let feed = Persistent::save(ctx, feed);
            let reset = Persistent::save(ctx, reset);
            let domready = domready.map(|f| Persistent::save(ctx, f));
            let pageload = pageload.map(|f| Persistent::save(ctx, f));
            Some(Self {
                feed,
                reset,
                domready,
                pageload,
            })
        }

        pub(crate) fn feed(
            &self,
            context: &Context,
            input: Option<&Arc<[payload_gen::input::RawEvent]>>,
        ) {
            context.with(|ctx| {
                let Ok(reset) = self.reset.clone().restore(&ctx) else {
                    return;
                };
                let cleared: Result<Value, _> = reset.call(());
                if cleared.is_err() {
                    let _ = ctx.catch();
                }
                let Some(events) = input else {
                    return;
                };
                if events.is_empty() {
                    return;
                }
                let Ok(feed) = self.feed.clone().restore(&ctx) else {
                    return;
                };
                let bytes = payload_gen::input::events_bytes(events);
                let Ok(buf) = rquickjs::ArrayBuffer::new_copy(ctx.clone(), bytes) else {
                    return;
                };
                let out: Result<Value, _> = feed.call((buf,));
                if out.is_err() {
                    let _ = ctx.catch();
                }
            })
        }

        pub(crate) fn fire_domready_ctx(&self, ctx: &Ctx<'_>) {
            set_ready_state(READY_INTERACTIVE);
            fire_lifecycle(&self.domready, ctx);
        }

        pub(crate) fn fire_pageload_ctx(&self, ctx: &Ctx<'_>) {
            set_ready_state(READY_COMPLETE);
            fire_lifecycle(&self.pageload, ctx);
        }
    }

    fn fire_lifecycle(slot: &Option<Persistent<Function<'static>>>, ctx: &Ctx<'_>) {
        let Some(f) = slot else { return };
        let Ok(f) = f.clone().restore(ctx) else {
            return;
        };
        let out: Result<Value, _> = f.call(());
        if out.is_err() {
            let _ = ctx.catch();
        }
    }
}

use crate::cache::NormCache;
use crate::canvas2d;
use crate::normalize::{Lit, normalize};
use crate::query;
use crate::stackfmt;
use crate::task::{
    DEFAULT_RTT_MS, ExecControl, ExecError, ExecKind, ExecOutcome, ExecPath, ExecReq, ProfileSnap,
};
use crate::task::{Event, EventTx};
use crate::timer;
use crate::timer::clock;
use crate::touch::{self, ApiKey};
use crate::wasm::{is_wasm_magic, run_wasm};
use crate::worker::dispatch::Dispatch;
use crate::{deviceapi, intl, netapi, observers, pageapi};
use bytes::Bytes;
use challenge_solver::anubis as anubis_solver;
use compact_str::CompactString;
use core_utils::{
    FxBuild, HrefParts, fx_map, md5_hex_into, pin_thread, sha256_hex_into, split_href,
};
use payload_gen::input::RawEvent;
use rquickjs::class::Trace;
use rquickjs::function::{Func, Function, Rest};
use rquickjs::object::Property;
use rquickjs::{Class, Context, Ctx, IntoJs, JsLifetime, Object, Persistent, Runtime, Type, Value};
use session_state::Profile;
use smallvec::SmallVec;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use thiserror::Error;

fn mem_limit() -> usize {
    with_prof(|p| p.mem_limit)
}

fn gc_threshold() -> usize {
    (mem_limit() / 4).max(256 * 1024)
}
const STACK_LIMIT: usize = 1024 * 1024;
const FN_CACHE_CAP: usize = 256;
const RELOAD_MARK: &str = crate::task::NAV_RELOAD;

pub(crate) struct ProfState {
    prof: Arc<Profile>,
    pub(crate) href: CompactString,
    pub(crate) cookie: CompactString,
    pub(crate) seed: u64,
    pub(crate) raster_seed: u64,
    pub(crate) mem_limit: usize,
    pub(crate) rtt_ms: u32,
    pub(crate) mobile: bool,
    pub(crate) firefox: bool,
    pub(crate) chromium: bool,
    pub(crate) windows: bool,
    pub(crate) display_hz: u32,
    pub(crate) app_version: CompactString,
    locale_base: CompactString,
    pub(crate) origin: CompactString,
    pub(crate) host: CompactString,
    path: CompactString,
    search: CompactString,
    hash: CompactString,
    pub(crate) tz_zone: Option<core_utils::tz::TzIdx>,
}

impl ProfState {
    #[inline(always)]
    pub(crate) fn prof(&self) -> &Profile {
        &self.prof
    }
}

fn empty_profile() -> Arc<Profile> {
    static EMPTY: LazyLock<Arc<Profile>> = LazyLock::new(|| Arc::new(Profile::shell()));
    EMPTY.clone()
}

thread_local! {
    pub(crate) static PROF: RefCell<Rc<ProfState>> = RefCell::new(Rc::new(empty_prof_state()));
    static PROF_PTR: Cell<*const ProfState> = const { Cell::new(std::ptr::null()) };
}

#[inline(always)]
pub(crate) fn prof_ptr() -> *const ProfState {
    let p = PROF_PTR.with(|c| c.get());
    if p.is_null() {
        PROF.with(|c| {
            let a = c.borrow();
            let p = Rc::as_ptr(&a);
            PROF_PTR.with(|c| c.set(p));
            p
        })
    } else {
        p
    }
}

thread_local! {
    static DOC: RefCell<Option<Arc<parser_pipeline::PageData>>> = const { RefCell::new(None) };
    static CONTROL: Cell<*const ExecControl> = const { Cell::new(std::ptr::null()) };
    static CANCELLED: Cell<*const std::sync::atomic::AtomicBool> =
        const { Cell::new(std::ptr::null()) };
    static DEADLINE: Cell<Instant> = Cell::new(Instant::now());
    pub(crate) static DOC_GENERATION: Cell<u64> = const { Cell::new(0) };
}

#[inline(always)]
fn control_ptr() -> Option<*const ExecControl> {
    let p = CONTROL.with(|c| c.get());
    (p as usize != 0).then_some(p)
}

struct RequestScope;

fn clear_scope_state() {
    store_doc(None);
    canvas2d::clear_registry();
    netapi::clear_registry();
    observers::clear_registry();
    intl::clear_registry();
    deviceapi::clear_registry();
    collections_clear();
    current_script_clear();
}

fn clear_request_state() {
    clear_scope_state();
    timer::clear_all();
    cookie_out_clear();
    nav_clear();
}

impl RequestScope {
    fn enter(control: Arc<ExecControl>) -> Self {
        clear_scope_state();
        crate::layout::clear();
        crate::webidl::reset_request();
        NET_SLOT.with(|s| s.set(0));
        DOC_GENERATION.with(|generation| generation.set(generation.get().wrapping_add(1)));
        CONTROL.with(|active| active.set(Arc::as_ptr(&control)));
        let ctrl = &*control;
        CANCELLED.with(|c| c.set(control_cancelled_ptr(ctrl)));
        DEADLINE.with(|d| d.set(ctrl.deadline));
        Self
    }
}

#[inline(always)]
fn control_cancelled_ptr(c: &ExecControl) -> *const std::sync::atomic::AtomicBool {
    c.cancelled_ptr()
}

impl Drop for RequestScope {
    fn drop(&mut self) {
        clear_request_state();
        CONTROL.with(|active| active.set(std::ptr::null()));
    }
}

thread_local! {
    pub(crate) static FAST_RNG: Cell<core_utils::rng::Rng> = Cell::new(core_utils::rng::Rng::new(core_utils::rng::GOLDEN));
    static COOKIE_OUT: RefCell<session_state::CookieJar> = RefCell::new(session_state::CookieJar::new());
    static NAV_OUT: RefCell<Option<CompactString>> = const { RefCell::new(None) };
    static INT_TICK: Cell<u64> = const { Cell::new(0) };
    static NET_SLOT: Cell<usize> = const { Cell::new(0) };
    static CURRENT_SCRIPT: Cell<Option<u32>> = const { Cell::new(None) };
    static THUNKS: RefCell<Thunks> = const { RefCell::new(Thunks::new()) };
}

type FnSlot = Option<Persistent<Function<'static>>>;

struct Thunks {
    promise_resolve: FnSlot,
    promise_reject: FnSlot,
    promise_reject_val: FnSlot,
    json_promise: FnSlot,
    fetch_stub: FnSlot,
    fetch_parse: FnSlot,
}

impl Thunks {
    const fn new() -> Self {
        Self {
            promise_resolve: None,
            promise_reject: None,
            promise_reject_val: None,
            json_promise: None,
            fetch_stub: None,
            fetch_parse: None,
        }
    }
}

pub(crate) fn current_script_set(node: Option<u32>) {
    CURRENT_SCRIPT.with(|c| c.set(node));
}

fn current_script_clear() {
    CURRENT_SCRIPT.with(|c| c.set(None));
}

pub(crate) fn net_slot() -> usize {
    NET_SLOT.with(Cell::get)
}

pub(crate) fn control_deadline() -> Instant {
    with_control_deadline()
}

#[inline]
pub(crate) fn viewport() -> (f64, f64) {
    with_prof(|p| p.prof().viewport())
}

#[inline]
pub(crate) fn with_doc<R>(f: impl FnOnce(Option<&parser_pipeline::PageData>) -> R) -> R {
    DOC.with(|c| f(c.borrow().as_deref()))
}

#[inline]
pub(crate) fn prof_seed() -> u64 {
    with_prof(|p| p.seed)
}

#[inline]
pub(crate) fn prof_raster_seed() -> u64 {
    with_prof(|p| p.raster_seed)
}

#[inline]
pub(crate) fn prof_cpu_scale() -> f64 {
    with_prof(|p| p.prof().cpu_scale().max(0.1))
}

#[inline]
pub(crate) fn prof_origin() -> CompactString {
    with_prof(|p| p.origin.clone())
}

#[inline]
pub(crate) fn prof_host() -> CompactString {
    with_prof(|p| p.host.clone())
}

fn store_doc(doc: Option<Arc<parser_pipeline::PageData>>) {
    DOC.with(|c| *c.borrow_mut() = doc);
}


type ColSlot = Option<(Persistent<Object<'static>>, u64)>;

thread_local! {
    static COLS: RefCell<[ColSlot; 5]> = const { RefCell::new([None, None, None, None, None]) };
    static CONN_OBJ: RefCell<Option<Persistent<Object<'static>>>> = const { RefCell::new(None) };
    static PROF_OBJ: RefCell<Option<Persistent<Object<'static>>>> = const { RefCell::new(None) };
    static MEM_OBJ: RefCell<Option<(u64, Persistent<Object<'static>>)>> = const { RefCell::new(None) };
}

fn collections_clear() {
    COLS.with(|c| *c.borrow_mut() = [None, None, None, None, None]);
}

pub(crate) fn cookie_out_clear() {
    COOKIE_OUT.with(|c| c.borrow_mut().clear());
}

pub(crate) fn cookie_set(line: &str) {
    COOKIE_OUT.with(|c| c.borrow_mut().ingest(line));
}

fn cookie_out_take() -> Option<CompactString> {
    COOKIE_OUT.with(|c| {
        let jar = c.borrow_mut();
        jar.header_str()
    })
}

fn nav_set(url: CompactString) {
    NAV_OUT.with(|n| *n.borrow_mut() = Some(url));
}

fn nav_clear() {
    NAV_OUT.with(|n| *n.borrow_mut() = None);
}

fn nav_take() -> Option<CompactString> {
    NAV_OUT.with(|n| n.borrow_mut().take())
}

#[inline]
pub(crate) fn handle_value<'js>(ctx: &Ctx<'js>, node: u32) -> rquickjs::Result<Value<'js>> {
    handle_value_with_iface(ctx, node, crate::webidl::iface_of_node(node))
}

pub(crate) fn handle_value_with_iface<'js>(
    ctx: &Ctx<'js>,
    node: u32,
    iface: crate::webidl::InterfaceId,
) -> rquickjs::Result<Value<'js>> {
    let (external, cached) = crate::webidl::lookup_handle_raw(node);
    if let Some(v) = crate::webidl::restore_persistent(ctx, external) {
        return Ok(v);
    }
    if let Some(obj) = crate::webidl::restore_persistent(ctx, cached) {
        return Ok(obj.into_value());
    }
    let generation = DOC_GENERATION.with(Cell::get);
    let class: Class<NodeHandle> = Class::instance(ctx.clone(), NodeHandle { node, generation })?;
    let obj: Object = class.into_inner();
    let proto = crate::webidl::prototype(ctx, iface)?;
    obj.set_prototype(Some(&proto))?;
    crate::webidl::store_handle(ctx, node, obj.clone());
    Ok(obj.into_value())
}

fn empty_prof_state() -> ProfState {
    ProfState {
        prof: empty_profile(),
        href: CompactString::new(""),
        cookie: CompactString::new(""),
        seed: 0,
        raster_seed: 0,
        rtt_ms: DEFAULT_RTT_MS,
        mobile: false,
        firefox: false,
        chromium: false,
        windows: true,
        display_hz: 60,
        app_version: CompactString::new(""),
        locale_base: CompactString::new(""),
        origin: CompactString::new(""),
        host: CompactString::new(""),
        path: CompactString::new(""),
        search: CompactString::new(""),
        hash: CompactString::new(""),
        tz_zone: None,
        mem_limit: 8 * 1024 * 1024,
    }
}

pub(crate) fn with_prof<R>(f: impl FnOnce(&ProfState) -> R) -> R {
    f(unsafe { &*prof_ptr() })
}

fn store_prof(snap: &ProfileSnap) {
    let profile = &snap.prof;
    let ua = profile.ua.as_ref();
    let app_version = CompactString::new(ua.strip_prefix("Mozilla/").unwrap_or(ua));
    let locale_base = CompactString::new(profile.locale.split('-').next().unwrap_or(""));
    let parts = split_href(&snap.href);
    let HrefParts {
        origin,
        host,
        path,
        search,
        hash,
    } = parts;
    PROF_PTR.with(|c| c.set(std::ptr::null()));
    PROF_OBJ.with(|c| *c.borrow_mut() = None);
    PROF.with(|p| {
        *p.borrow_mut() = Rc::new(ProfState {
            prof: Arc::clone(profile),
            href: snap.href.clone(),
            cookie: snap.cookie.clone(),
            seed: snap.seed(),
            raster_seed: core_utils::profile::raster_seed(
                profile.canvas_seed,
                profile.webgl_vendor().as_bytes(),
                profile.webgl_renderer().as_bytes(),
            ),
            mem_limit: profile.device_memory().clamp(2, 8) as usize * 1024 * 1024,
            rtt_ms: snap.rtt_ms,
            mobile: profile.platform.is_mobile(),
            firefox: matches!(profile.family, session_state::Family::Firefox { .. }),
            chromium: matches!(
                profile.family,
                session_state::Family::Chrome { .. } | session_state::Family::Edge { .. }
            ),
            windows: profile.platform == session_state::Platform::Windows,
            display_hz: profile.emit_hz(),
            app_version,
            locale_base,
            origin,
            host,
            path,
            search,
            hash,
            tz_zone: core_utils::tz::zone_of(profile.tz.as_str()),
        });
    });
}

#[derive(Trace, JsLifetime, Clone)]
#[rquickjs::class(rename = "Node", rename_all = "camelCase")]
pub(crate) struct NodeHandle {
    pub(crate) node: u32,
    generation: u64,
}

fn host_prototype<'js>(
    ctx: &Ctx<'js>,
    obj: &Object<'js>,
    ctor_name: &'static str,
) -> rquickjs::Result<Object<'js>> {
    let proto = Object::new(ctx.clone())?;
    let ctor = crate::webidl::illegal_ctor_fn(ctx, ctor_name)?;
    crate::webidl::install_host_ctor(ctx, &ctor, &proto, ctor_name, 0, true)?;
    obj.set_prototype(Some(&proto))?;
    Ok(proto)
}

fn add_get_element_by_id<'js>(ctx: &Ctx<'js>, proto: &Object<'js>) -> rquickjs::Result<()> {
    let gebi = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, id: rquickjs::String<'js>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::GET_ELEMENT_BY_ID);
            let cs = id.to_cstring()?;
            crate::webidl::opt_node_or_null(&c, crate::webidl::find_by_id_view(cs.as_str()))
        },
    )?;
    crate::webidl::define_method(ctx, proto, "getElementById", gebi)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColKind {
    Scripts,
    Forms,
    Images,
    Links,
    Embeds,
}

fn col_fetch(kind: ColKind) -> SmallVec<[u32; 32]> {
    let mut out: SmallVec<[u32; 32]> = SmallVec::new();
    let tag = match kind {
        ColKind::Scripts => "script",
        ColKind::Forms => "form",
        ColKind::Images => "img",
        ColKind::Links => "a",
        ColKind::Embeds => "embed",
    };
    match kind {
        ColKind::Scripts => with_doc(|d| {
            if let Some(p) = d {
                out.extend(p.dom.scripts.iter().copied());
            }
        }),
        ColKind::Forms => with_doc(|d| {
            if let Some(p) = d {
                out.extend(p.dom.forms.iter().copied());
            }
        }),
        _ => query::collect_by_tag_view_into(tag, &mut out),
    }
    if matches!(kind, ColKind::Scripts | ColKind::Forms) {
        crate::webidl::overlay_elements_into(tag, &mut out);
    }
    out
}

fn build_node_collection<'js>(ctx: &Ctx<'js>, kind: ColKind) -> rquickjs::Result<Object<'js>> {
    let col = Object::new(ctx.clone())?;
    let ids = col_fetch(kind);
    let len = ids.len() as u32;
    let g_len = Function::new(ctx.clone(), move || -> u32 { len })?;
    crate::webidl::named_accessor(ctx, &col, "length", g_len, None)?;
    let item = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, i: u32| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::SCRIPTS);
            match ids.get(i as usize) {
                Some(&n) => Ok(crate::worker::handle_value(&c, n)?),
                None => Ok(Value::new_null(c)),
            }
        },
    )?;
    crate::webidl::define_method(ctx, &col, "item", item)?;
    Ok(col)
}

fn collection_accessor<'js>(
    proto: &Object<'js>,
    key: &'static str,
    kind: ColKind,
) -> rquickjs::Result<()> {
    let api = match kind {
        ColKind::Forms => ApiKey::FORMS,
        ColKind::Images => ApiKey::IMAGES,
        ColKind::Links => ApiKey::LINKS,
        ColKind::Embeds => ApiKey::EMBEDS,
        _ => ApiKey::SCRIPTS,
    };
    let ctx = proto.ctx();
    let g = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(api);
            let dgen = crate::webidl::view_epoch(DOC_GENERATION.with(Cell::get));
            let cached = COLS.with(|b| b.borrow()[kind as usize].clone());
            if let Some((p, g)) = cached
                && g == dgen
                && let Ok(v) = p.restore(&c)
            {
                return Ok(v.into_value());
            }
            let col = build_node_collection(&c, kind)?;
            COLS.with(|b| {
                b.borrow_mut()[kind as usize] = Some((Persistent::save(&c, col.clone()), dgen));
            });
            Ok(col.into_value())
        },
    )?;
    crate::webidl::named_accessor(ctx, proto, key, g, None)
}

fn add_prop_acc<'js, const STACK: bool, F>(
    ctx: &Ctx<'js>,
    target: &Object<'js>,
    key: &str,
    api: u32,
    f: F,
) -> rquickjs::Result<()>
where
    F: Fn(&Ctx<'js>, &ProfState) -> rquickjs::Result<Value<'js>> + 'js,
{
    let g = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            if STACK {
                touch::touch_log_record_stack(api, &c);
            } else {
                touch::touch_log_record(api);
            }
            with_prof(|p| f(&c, p))
        },
    )?;
    crate::webidl::named_accessor(ctx, target, key, g, None)
}

fn add_str_acc<'js, F>(obj: &Object<'js>, key: &str, api: u32, f: F) -> rquickjs::Result<()>
where
    F: Fn(&ProfState) -> &str + 'js,
{
    let ctx = obj.ctx();
    add_prop_acc::<true, _>(&ctx, obj, key, api, move |c, p| f(p).into_js(c))
}

fn add_acc<'js, R, F>(obj: &Object<'js>, key: &str, api: u32, f: F) -> rquickjs::Result<()>
where
    R: IntoJs<'js> + 'js,
    F: Fn(&ProfState) -> R + 'js,
{
    let ctx = obj.ctx();
    add_prop_acc::<true, _>(&ctx, obj, key, api, move |c, p| f(p).into_js(c))
}

fn add_acc_null<'js>(obj: &Object<'js>, key: &str, api: u32) -> rquickjs::Result<()> {
    let ctx = obj.ctx();
    add_prop_acc::<true, _>(&ctx, obj, key, api, move |c: &Ctx<'js>, _| {
        Ok(Value::new_null(c.clone()))
    })
}

fn add_win_acc<'js, F>(ctx: &Ctx<'js>, key: &'static str, api: u32, f: F) -> rquickjs::Result<()>
where
    F: Fn(&ProfState) -> f64 + 'js,
{
    add_prop_acc::<false, _>(ctx, &ctx.globals(), key, api, move |c, p| {
        f(p).into_js(c)
    })
}

fn add_conn_acc<'js, R, F>(
    ctx: &Ctx<'js>,
    obj: &Object<'js>,
    key: &str,
    f: F,
) -> rquickjs::Result<()>
where
    R: IntoJs<'js> + 'js,
    F: Fn(&ProfState) -> R + 'js,
{
    add_prop_acc::<false, _>(ctx, obj, key, ApiKey::CONNECTION, move |c, p| {
        f(p).into_js(c)
    })
}

fn add_lang_acc<'js>(
    obj: &Object<'js>,
    key: &str,
    api: u32,
    f: impl Fn(&Ctx<'js>) -> rquickjs::Result<Value<'js>> + 'js,
) -> rquickjs::Result<()> {
    let ctx = obj.ctx();
    add_prop_acc::<true, _>(&ctx, obj, key, api, move |c, _| f(c))
}

fn add_languages<'js>(obj: &Object<'js>) -> rquickjs::Result<()> {
    add_lang_acc(obj, "languages", ApiKey::LANGUAGES, |c| {
        let arr = rquickjs::Array::new(c.clone())?;
        with_prof(|p| -> rquickjs::Result<()> {
            arr.set(0, p.prof().locale.as_str())?;
            arr.set(1, p.locale_base.as_str())?;
            Ok(())
        })?;
        Ok(arr.into_value())
    })
}

fn build_conn_obj<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let conn = Object::new(ctx.clone())?;
    add_conn_acc(ctx, &conn, "rtt", |p| p.rtt_ms as f64)?;
    add_conn_acc(ctx, &conn, "downlink", |p| {
        ((p.rtt_ms as f64) / DEFAULT_RTT_MS as f64 * 10.0).clamp(1.0, 10.0)
    })?;
    add_conn_acc(ctx, &conn, "effectiveType", |p| {
        if p.rtt_ms > 150 { "3g" } else { "4g" }
    })?;
    add_conn_acc(ctx, &conn, "saveData", |_| false)?;
    host_prototype(ctx, &conn, "NetworkInformation")?;
    Ok(conn)
}

fn add_conn_singleton<'js>(proto: &Object<'js>) -> rquickjs::Result<()> {
    let ctx = proto.ctx();
    let g = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::CONNECTION);
            if let Some(v) = crate::webidl::restore_slot(&c, &CONN_OBJ) {
                return Ok(v.into_value());
            }
            let conn = build_conn_obj(&c)?;
            CONN_OBJ.with(|b| *b.borrow_mut() = Some(Persistent::save(&c, conn.clone())));
            Ok(conn.into_value())
        },
    )?;
    crate::webidl::named_accessor(ctx, proto, "connection", g, None)
}

fn add_method_empty<'js>(obj: &Object<'js>, key: &str, api: u32) -> rquickjs::Result<()> {
    obj.prop(
        key,
        Property::from(Func::from(move || -> Vec<Value<'js>> {
            touch::touch_log_record(api);
            Vec::new()
        }))
        .writable()
        .configurable(),
    )
}

fn add_query_selector<'js>(ctx: &Ctx<'js>, proto: &Object<'js>) -> rquickjs::Result<()> {
    let qs = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, q: String| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::QUERY_SELECTOR);
            crate::webidl::opt_node_or_null(&c, query::select_first(q.as_str()))
        },
    )?;
    crate::webidl::define_method(ctx, proto, "querySelector", qs)?;
    let qsa = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, q: String| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::QUERY_SELECTOR_ALL);
            let arr = rquickjs::Array::new(c.clone())?;
            if let Some(list) = query::select_all(q.as_str()) {
                for (i, n) in list.iter().enumerate() {
                    let v = handle_value(&c, *n)?;
                    arr.set(i, v)?;
                }
            }
            Ok(arr.into_value())
        },
    )?;
    crate::webidl::define_method(ctx, proto, "querySelectorAll", qsa)
}

fn add_doc_geometry<'js>(ctx: &Ctx<'js>, proto: &Object<'js>) -> rquickjs::Result<()> {
    let g_de = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::BODY);
            crate::webidl::opt_node_or_null(&c, query::find_first_by_tag_view("html"))
        },
    )?;
    crate::webidl::named_accessor(ctx, proto, "documentElement", g_de, None)?;
    let g_cw = Function::new(ctx.clone(), move || {
        touch::touch_log_record(ApiKey::INNER_WIDTH);
        with_prof(|p| p.prof().screen_css().0)
    })?;
    crate::webidl::named_accessor(ctx, proto, "clientWidth", g_cw, None)?;
    let g_ch = Function::new(ctx.clone(), move || {
        touch::touch_log_record(ApiKey::INNER_HEIGHT);
        with_prof(|p| p.prof().viewport().1)
    })?;
    crate::webidl::named_accessor(ctx, proto, "clientHeight", g_ch, None)?;
    let efp = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, x: f64, y: f64| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::QUERY_SELECTOR);
            match crate::layout::element_from_point(x, y) {
                Some(n) => handle_value(&c, n),
                None => Ok(Value::new_null(c)),
            }
        },
    )?;
    crate::webidl::define_method(ctx, proto, "elementFromPoint", efp)?;
    let gcs = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, el: Value<'js>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::GET_BOUNDING_CLIENT_RECT);
            let node = el
                .as_object()
                .and_then(Class::<NodeHandle>::from_object)
                .map(|cl| cl.borrow().node);
            let (display, w, h) = match node {
                Some(n) => {
                    let b = crate::layout::rect_for(n);
                    with_doc(|doc| {
                        let hidden = doc
                            .and_then(|p| {
                                let f = p.dom.flags(n);
                                (f & parser_pipeline::dom::node_flags::HIDDEN != 0).then_some(())
                            })
                            .is_some();
                        let disp = if hidden {
                            "none"
                        } else if crate::layout::is_inline_node(n) {
                            "inline"
                        } else {
                            "block"
                        };
                        (disp, b.w, b.h)
                    })
                }
                None => ("none", 0.0, 0.0),
            };
            let o = Object::new(c)?;
            o.set("display", display)?;
            o.set("visibility", "visible")?;
            o.set("opacity", "1")?;
            o.set("transform", "none")?;
            o.set("position", "static")?;
            o.set("width", core_utils::px_to_compact(w).as_str())?;
            o.set("height", core_utils::px_to_compact(h).as_str())?;
            o.set("backgroundColor", "rgba(0, 0, 0, 0)")?;
            Ok(o.into_value())
        },
    )?;
    crate::stackfmt::set_fn_name(ctx, &gcs, "getComputedStyle")?;
    ctx.globals().set("getComputedStyle", gcs)?;
    Ok(())
}

pub(crate) fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "",
    }
}

macro_rules! call_thunk {
    ($field:ident, $c:expr, ($($arg:expr,)*), $fallback:expr) => {{
        let f = THUNKS.with(|t| t.borrow().$field.clone());
        if let Some(f) = f
            && let Ok(f) = f.restore($c)
        {
            f.call(($($arg,)*))?
        } else {
            $fallback
        }
    }};
}

macro_rules! set_named_fn {
    ($ctx:expr, $target:expr, $name:expr, $f:expr) => {{
        let f = Function::new($ctx.clone(), $f)?;
        crate::stackfmt::set_fn_name($ctx, &f, $name)?;
        $target.set($name, f)
    }};
    ($ctx:expr, $target:expr, $key:expr, $name:expr, $f:expr) => {{
        let f = Function::new($ctx.clone(), $f)?;
        crate::stackfmt::set_fn_name($ctx, &f, $name)?;
        $target.set($key, f)
    }};
}

pub(crate) fn promise_resolve<'js>(c: &Ctx<'js>, v: Value<'js>) -> rquickjs::Result<Value<'js>> {
    Ok(call_thunk!(promise_resolve, c, (v.clone(),), v))
}

pub(crate) fn promise_reject<'js>(c: &Ctx<'js>, msg: &str) -> rquickjs::Result<Value<'js>> {
    Ok(call_thunk!(
        promise_reject,
        c,
        (msg,),
        Value::new_undefined(c.clone())
    ))
}

pub(crate) fn promise_reject_value<'js>(
    c: &Ctx<'js>,
    v: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    Ok(call_thunk!(promise_reject_val, c, (v.clone(),), v))
}

fn json_promise<'js>(c: &Ctx<'js>, s: &str) -> rquickjs::Result<Value<'js>> {
    Ok(call_thunk!(
        json_promise,
        c,
        (s,),
        Value::new_undefined(c.clone())
    ))
}

fn install_native_fetch<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let stub: Option<Function<'js>> = globals.get("__silo_stub_fetch").ok().flatten();
    if let Some(stub) = stub {
        THUNKS.with(|t| t.borrow_mut().fetch_stub = Some(Persistent::save(ctx, stub)));
    }
    let parse: Function<'js> = ctx.eval(
        "(function (url, init) { var m = 'GET', h = [], b = null;\
         if (init) { if (init.method) { m = String(init.method).toUpperCase(); }\
         var hs = init.headers;\
         if (hs) { if (typeof hs.forEach === 'function' && typeof hs.append === 'function') { hs.forEach(function (v, k) { h.push([String(k), String(v)]); }); }\
         else { for (var k in hs) { if (Object.prototype.hasOwnProperty.call(hs, k)) { h.push([String(k), String(hs[k])]); } } } }\
         if (init.body != null) { b = (typeof URLSearchParams === 'function' && init.body instanceof URLSearchParams) ? String(init.body) : init.body; } }\
         return [url, m, h, b]; })",
    )?;
    THUNKS.with(|t| t.borrow_mut().fetch_parse = Some(Persistent::save(ctx, parse)));
    let fetch = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>,
              url: rquickjs::String<'js>,
              rest: Rest<Value<'js>>|
              -> rquickjs::Result<Value<'js>> {
            let init: Option<Object<'js>> = rest.first().and_then(|v| v.as_object().cloned());
            if !crate::task::installed() {
                let stub = THUNKS.with(|t| t.borrow().fetch_stub.clone());
                if let Some(s) = stub
                    && let Ok(f) = s.restore(&c)
                {
                    return f.call((url, init.map(|i| i.into_value())));
                }
                return Err(rquickjs::Exception::throw_message(
                    &c,
                    "TypeError: fetch is not available",
                ));
            }
            let parse = THUNKS.with(|t| t.borrow().fetch_parse.clone());
            let Ok(p) = parse.ok_or(()).and_then(|p| p.restore(&c).map_err(|_| ())) else {
                return Err(rquickjs::Exception::throw_message(
                    &c,
                    "TypeError: fetch is not available",
                ));
            };
            let parts: rquickjs::Array = p.call((url.clone(), init))?;
            let method_v: rquickjs::String<'js> = parts.get(1)?;
            let mc = method_v.to_cstring()?;
            let hlist: rquickjs::Array = parts.get(2)?;
            let body_v: Option<Value<'js>> = parts.get(3)?;
            let body: Option<Bytes> = body_v.as_ref().and_then(crate::netapi::js_value_to_bytes);
            let mut headers: crate::task::HeaderList = SmallVec::with_capacity(hlist.len().min(32));
            for i in 0..hlist.len() {
                let pair: rquickjs::Array = hlist.get(i)?;
                let k: rquickjs::String<'js> = pair.get(0)?;
                let v: rquickjs::String<'js> = pair.get(1)?;
                let kc = k.to_cstring()?;
                let vc = v.to_cstring()?;
                headers.push((
                    CompactString::new(kc.as_str()),
                    CompactString::new(vc.as_str()),
                ));
            }
            let uc = {
                let probe = url.clone();
                probe.to_cstring()?
            };
            let cookie = with_prof(|p| p.cookie.clone());
            let reply =
                crate::netapi::bridge_fetch(uc.as_str(), mc.as_str(), headers, body, cookie);
            let Some(reply) = reply else {
                return promise_reject(&c, "TypeError: Failed to fetch");
            };
            if reply.status == 0 {
                return promise_reject(&c, "TypeError: Failed to fetch");
            }
            let text_of = |b: &Bytes| -> std::sync::Arc<str> {
                if b.is_empty() {
                    std::sync::Arc::from("")
                } else if core_utils::utf8::basic::from_utf8(b).is_ok() {
                    let s = unsafe { std::str::from_utf8_unchecked(b) };
                    std::sync::Arc::from(s)
                } else {
                    match String::from_utf8_lossy(b) {
                        std::borrow::Cow::Borrowed(s) => std::sync::Arc::from(s),
                        std::borrow::Cow::Owned(s) => std::sync::Arc::from(s),
                    }
                }
            };
            let ok = reply.ok();
            let bytes: Bytes = reply.body;
            let resp = Object::new(c.clone())?;
            resp.set("ok", ok)?;
            resp.set("status", reply.status)?;
            resp.set("statusText", status_text(reply.status))?;
            resp.set("url", url)?;
            resp.set("redirected", false)?;
            resp.set("type", "basic")?;
            let hdrs = Object::new(c.clone())?;
            for (k, v) in &reply.headers {
                hdrs.set(k.as_str(), v.as_str())?;
            }
            resp.set("headers", hdrs)?;
            {
                let bytes = bytes.clone();
                set_named_fn!(
                    &c,
                    resp,
                    "text",
                    move |cc: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
                        promise_resolve(&cc, text_of(&bytes).as_ref().into_js(&cc)?)
                    }
                )?;
            }
            {
                let bytes = bytes.clone();
                set_named_fn!(
                    &c,
                    resp,
                    "json",
                    move |cc: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
                        json_promise(&cc, text_of(&bytes).as_ref())
                    }
                )?;
            }
            {
                let bytes = bytes.clone();
                set_named_fn!(
                    &c,
                    resp,
                    "arrayBuffer",
                    move |cc: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
                        let buf = rquickjs::ArrayBuffer::new_copy(cc.clone(), bytes.as_ref())?;
                        promise_resolve(&cc, buf.into_value())
                    }
                )?;
            }
            set_named_fn!(
                &c,
                resp,
                "blob",
                move |cc: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
                    let o = Object::new(cc.clone())?;
                    o.set("size", bytes.len() as u32)?;
                    o.set("type", "")?;
                    promise_resolve(&cc, o.into_value())
                }
            )?;
            promise_resolve(&c, resp.into_value())
        },
    )?;
    crate::stackfmt::set_fn_name(ctx, &fetch, "fetch")?;
    globals.set("fetch", fetch)?;
    let _: Value<'js> = ctx.eval("delete globalThis.__silo_stub_fetch")?;
    Ok(())
}

fn add_title_acc<'js>(proto: &Object<'js>) -> rquickjs::Result<()> {
    let ctx = proto.ctx();
    let g = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record_stack(ApiKey::TITLE, &c);
            let t: Option<CompactString> = with_doc(|d| d.and_then(|p| p.title.clone()));
            t.unwrap_or_default().as_str().into_js(&c)
        },
    )?;
    crate::webidl::named_accessor(ctx, proto, "title", g, None)
}

fn add_cookie_prop<'js>(proto: &Object<'js>) -> rquickjs::Result<()> {
    let ctx = proto.ctx();
    let g = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record_stack(ApiKey::COOKIE, &c);
            with_prof(|p| p.cookie.as_str().into_js(&c))
        },
    )?;
    let s = Function::new(ctx.clone(), move |v: String| {
        touch::touch_log_record(ApiKey::COOKIE);
        cookie_set(v.as_str());
    })?;
    crate::webidl::named_accessor(ctx, proto, "cookie", g, Some(s))
}

fn add_doc_body_head<'js>(proto: &Object<'js>) -> rquickjs::Result<()> {
    let ctx = proto.ctx();
    let g_body = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::BODY);
            crate::webidl::opt_node_or_null(&c, query::find_first_by_tag_view("body"))
        },
    )?;
    crate::webidl::named_accessor(ctx, proto, "body", g_body, None)?;
    let g_head = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::HEAD);
            crate::webidl::opt_node_or_null(&c, query::find_first_by_tag_view("head"))
        },
    )?;
    crate::webidl::named_accessor(ctx, proto, "head", g_head, None)
}

fn add_current_script<'js>(proto: &Object<'js>) -> rquickjs::Result<()> {
    let ctx = proto.ctx();
    let g = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record_stack(ApiKey::CURRENT_SCRIPT, &c);
            crate::webidl::opt_node_or_null(&c, CURRENT_SCRIPT.with(Cell::get))
        },
    )?;
    crate::webidl::named_accessor(ctx, proto, "currentScript", g, None)
}

fn build_natives<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let n = Object::new(ctx.clone())?;
    set_named_fn!(ctx, n, "audioFp", "audioFp", || -> f64 {
        let seed = with_prof(|p| p.seed);
        payload_gen::audio_fp(seed)
    })?;
    set_named_fn!(ctx, n, "uaFullVersion", "uaFullVersion", || -> &'static str {
        with_prof(|p| p.prof().ua_full_version().unwrap_or(""))
    })?;
    Ok(n)
}

fn install_timer_natives<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    fn rest_args<'js>(c: &Ctx<'js>, rest: &Rest<Value<'js>>) -> Option<Value<'js>> {
        if rest.is_empty() {
            return None;
        }
        let arr = rquickjs::Array::new(c.clone()).ok()?;
        for (i, v) in rest.iter().enumerate() {
            arr.set(i, v.clone()).ok()?;
        }
        Some(arr.into_value())
    }
    set_named_fn!(ctx, ctx.globals(), "setTimeout", |c: Ctx<'js>,
                                                     cb: Value<'js>,
                                                     delay: Option<f64>,
                                                     rest: Rest<
        Value<'js>,
    >|
     -> rquickjs::Result<
        f64,
    > {
        Ok(timer::schedule(&c, cb, delay.unwrap_or(0.0), rest_args(&c, &rest), false) as f64)
    })?;
    set_named_fn!(ctx, ctx.globals(), "setInterval", |c: Ctx<'js>,
                                                      cb: Value<'js>,
                                                      delay: Option<f64>,
                                                      rest: Rest<
        Value<'js>,
    >|
     -> rquickjs::Result<
        f64,
    > {
        Ok(timer::schedule(&c, cb, delay.unwrap_or(4.0), rest_args(&c, &rest), true) as f64)
    })?;
    set_named_fn!(ctx, ctx.globals(), "clearTimeout", |_c: Ctx<'js>,
                                                       id: f64|
     -> rquickjs::Result<
        (),
    > {
        timer::unschedule(id);
        Ok(())
    })?;
    set_named_fn!(
        ctx,
        ctx.globals(),
        "clearInterval",
        |_c: Ctx<'js>, id: f64| -> rquickjs::Result<()> {
            timer::unschedule(id);
            Ok(())
        }
    )?;
    set_named_fn!(
        ctx,
        ctx.globals(),
        "queueMicrotask",
        |c: Ctx<'js>, cb: Value<'js>| -> rquickjs::Result<()> { timer::microtask(&c, cb) }
    )?;
    set_named_fn!(
        ctx,
        ctx.globals(),
        "requestAnimationFrame",
        |c: Ctx<'js>, cb: Value<'js>| -> rquickjs::Result<f64> {
            Ok(timer::raf_schedule(&c, cb) as f64)
        }
    )?;
    set_named_fn!(
        ctx,
        ctx.globals(),
        "cancelAnimationFrame",
        |_c: Ctx<'js>, id: f64| -> rquickjs::Result<()> {
            timer::raf_cancel(id);
            Ok(())
        }
    )?;
    Ok(())
}

fn install_location_methods<'js>(ctx: &Ctx<'js>, loc_proto: &Object<'js>) -> rquickjs::Result<()> {
    for name in ["assign", "replace"] {
        let f = Function::new(
            ctx.clone(),
            |c: Ctx<'js>, url: rquickjs::String<'js>| -> Value<'js> {
                let url = url.to_cstring().ok();
                let cs = url.as_deref().map(CompactString::new);
                if let Some(cs) = cs {
                    nav_set(cs);
                }
                Value::new_undefined(c)
            },
        )?;
        crate::webidl::define_method(ctx, loc_proto, name, f)?;
    }
    let reload = Function::new(ctx.clone(), |c: Ctx<'js>| -> Value<'js> {
        nav_set(CompactString::const_new(RELOAD_MARK));
        Value::new_undefined(c)
    })?;
    crate::webidl::define_method(ctx, loc_proto, "reload", reload)
}

fn stubpack_acc<'js>(
    ctx: &Ctx<'js>,
    proto: &Object<'js>,
    key: &'static str,
    val: Object<'js>,
) -> rquickjs::Result<()> {
    crate::deviceapi::stubbed_accessor(ctx, proto, key, val, None)
}

fn stubpack_chromium_gated<'js>(
    ctx: &Ctx<'js>,
    target: &Object<'js>,
    key: &'static str,
    val: Object<'js>,
) -> rquickjs::Result<()> {
    let slot = crate::webidl::store_stub(ctx, val);
    let g = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            if with_prof(|p| p.chromium) {
                match crate::webidl::stub_value(&c, slot) {
                    Some(o) => Ok(o.into_value()),
                    None => Ok(Value::new_undefined(c)),
                }
            } else {
                Ok(Value::new_undefined(c))
            }
        },
    )?;
    crate::webidl::named_accessor(ctx, target, key, g, None)
}

fn install_stubpack<'js>(
    ctx: &Ctx<'js>,
    nav_proto: &Object<'js>,
    screen_proto: &Object<'js>,
) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let Ok(pack): rquickjs::Result<Object<'js>> = globals.get("__silo_stubpack") else {
        return Ok(());
    };
    let take = |key: &str| pack.get::<_, Option<Object<'js>>>(key).ok().flatten();
    if let Some(p) = take("plugins") {
        stubpack_acc(ctx, nav_proto, "plugins", p)?;
    }
    if let Some(m) = take("mimeTypes") {
        stubpack_acc(ctx, nav_proto, "mimeTypes", m)?;
    }
    if let Some(u) = take("uaData") {
        stubpack_chromium_gated(ctx, nav_proto, "userAgentData", u)?;
    }
    if let Some(c) = take("chrome") {
        stubpack_chromium_gated(ctx, &globals, "chrome", c)?;
    }
    if let Some(o) = take("orientation") {
        stubpack_acc(ctx, screen_proto, "orientation", o)?;
    }
    Ok(())
}

type FnKey = (u64, u64);
type FnVal = Persistent<Function<'static>>;

struct FnCache {
    cur: std::collections::HashMap<FnKey, FnVal, FxBuild>,
    prev: std::collections::HashMap<FnKey, FnVal, FxBuild>,
}

impl FnCache {
    fn new() -> Self {
        Self {
            cur: fx_map(),
            prev: fx_map(),
        }
    }

    fn fetch(&mut self, k: FnKey) -> Option<FnVal> {
        if let Some(v) = self.cur.get(&k) {
            return Some(v.clone());
        }
        let v = self.prev.remove(&k)?;
        if self.cur.len() >= FN_CACHE_CAP {
            self.prev = std::mem::take(&mut self.cur);
        }
        self.cur.insert(k, v.clone());
        Some(v)
    }

    fn insert(&mut self, k: FnKey, v: FnVal) {
        if self.cur.len() >= FN_CACHE_CAP {
            self.prev = std::mem::take(&mut self.cur);
        }
        self.prev.remove(&k);
        self.cur.insert(k, v);
    }
}

struct JsEnv {
    sha256: Persistent<Function<'static>>,
    md5: Persistent<Function<'static>>,
    fns: RefCell<FnCache>,
    dispatch: Option<Arc<Dispatch>>,
    context: Context,
    oom_lift: u8,
}

fn native_sha256<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    native_hex_hash(ctx, "sha256", 64, |d, o| {
        sha256_hex_into(d, o.try_into().expect("sha256 out"));
    })
}

fn native_md5<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    native_hex_hash(ctx, "md5", 32, |d, o| {
        md5_hex_into(d, o.try_into().expect("md5 out"));
    })
}

fn native_hex_hash<'js, F>(
    ctx: &Ctx<'js>,
    name: &'static str,
    width: usize,
    hash: F,
) -> rquickjs::Result<Function<'js>>
where
    F: Fn(&[u8], &mut [u8]) + Clone + 'js,
{
    let f = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, s: rquickjs::String<'js>| -> rquickjs::Result<rquickjs::String<'js>> {
            let sc = s.to_cstring()?;
            let mut out = [0u8; 64];
            hash(sc.as_bytes(), &mut out[..width]);
            rquickjs::String::from_str(c, core::str::from_utf8(&out[..width]).expect("hex ascii"))
        },
    )?;
    crate::stackfmt::set_fn_name(ctx, &f, name)?;
    Ok(f)
}

fn native_profile<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    let f = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            if let Some(v) = crate::webidl::restore_slot(&c, &PROF_OBJ) {
                return Ok(v.into_value());
            }
            let o = Object::new(c.clone())?;
            with_prof(|p| -> rquickjs::Result<()> {
                o.set("ua", &*p.prof().ua)?;
                o.set("secChUa", &*p.prof().sec_ch_ua)?;
                o.set("platform", p.prof().platform.as_str())?;
                o.set("locale", p.prof().locale.as_str())?;
                o.set("tz", p.prof().tz.as_str())?;
                let (cw, ch) = p.prof().screen_css();
                o.set("screenW", cw)?;
                o.set("screenH", ch)?;
                o.set("mobile", p.mobile)?;
                o.set("chromium", p.chromium)?;
                o.set("firefox", p.firefox)?;
                o.set("seed", p.seed)?;
                o.set("webglVendor", p.prof().webgl_vendor())?;
                o.set("webglRenderer", p.prof().webgl_renderer())?;
                o.set("canvasHash", p.prof().canvas_hex().as_str())?;
                Ok(())
            })?;
            PROF_OBJ.with(|b| *b.borrow_mut() = Some(Persistent::save(&c, o.clone())));
            Ok(o.into_value())
        },
    )?;
    crate::stackfmt::set_fn_name(ctx, &f, "profile")?;
    Ok(f)
}

fn native_mem_getter<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    let f = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            let seed = prof_seed();
            if let Some(o) = MEM_OBJ.with(|m| {
                m.borrow()
                    .as_ref()
                    .and_then(|(s, p)| (*s == seed).then(|| p.clone().restore(&c).ok()))
                    .flatten()
            }) {
                return Ok(o.into_value());
            }
            let dev = with_prof(|p| p.prof().device_memory()).max(4) as u64;
            let base = core_utils::rng::mix_to_range(
                seed,
                core_utils::rng::seeds::SALT_MEM_BASE,
                12_000_000,
            ) + 3_000_000;
            let limit = dev * 1_073_741_824;
            let o = Object::new(c.clone())?;
            o.prop(
                "jsHeapSizeLimit",
                Property::from(limit as f64).enumerable().configurable(),
            )?;
            o.prop(
                "totalJSHeapSize",
                Property::from((base * 2) as f64).enumerable().configurable(),
            )?;
            o.prop(
                "usedJSHeapSize",
                Property::from(base as f64).enumerable().configurable(),
            )?;
            MEM_OBJ.with(|m| {
                *m.borrow_mut() = Some((seed, Persistent::save(&c, o.clone())))
            });
            Ok(o.into_value())
        },
    )?;
    Ok(f)
}

impl JsEnv {
    fn new(bundle: &Bundle) -> Result<Self, rquickjs::Error> {
        let runtime = Runtime::new()?;
        if std::env::var_os("SILO_LEAK_DUMP").is_some() {
            runtime.set_dump_flags(0x4000);
        }
        runtime.set_memory_limit(mem_limit());
        runtime.set_gc_threshold(gc_threshold());
        runtime.set_max_stack_size(STACK_LIMIT);
        runtime.set_interrupt_handler(Some(Box::new(|| {
            let n = INT_TICK.with(|t| {
                let v = t.get().wrapping_add(1);
                t.set(v);
                v
            });
            let cancelled = CANCELLED.with(|c| c.get());
            if cancelled.is_null() {
                return false;
            }
            let flag = unsafe { &*cancelled };
            if !n.is_multiple_of(256) {
                flag.load(std::sync::atomic::Ordering::Acquire)
            } else {
                flag.load(std::sync::atomic::Ordering::Acquire)
                    || Instant::now() >= DEADLINE.with(|d| d.get())
            }
        })));
        let context = Context::full(&runtime)?;
        let (sha256, md5) = context.with(
            |ctx| -> rquickjs::Result<(
                Persistent<Function<'static>>,
                Persistent<Function<'static>>,
            )> {
                let sha = native_sha256(&ctx)?;
                let md5 = native_md5(&ctx)?;
                Ok((Persistent::save(&ctx, sha), Persistent::save(&ctx, md5)))
            },
        )?;
        let dispatch_result = context.with(|ctx| -> rquickjs::Result<Option<Arc<Dispatch>>> {
            let globals = ctx.globals();
            let pr: Function = ctx.eval("(function (v) { return Promise.resolve(v); })")?;
            let pj: Function = ctx.eval("(function (m) { return Promise.reject(new TypeError(m)); })")?;
            let pv: Function = ctx.eval("(function (v) { return Promise.reject(v); })")?;
            let jp: Function = ctx.eval("(function (s) { return Promise.resolve(JSON.parse(s)); })")?;
            THUNKS.with(|t| {
                let mut t = t.borrow_mut();
                t.promise_resolve = Some(Persistent::save(&ctx, pr));
                t.promise_reject = Some(Persistent::save(&ctx, pj));
                t.promise_reject_val = Some(Persistent::save(&ctx, pv));
                t.json_promise = Some(Persistent::save(&ctx, jp));
            });
            timer::init(&ctx)?;
            canvas2d::init(&ctx)?;
            stackfmt::init(&ctx)?;

            let perf = Object::new(ctx.clone())?;
            let perf_proto = host_prototype(&ctx, &perf, "Performance")?;
            let g_to = Function::new(
                ctx.clone(),
                move || {
                    touch::touch_log_record(ApiKey::NOW);
                    clock::time_origin_ms()
                },
            )?;
            crate::webidl::named_accessor(&ctx, &perf_proto, "timeOrigin", g_to, None)?;
            let now_f = Function::new(ctx.clone(), || -> f64 {
                touch::touch_log_record(ApiKey::NOW);
                clock::now_ms_quantized()
            })?;
            crate::webidl::define_method(&ctx, &perf_proto, "now", now_f)?;
            let mem_g: Function = native_mem_getter(&ctx)?;
            crate::webidl::named_accessor(&ctx, &perf_proto, "memory", mem_g, None)?;
            globals.prop(
                "performance",
                Property::from(perf).writable().enumerable().configurable(),
            )?;

            let nav = Object::new(ctx.clone())?;
            let nav_proto = host_prototype(&ctx, &nav, "Navigator")?;
            add_str_acc(&nav_proto, "userAgent", ApiKey::USER_AGENT, |p| &*p.prof().ua)?;
            add_str_acc(&nav_proto, "appVersion", ApiKey::APP_VERSION, |p| {
                p.app_version.as_str()
            })?;
            add_str_acc(&nav_proto, "platform", ApiKey::PLATFORM, |p| {
                p.prof().platform.as_str()
            })?;
            add_str_acc(&nav_proto, "language", ApiKey::LANGUAGE, |p| {
                p.prof().locale.as_str()
            })?;
            add_languages(&nav_proto)?;
            add_acc(
                &nav_proto,
                "hardwareConcurrency",
                ApiKey::HARDWARE_CONCURRENCY,
                |p| p.prof().hw_concurrency(),
            )?;
            add_acc(&nav_proto, "deviceMemory", ApiKey::DEVICE_MEMORY, |p| {
                p.prof().device_memory()
            })?;
            add_acc(&nav_proto, "cookieEnabled", ApiKey::COOKIE_ENABLED, |_| true)?;
            add_acc(&nav_proto, "webdriver", ApiKey::WEBDRIVER, |_| false)?;
            add_acc(
                &nav_proto,
                "maxTouchPoints",
                ApiKey::MAX_TOUCH_POINTS,
                |p| if p.mobile { 5u32 } else { 0u32 },
            )?;
            add_str_acc(&nav_proto, "productSub", ApiKey::PRODUCT_SUB, |_| "20030107")?;
            add_str_acc(&nav_proto, "product", ApiKey::PRODUCT, |_| "Gecko")?;
            add_str_acc(&nav_proto, "vendor", ApiKey::VENDOR, |_| "Google Inc.")?;
            add_conn_singleton(&nav_proto)?;
            let firefox = with_prof(|p| p.firefox);
            if firefox {
                add_str_acc(&nav_proto, "oscpu", ApiKey::OSCPU, |_| "Windows NT 10.0")?;
                add_str_acc(&nav_proto, "buildID", ApiKey::BUILD_ID, |_| "20181001000000")?;
            }
            add_acc_null(&nav_proto, "doNotTrack", ApiKey::DO_NOT_TRACK)?;
            add_acc_null(
                &nav_proto,
                "globalPrivacyControl",
                ApiKey::GLOBAL_PRIVACY_CONTROL,
            )?;
            add_acc(&nav_proto, "pdfViewerEnabled", ApiKey::PDF_VIEWER, |_| true)?;
            let java = Function::new(ctx.clone(), || -> bool { false })?;
            crate::webidl::define_method(&ctx, &nav_proto, "javaEnabled", java)?;
            globals.prop(
                "navigator",
                Property::from(nav).writable().enumerable().configurable(),
            )?;

            let scr = Object::new(ctx.clone())?;
            let scr_proto = host_prototype(&ctx, &scr, "Screen")?;
            add_acc(&scr_proto, "width", ApiKey::WIDTH, |p| p.prof().screen_css().0)?;
            add_acc(&scr_proto, "height", ApiKey::HEIGHT, |p| p.prof().screen_css().1)?;
            add_acc(&scr_proto, "availWidth", ApiKey::AVAIL_WIDTH, |p| {
                p.prof().avail_css().0
            })?;
            add_acc(&scr_proto, "availHeight", ApiKey::AVAIL_HEIGHT, |p| {
                p.prof().avail_css().1
            })?;
            add_acc(&scr_proto, "colorDepth", ApiKey::COLOR_DEPTH, |_| 24u32)?;
            add_acc(&scr_proto, "pixelDepth", ApiKey::PIXEL_DEPTH, |_| 24u32)?;
            globals.prop(
                "screen",
                Property::from(scr).writable().enumerable().configurable(),
            )?;

            let loc = Object::new(ctx.clone())?;
            let loc_proto = host_prototype(&ctx, &loc, "Location")?;
            add_str_acc(&loc_proto, "href", ApiKey::LOCATION_HREF, |p| {
                p.href.as_str()
            })?;
            add_str_acc(&loc_proto, "origin", ApiKey::LOCATION_ORIGIN, |p| {
                p.origin.as_str()
            })?;
            add_str_acc(&loc_proto, "host", ApiKey::LOCATION_HOST, |p| p.host.as_str())?;
            add_str_acc(&loc_proto, "hostname", ApiKey::LOCATION_HOSTNAME, |p| {
                p.host.as_str()
            })?;
            add_str_acc(&loc_proto, "pathname", ApiKey::LOCATION_PATHNAME, |p| {
                p.path.as_str()
            })?;
            add_str_acc(&loc_proto, "protocol", ApiKey::LOCATION_PROTOCOL, |_| "https:")?;
            add_str_acc(&loc_proto, "search", ApiKey::LOCATION_SEARCH, |p| {
                p.search.as_str()
            })?;
            add_str_acc(&loc_proto, "hash", ApiKey::LOCATION_HASH, |p| {
                p.hash.as_str()
            })?;
            install_location_methods(&ctx, &loc_proto)?;
            globals.prop(
                "location",
                Property::from(loc).writable().enumerable().configurable(),
            )?;

            let doc = Object::new(ctx.clone())?;
            let doc_proto = host_prototype(&ctx, &doc, "Document")?;
            add_cookie_prop(&doc_proto)?;
            add_str_acc(&doc_proto, "referrer", ApiKey::REFERRER, |_| "")?;
            add_title_acc(&doc_proto)?;
            add_str_acc(&doc_proto, "URL", ApiKey::URL, |p| p.href.as_str())?;
            add_str_acc(&doc_proto, "origin", ApiKey::ORIGIN, |p| p.origin.as_str())?;
            add_str_acc(&doc_proto, "domain", ApiKey::DOMAIN, |p| p.host.as_str())?;
            add_str_acc(&doc_proto, "readyState", ApiKey::READY_STATE, |_| {
                match dispatch::ready_state() {
                    dispatch::READY_COMPLETE => "complete",
                    dispatch::READY_INTERACTIVE => "interactive",
                    _ => "loading",
                }
            })?;
            add_str_acc(&doc_proto, "visibilityState", ApiKey::VISIBILITY, |_| "visible")?;
            add_acc(&doc_proto, "hidden", ApiKey::VISIBILITY, |_| false)?;
            add_str_acc(&doc_proto, "characterSet", ApiKey::CHARACTER_SET, |_| "UTF-8")?;
            add_str_acc(&doc_proto, "contentType", ApiKey::CONTENT_TYPE, |_| "text/html")?;
            add_query_selector(&ctx, &doc_proto)?;
            add_get_element_by_id(&ctx, &doc_proto)?;
            add_doc_geometry(&ctx, &doc_proto)?;
            add_method_empty(&doc_proto, "getElementsByTagName", ApiKey::GET_ELEMENTS_BY_TAG_NAME)?;
            collection_accessor(&doc_proto, "scripts", ColKind::Scripts)?;
            collection_accessor(&doc_proto, "forms", ColKind::Forms)?;
            collection_accessor(&doc_proto, "images", ColKind::Images)?;
            collection_accessor(&doc_proto, "links", ColKind::Links)?;
            collection_accessor(&doc_proto, "embeds", ColKind::Embeds)?;
            add_doc_body_head(&doc_proto)?;
            add_current_script(&doc_proto)?;
            globals.prop(
                "document",
                Property::from(doc.clone()).writable().enumerable().configurable(),
            )?;
            let crypto = crate::cryptoapi::build(&ctx)?;
            globals.prop(
                "crypto",
                Property::from(crypto).writable().enumerable().configurable(),
            )?;
            crate::webidl::install(&ctx, &doc, &doc_proto)?;

            add_win_acc(&ctx, "innerWidth", ApiKey::INNER_WIDTH, |p| p.prof().viewport().0)?;
            add_win_acc(&ctx, "innerHeight", ApiKey::INNER_HEIGHT, |p| p.prof().viewport().1)?;
            add_win_acc(&ctx, "outerWidth", ApiKey::OUTER_WIDTH, |p| {
                p.prof().avail_css().0
            })?;
            add_win_acc(&ctx, "outerHeight", ApiKey::OUTER_HEIGHT, |p| {
                p.prof().avail_css().1
            })?;
            add_win_acc(&ctx, "devicePixelRatio", ApiKey::DEVICE_PIXEL_RATIO, |p| {
                p.prof().device_pixel_ratio()
            })?;
            add_win_acc(&ctx, "screenX", ApiKey::SCREEN_X, |_| 0.0)?;
            add_win_acc(&ctx, "screenY", ApiKey::SCREEN_Y, |_| 0.0)?;
            add_win_acc(&ctx, "pageXOffset", ApiKey::PAGE_X_OFFSET, |_| 0.0)?;
            add_win_acc(&ctx, "pageYOffset", ApiKey::PAGE_Y_OFFSET, |_| 0.0)?;
            let win_tag: rquickjs::Symbol = ctx.eval("Symbol.toStringTag")?;
            globals.prop(win_tag, Property::from("Window").configurable())?;

            let math: Object = ctx.eval("Math")?;
            let random_f = Function::new(ctx.clone(), move || -> f64 {
                touch::touch_log_record(ApiKey::RANDOM);
                FAST_RNG.with(|c| {
                    let mut rng = c.get();
                    let v = rng.next_f64();
                    c.set(rng);
                    v
                })
            })?;
            crate::stackfmt::set_fn_name(&ctx, &random_f, "random")?;
            math.prop(
                "random",
                Property::from(random_f).writable().configurable(),
            )?;
            let p_getter = native_profile(&ctx)?;
            let natives = build_natives(&ctx)?;
            let bundle_fn: Function = ctx.eval(bundle.as_str())?;
            let _: Value = bundle_fn.call((p_getter, natives))?;
            let dispatch = Dispatch::capture(&ctx);
            install_stubpack(&ctx, &nav_proto, &scr_proto)?;
            let _: Value = ctx.eval(
                "delete globalThis.__silo_feed; delete globalThis.__silo_reset; delete globalThis.__silo_domready; delete globalThis.__silo_pageload; delete globalThis.__silo_stubpack;",
            )?;
            stackfmt::install_error_stack(&ctx)?;
            stackfmt::install_tostring(&ctx)?;
            install_timer_natives(&ctx)?;
            install_native_fetch(&ctx)?;
            netapi::install(&ctx)?;
            intl::install(&ctx)?;
            observers::install(&ctx)?;
            deviceapi::install(&ctx, &nav_proto)?;
            pageapi::install(&ctx, &doc_proto)?;
            Ok::<Option<Arc<Dispatch>>, rquickjs::Error>(dispatch.map(Arc::new))
        });
        let dispatch = dispatch_result?;
        let env = Self {
            sha256,
            md5,
            fns: RefCell::new(FnCache::new()),
            dispatch,
            context,
            oom_lift: 0,
        };
        Ok(env)
    }

    fn exec(
        &mut self,
        domain: u64,
        skel: u64,
        src: &Arc<str>,
        args: &[Lit],
        snap: &ProfileSnap,
        input: Option<&Arc<[RawEvent]>>,
    ) -> (Result<Option<CompactString>, ExecError>, bool) {
        crate::cryptoapi::reseed(snap.seed());
        store_prof(snap);
        clock::reset();
        timer::clear_all();
        cookie_out_clear();
        nav_clear();
        dispatch::set_ready_state(dispatch::READY_LOADING);
        self.context.runtime().set_memory_limit(mem_limit());
        self.context.runtime().set_gc_threshold(gc_threshold());
        FAST_RNG.with(|r| {
            r.set(core_utils::rng::Rng::new(
                snap.seed() ^ core_utils::rng::seeds::SALT_FOCUS_ENTER,
            ));
        });
        if let Some(dispatch) = self.dispatch.as_ref() {
            dispatch.feed(&self.context, input);
        }
        let dispatch = self.dispatch.clone();
        let mut local_hit = false;
        let result: Result<Option<CompactString>, rquickjs::Error> = {
            let JsEnv {
                context,
                fns,
                sha256,
                md5,
                ..
            } = self;
            context.with(|ctx| {
                let sha = sha256.clone().restore(&ctx)?;
                let md = md5.clone().restore(&ctx)?;
                let mut cache = fns.borrow_mut();
                let func: Function = if let Some(f) = cache.fetch((domain, skel)) {
                    local_hit = true;
                    f.restore(&ctx)?
                } else {
                    let val: Value = ctx.eval(src.as_ref())?;
                    if !matches!(val.type_of(), Type::Function | Type::Constructor) {
                        return Err(rquickjs::Error::new_from_js_message(
                            "Value",
                            "Function",
                            "normalized source did not yield function",
                        ));
                    }
                    let f: Function = val.get()?;
                    cache.insert((domain, skel), Persistent::save(&ctx, f.clone()));
                    f
                };
                drop(cache);
                let arr = rquickjs::Array::new(ctx.clone())?;
                for (i, lit) in args.iter().enumerate() {
                    match lit {
                        Lit::Num(f) => arr.set(i, *f)?,
                        Lit::Str(s) => arr.set(i, s.as_str())?,
                    }
                }
                let out: Value = func.call((arr, sha, md))?;
                if let Some(d) = dispatch.as_ref() {
                    let d1 = Arc::clone(d);
                    timer::defer(&ctx, move |c| d1.fire_domready_ctx(c))?;
                    let d2 = Arc::clone(d);
                    timer::defer(&ctx, move |c| d2.fire_pageload_ctx(c))?;
                }
                let deadline = with_control_deadline();
                timer::run_loop(&ctx, deadline);
                Ok(coerce(out))
            })
        };
        let token = match result {
            Ok(Some(t)) => Ok(Some(t)),
            Ok(None) => Err(ExecError::NoResult),
            Err(e) => Err(classify(&e, &self.context)),
        };
        (token, local_hit)
    }
}

pub(crate) fn with_control_deadline() -> Instant {
    DEADLINE.with(|d| d.get())
}

#[inline(always)]
fn format_worker_error<E: core::fmt::Display>(err: E) -> CompactString {
    use core::fmt::Write;
    let mut s = compact_str::CompactString::new("");
    let _ = write!(s, "{err}");
    s
}

fn coerce(v: Value<'_>) -> Option<CompactString> {
    match v.type_of() {
        Type::String => v.get::<String>().ok().map(CompactString::from),
        Type::Int => v
            .get::<i32>()
            .ok()
            .map(|n| core_utils::int_to_compact(n as i64)),
        Type::Float => v.get::<f64>().ok().map(core_utils::float_to_compact),
        Type::Bool => v
            .get::<bool>()
            .ok()
            .map(|b| CompactString::new(if b { "true" } else { "false" })),
        _ => None,
    }
}

fn classify(err: &rquickjs::Error, context: &Context) -> ExecError {
    if !matches!(err, rquickjs::Error::Exception) {
        return ExecError::Js(format_worker_error(err));
    }
    context.with(|ctx| {
        let caught = ctx.catch();
        if caught.is_uncatchable_error() {
            return ExecError::Timeout;
        }
        let exc: Option<String> = if let Some(e) = caught.as_exception() {
            e.message()
        } else if let Some(obj) = caught.as_object().cloned() {
            rquickjs::Exception::from_object(obj).and_then(|e| e.message())
        } else {
            caught.get::<String>().ok()
        };
        if let Some(msg) = exc {
            if msg.contains("out of memory") {
                return ExecError::Oom;
            }
            return ExecError::Js(msg.into());
        }
        ExecError::Oom
    })
}

struct Worker {
    env: JsEnv,
    bundle: Arc<Bundle>,
    cache: Arc<NormCache>,
    events: EventTx,
    next_sweep: Instant,
}

impl Worker {
    fn new(bundle: Arc<Bundle>, cache: Arc<NormCache>, events: EventTx) -> Option<Self> {
        let env = match JsEnv::new(&bundle) {
            Ok(env) => env,
            Err(_) => {
                drain_env_locals();
                return None;
            }
        };
        Some(Self {
            env,
            bundle,
            cache,
            events,
            next_sweep: Instant::now(),
        })
    }

    fn rebuild_env(&mut self) -> bool {
        let _ = self.env.context.with(|ctx| {
            crate::intl::teardown_intl(&ctx);
        });
        drain_env_locals();
        DEADLINE.with(|d| d.set(Instant::now() + Duration::from_secs(60)));
        CANCELLED.with(|c| c.set(std::ptr::null()));
        match JsEnv::new(&self.bundle) {
            Ok(env) => {
                self.env = env;
                true
            }
            Err(_) => {
                drain_env_locals();
                false
            }
        }
    }

    fn gc_and_maybe_lift(&mut self) -> bool {
        self.env.context.runtime().run_gc();
        if self.env.oom_lift < 1 {
            self.env.oom_lift += 1;
            let runtime = self.env.context.runtime();
            runtime.set_memory_limit(mem_limit() * 4);
            true
        } else {
            false
        }
    }

    fn run(&mut self, req: ExecReq, control: &ExecControl) -> ExecOutcome {
        let start = Instant::now();
        if control.stopped() {
            return ExecOutcome::failed(ExecError::Timeout);
        }
        NET_SLOT.with(|s| s.set(req.net_slot));
        current_script_set(req.script_node);
        if start >= self.next_sweep {
            self.cache.sweep(300);
            self.next_sweep = start + Duration::from_secs(30);
        }
        touch::touch_log_reset();
        crate::webidl::clear_handles();
        store_doc(req.doc.clone());
        match req.kind {
            ExecKind::Wasm => {
                let (token, err) = if is_wasm_magic(&req.script) {
                    match run_wasm(&req.script, control.deadline) {
                        Ok(tok) => {
                            let _ = self.events.try_send(Event::WasmRun);
                            (Some(tok), None)
                        }
                        Err(e) => {
                            let _ = self.events.try_send(Event::WasmFail);
                            let err = match e {
                                crate::wasm::WasmError::Timeout => ExecError::WasmFuel,
                                crate::wasm::WasmError::Fuel => ExecError::WasmFuel,
                                crate::wasm::WasmError::Imports => ExecError::WasmImports,
                                _ => ExecError::WasmCompile,
                            };
                            (None, Some(err))
                        }
                    }
                } else {
                    (None, Some(ExecError::WasmCompile))
                };
                if control.stopped() {
                    return ExecOutcome::failed(ExecError::Timeout);
                }
                return outcome_now(token, ExecPath::Wasm, false, err);
            }
            ExecKind::Anubis => {
                let solved = anubis_solve(&req, control);
                let ev = if solved.is_ok() {
                    Event::ExecDone(start.elapsed().as_millis() as u64)
                } else {
                    Event::ExecFail
                };
                let _ = self.events.try_send(ev);
                if control.stopped() {
                    return ExecOutcome::failed(ExecError::Timeout);
                }
                return match solved {
                    Ok((token, sol)) => {
                        let mut out = outcome_now(Some(token), ExecPath::Compile, false, None);
                        out.anubis = Some(sol);
                        out
                    }
                    Err(err) => outcome_now(None, ExecPath::Compile, false, Some(err)),
                };
            }
            ExecKind::Js => {}
        }
        let domain = req.domain;
        let raw = NormCache::raw_hash(&req.script);
        let mut fresh_src = None;
        let (skel, args, raw_hit) = match self.cache.lookup_raw(domain, raw) {
            Some((skel, args)) => {
                let _ = self.events.try_send(Event::CacheHit);
                (skel, args, true)
            }
            None => {
                let _ = self.events.try_send(Event::CacheMiss);
                match normalize(&req.script) {
                    Ok(n) => {
                        let skel = n.skel;
                        let src = n.src;
                        let args = Arc::new(n.args);
                        if control.stopped() {
                            return ExecOutcome::failed(ExecError::Timeout);
                        }
                        self.cache.put_raw(domain, raw, skel, Arc::clone(&args));
                        self.cache.put_src(domain, skel, Arc::clone(&src));
                        fresh_src = Some(src);
                        (skel, args, false)
                    }
                    Err(_) => {
                        let _ = self.events.try_send(Event::ParseFail);
                        return outcome_now(None, ExecPath::Compile, false, Some(ExecError::Parse));
                    }
                }
            }
        };
        let src = match fresh_src.or_else(|| self.cache.lookup_src(domain, skel)) {
            Some(src) => src,
            None => match normalize(&req.script) {
                Ok(n) => {
                    if control.stopped() {
                        return ExecOutcome::failed(ExecError::Timeout);
                    }
                    self.cache.put_src(domain, skel, n.src.clone());
                    n.src
                }
                Err(_) => {
                    let _ = self.events.try_send(Event::ParseFail);
                    return outcome_now(None, ExecPath::Compile, false, Some(ExecError::Parse));
                }
            },
        };
        if control.stopped() {
            return ExecOutcome::failed(ExecError::Timeout);
        }
        let req_input = req.input.as_ref();
        let (mut token, local_hit) = self
            .env
            .exec(domain, skel, &src, &args, &req.snap, req_input);
        if matches!(token, Err(ExecError::Oom)) && self.gc_and_maybe_lift() {
            token = self
                .env
                .exec(domain, skel, &src, &args, &req.snap, req_input)
                .0;
            self.env.context.runtime().set_memory_limit(mem_limit());
        }
        if control.stopped() && !matches!(&token, Err(ExecError::Oom)) {
            return ExecOutcome::failed(ExecError::Timeout);
        }
        if token.is_ok() && !local_hit {
            let _ = self.events.try_send(Event::Compile);
        }
        let path = if raw_hit {
            ExecPath::RawHit
        } else if local_hit {
            ExecPath::NormHit
        } else {
            ExecPath::Compile
        };
        let (token_opt, err) = match token {
            Ok(t) => (t, None),
            Err(e) => (None, Some(e)),
        };
        if let Some(e) = &err {
            let ev = match e {
                ExecError::Timeout => Event::Timeout,
                ExecError::Oom => Event::Oom,
                ExecError::NoResult => Event::NoResult,
                _ => Event::ExecFail,
            };
            let _ = self.events.try_send(ev);
        }
        let _ = self
            .events
            .try_send(Event::ExecDone(start.elapsed().as_millis() as u64));
        outcome_now(token_opt, path, raw_hit || local_hit, err)
    }
}
fn anubis_solve(
    req: &ExecReq,
    control: &ExecControl,
) -> Result<(CompactString, anubis_solver::SolvedAnubis), ExecError> {
    let ch = anubis_solver::AnubisChallenge::parse(&req.script)
        .map_err(|_| ExecError::Js(CompactString::const_new("anubis parse failed")))?;
    let threads = (*NCPUS).clamp(1, 8);
    let deadline = control
        .deadline
        .checked_sub(Duration::from_millis(250))
        .unwrap_or(control.deadline);
    let sol = anubis_solver::solve(&ch, threads, req.snap.prof.cpu_scale(), Some(deadline))
        .map_err(|_| ExecError::Js(CompactString::const_new("anubis solve failed")))?;
    let mut answer = String::with_capacity(192);
    anubis_solver::answer_json(&sol, &mut answer);
    Ok((CompactString::from(answer), sol))
}


static NCPUS: LazyLock<usize> = LazyLock::new(|| {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
});

fn outcome_now(
    token: Option<CompactString>,
    path: ExecPath,
    cache_hit: bool,
    err: Option<ExecError>,
) -> ExecOutcome {
    ExecOutcome {
        token,
        path,
        cache_hit,
        err,
        touches: touch::touch_log_count(),
        cookie_out: cookie_out_take(),
        nav: nav_take(),
        anubis: None,
    }
}

fn drain_env_locals() {
    clear_request_state();
    CONTROL.with(|active| active.set(std::ptr::null()));
    INT_TICK.with(|t| t.set(0));
    timer::clear_thunks();
    crate::touch::clear_thunks();
    canvas2d::clear_thunks();
    intl::clear_thunks();

    stackfmt::clear();
    crate::layout::clear();
    crate::webidl::clear();
    CONN_OBJ.with(|c| *c.borrow_mut() = None);
    PROF_OBJ.with(|c| *c.borrow_mut() = None);
    MEM_OBJ.with(|c| *c.borrow_mut() = None);
    THUNKS.with(|t| *t.borrow_mut() = Thunks::new());
}

pub(crate) fn worker_main(
    idx: usize,
    rx: crate::task::ExecQueueRx,
    bundle: Arc<Bundle>,
    events: EventTx,
    cache: Arc<NormCache>,
) {
    if let Ok(n) = std::thread::available_parallelism() {
        let n = n.get();
        if n > 1 {
            pin_thread((idx + 1) % n);
        }
    }

    let mut worker: Option<Worker> = None;
    while let Some(task) = rx.recv() {
        if task.control.stopped() {
            let _ = task.reply.send(ExecOutcome::failed(ExecError::Timeout));
            continue;
        }
        if worker.is_none() {
            worker = Worker::new(Arc::clone(&bundle), Arc::clone(&cache), events.clone());
        }
        let Some(w) = worker.as_mut() else {
            let _ = task.reply.send(ExecOutcome::failed(ExecError::Timeout));
            continue;
        };
        let scope = RequestScope::enter(Arc::clone(&task.control));
        let req = task.req;
        let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            w.run(req, &task.control)
        })) {
            Ok(o) => o,
            Err(_) => ExecOutcome::failed(ExecError::Panic),
        };
        drop(scope);
        let rebuild = matches!(&outcome.err, Some(ExecError::Oom | ExecError::Panic));
        let _ = task.reply.send(outcome);
        if rebuild
            && let Some(w) = worker.as_mut()
            && !w.rebuild_env()
        {
            break;
        }
    }
    if let Some(w) = worker {
        let _ = w.env.context.with(|ctx| {
            crate::intl::teardown_intl(&ctx);
        });
        drain_env_locals();
        let _ = w.env.context.with(|ctx| {
            let _: Result<Value, _> = ctx.eval(
                "for (var k in globalThis) { try { delete globalThis[k]; } catch (e) {} }",
            );
        });
        w.env.context.runtime().run_gc();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("polyfill unreadable: {0}")]
    Io(#[from] std::io::Error),
    #[error("polyfill not utf8")]
    Utf8,
}

pub struct Bundle {
    src: BundleSrc,
}

enum BundleSrc {
    Mmap(memmap2::Mmap),
    Mem(Arc<str>),
}

impl Bundle {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BundleError> {
        let file = File::open(path)?;
        let map = unsafe { memmap2::Mmap::map(&file)? };
        if core_utils::utf8::basic::from_utf8(&map).is_err() {
            return Err(BundleError::Utf8);
        }
        Ok(Self {
            src: BundleSrc::Mmap(map),
        })
    }

    pub fn from_source(src: Arc<str>) -> Self {
        Self {
            src: BundleSrc::Mem(src),
        }
    }

    pub fn as_str(&self) -> &str {
        match &self.src {
            BundleSrc::Mmap(map) => unsafe { std::str::from_utf8_unchecked(map) },
            BundleSrc::Mem(src) => src,
        }
    }
}
