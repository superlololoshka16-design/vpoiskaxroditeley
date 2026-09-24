use crate::task as fetch_bridge;
use crate::timer;
use crate::touch::{self, ApiKey};
use crate::worker::{promise_resolve, status_text, with_prof};
use bytes::Bytes;
use compact_str::CompactString;
use core_utils::rng::mix_ctx;
use core_utils::rng::seeds;
use core_utils::sha256_seed_tail;
use core_utils::xxh3;
use core_utils::{FxBuild, fx_map};
use rquickjs::class::Trace;
use rquickjs::function::Rest;
use rquickjs::{Class, Ctx, Function, IntoJs as _, JsLifetime, Object, Persistent, Value};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use crate::webidl::{RegCell, RegVec, cached_persistent, install_host_ctor};

const XHR_CAP: usize = 512;
const XHR_HDR_CAP: usize = 64;
const XHR_LISTENER_CAP: usize = 64;
const RTC_CAP: usize = 256;
const ICE_CHARS: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz012345";
const ICE_UFRAG_SALT: u64 = core_utils::rng::seeds::SALT_ICE_UFRAG;
const ICE_PWD_SALT: u64 = core_utils::rng::seeds::SALT_ICE_PWD;
const RESP_KIND_PLAIN: u8 = 0;
const RESP_KIND_TEXT: u8 = 1;
const RESP_KIND_JSON: u8 = 2;

const XHR_BANNED_HEADERS: [&str; 18] = [
    "accept-charset",
    "accept-encoding",
    "connection",
    "content-length",
    "cookie",
    "cookie2",
    "date",
    "dnt",
    "expect",
    "host",
    "keep-alive",
    "origin",
    "referer",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "via",
];

struct Reply {
    status: u16,
    headers: crate::task::HeaderList,
    body: Bytes,
    ok: bool,
    url: CompactString,
    text: Option<CompactString>,
}

impl Reply {
    fn offline_204() -> Self {
        Self {
            status: 204,
            headers: SmallVec::new(),
            body: Bytes::new(),
            ok: true,
            url: CompactString::const_new(""),
            text: None,
        }
    }

    fn header_find(&self, name: &str) -> Option<&CompactString> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    }

    fn decode_text(&mut self) {
        if self.text.is_none() {
            self.text = Some(match core_utils::utf8::basic::from_utf8(&self.body) {
                Ok(s) => CompactString::from(s),
                Err(_) => CompactString::from(String::from_utf8_lossy(&self.body).into_owned()),
            });
        }
    }
}

struct XhrState {
    method: CompactString,
    url: CompactString,
    req_headers: SmallVec<[(CompactString, CompactString); 8]>,
    sent: bool,
    aborted: bool,
    completed: bool,
    ready: u8,
    reply: Option<Reply>,
    with_credentials: bool,
    timeout_ms: f64,
    response_kind: u8,
    onreadystatechange: Option<Persistent<Function<'static>>>,
    onload: Option<Persistent<Function<'static>>>,
    onerror: Option<Persistent<Function<'static>>>,
    onloadend: Option<Persistent<Function<'static>>>,
    onabort: Option<Persistent<Function<'static>>>,
    listeners: Vec<(CompactString, Persistent<Function<'static>>)>,
}

impl XhrState {
    fn fresh() -> Self {
        Self {
            method: CompactString::const_new(""),
            url: CompactString::const_new(""),
            req_headers: SmallVec::new(),
            sent: false,
            aborted: false,
            completed: false,
            ready: 0,
            reply: None,
            with_credentials: false,
            timeout_ms: 0.0,
            response_kind: RESP_KIND_PLAIN,
            onreadystatechange: None,
            onload: None,
            onerror: None,
            onloadend: None,
            onabort: None,
            listeners: Vec::new(),
        }
    }
}

struct RtcState {
    seed: u64,
    local_sdp: Option<CompactString>,
    onicecandidate: Option<Persistent<Function<'static>>>,
    connection_state: &'static str,
    ice_state: &'static str,
    signaling: &'static str,
    data_channels: u32,
}

impl RtcState {
    fn fresh(seed: u64) -> Self {
        Self {
            seed,
            local_sdp: None,
            onicecandidate: None,
            connection_state: "new",
            ice_state: "new",
            signaling: "stable",
            data_channels: 0,
        }
    }
}

thread_local! {
    static XHR_REG: RegCell<XhrState> = const { RefCell::new(RegVec::with_tag(1)) };
    static RTC_REG: RegCell<RtcState> = const { RefCell::new(RegVec::with_tag(2)) };
    static SELF_OBJ: RefCell<HashMap<u64, Persistent<Object<'static>>, FxBuild>> = RefCell::new(fx_map());
    static JSON_PARSE: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
    static NEW_MAP: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
}

pub(crate) fn clear_registry() {
    XHR_REG.with(|m| m.borrow_mut().clear());
    RTC_REG.with(|m| m.borrow_mut().clear());
    SELF_OBJ.with(|m| m.borrow_mut().clear());
    JSON_PARSE.with(|m| *m.borrow_mut() = None);
    NEW_MAP.with(|m| *m.borrow_mut() = None);
}

fn json_parse<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    cached_persistent(ctx, &JSON_PARSE, |c| c.eval("JSON.parse"))
}

fn new_map<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let ctor = cached_persistent(ctx, &NEW_MAP, |c| {
        c.eval("(function () { return new Map(); })")
    })?;
    let v: Value = ctor.call(())?;
    v.get()
}

use crate::webidl::with_reg;

fn with_xhr<R>(id: u64, f: impl FnOnce(&mut XhrState) -> R) -> Option<R> {
    with_reg(&XHR_REG, id, f)
}

fn with_rtc<R>(id: u64, f: impl FnOnce(&mut RtcState) -> R) -> Option<R> {
    with_reg(&RTC_REG, id, f)
}

fn save_self<'js>(ctx: &Ctx<'js>, id: u64, obj: Object<'js>) {
    SELF_OBJ.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() < XHR_CAP + RTC_CAP {
            m.insert(id, Persistent::save(ctx, obj));
        }
    });
}

fn self_obj<'js>(ctx: &Ctx<'js>, id: u64) -> Option<Object<'js>> {
    SELF_OBJ
        .with(|m| m.borrow().get(&id).cloned())
        .and_then(|p| p.restore(ctx).ok())
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "XMLHttpRequest")]
pub(crate) struct Xhr {
    #[qjs(skip_trace)]
    id: u64,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Xhr {
    #[qjs(get, rename = "readyState")]
    pub fn ready_state(&self) -> u8 {
        with_xhr(self.id, |s| s.ready).unwrap_or(0)
    }

    #[qjs(get, rename = "status")]
    pub fn status(&self) -> u16 {
        with_xhr(self.id, |s| s.reply.as_ref().map(|r| r.status).unwrap_or(0)).unwrap_or(0)
    }

    #[qjs(get, rename = "statusText")]
    pub fn status_text<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let st = self.status();
        status_text(st).into_js(&ctx)
    }

    #[qjs(get, rename = "responseURL")]
    pub fn response_url<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let under = with_xhr(self.id, |s| {
            s.reply
                .as_ref()
                .map(|r| r.url.is_empty())
                .unwrap_or(true)
        })
        .unwrap_or(true);
        if under {
            return Ok(Value::new_null(ctx));
        }
        with_xhr(self.id, |s| {
            s.reply
                .as_ref()
                .map(|r| r.url.as_str().into_js(&ctx))
                .unwrap_or_else(|| Ok(Value::new_null(ctx.clone())))
        })
        .unwrap_or_else(|| Ok(Value::new_null(ctx.clone())))
    }

    #[qjs(get, rename = "responseText")]
    pub fn response_text<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let kind = with_xhr(self.id, |s| s.response_kind).unwrap_or(RESP_KIND_PLAIN);
        if kind == RESP_KIND_JSON {
            return Err(rquickjs::Exception::throw_message(
                &ctx,
                "InvalidStateError: responseText is only available if responseType is '' or 'text'",
            ));
        }
        with_xhr(self.id, |s| {
            s.reply
                .as_mut()
                .map(|r| {
                    r.decode_text();
                    r.text
                        .as_deref()
                        .unwrap_or("")
                        .into_js(&ctx)
                })
                .unwrap_or_else(|| "".into_js(&ctx))
        })
        .unwrap_or_else(|| "".into_js(&ctx))
    }

    #[qjs(get, rename = "response")]
    pub fn response<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let kind = with_xhr(self.id, |s| s.response_kind).unwrap_or(RESP_KIND_PLAIN);
        let ctx_j = ctx.clone();
        let json_text = |id: u64| -> rquickjs::Result<Option<rquickjs::String<'js>>> {
            let inner = with_xhr(id, |s| {
                s.reply
                    .as_mut()
                    .map(|r| {
                        r.decode_text();
                        match r.text.as_deref() {
                            Some(t) if !t.is_empty() => {
                                Some(rquickjs::String::from_str(ctx_j.clone(), t))
                            }
                            _ => None,
                        }
                    })
                    .flatten()
            });
            match inner.flatten() {
                Some(r) => r.map(Some),
                None => Ok(None),
            }
        };
        match kind {
            RESP_KIND_TEXT => with_xhr(self.id, |s| {
                s.reply
                    .as_mut()
                    .map(|r| {
                        r.decode_text();
                        r.text
                            .as_deref()
                            .unwrap_or("")
                            .into_js(&ctx)
                    })
                    .unwrap_or_else(|| "".into_js(&ctx))
            })
            .unwrap_or_else(|| "".into_js(&ctx)),
            RESP_KIND_JSON => {
                let Some(s) = json_text(self.id)? else {
                    return Ok(Value::new_null(ctx));
                };
                let parse = json_parse(&ctx)?;
                match parse.call((s,)) {
                    Ok(v) => Ok(v),
                    Err(_) => {
                        let _ = ctx.catch();
                        Ok(Value::new_null(ctx))
                    }
                }
            }
            _ => {
                let body = with_xhr(self.id, |s| {
                    s.reply.as_ref().map(|r| r.body.clone()).unwrap_or_default()
                })
                .unwrap_or_default();
                let buf = rquickjs::ArrayBuffer::new(ctx.clone(), body)?;
                buf.into_js(&ctx)
            }
        }
    }

    #[qjs(get, rename = "responseType")]
    pub fn response_type<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let kind = with_xhr(self.id, |s| s.response_kind).unwrap_or(RESP_KIND_PLAIN);
        let s = match kind {
            RESP_KIND_TEXT => "text",
            RESP_KIND_JSON => "json",
            _ => "",
        };
        s.into_js(&ctx)
    }

    #[qjs(set, rename = "responseType")]
    pub fn set_response_type<'js>(&mut self, ctx: Ctx<'js>, v: Value<'js>) -> rquickjs::Result<()> {
        let c = crate::webidl::value_to_str(&v);
        let s = c.as_deref().unwrap_or("");
        let kind = match s {
            "" => RESP_KIND_PLAIN,
            "text" => RESP_KIND_TEXT,
            "json" => RESP_KIND_JSON,
            "arraybuffer" | "blob" => RESP_KIND_PLAIN,
            _ => {
                return Err(rquickjs::Exception::throw_message(
                    &ctx,
                    "SyntaxError: The provided value '' is not a valid enum value",
                ));
            }
        };
        let sent = with_xhr(self.id, |s| {
            if s.sent {
                return true;
            }
            s.response_kind = kind;
            false
        })
        .unwrap_or(true);
        if sent {
            return Err(throw_invalid_state(&ctx));
        }
        Ok(())
    }

    #[qjs(get, rename = "timeout")]
    pub fn timeout(&self) -> u32 {
        with_xhr(self.id, |s| s.timeout_ms as u32).unwrap_or(0)
    }

    #[qjs(set, rename = "timeout")]
    pub fn set_timeout(&mut self, v: f64) {
        with_xhr(self.id, |s| s.timeout_ms = v.max(0.0));
    }

    #[qjs(get, rename = "withCredentials")]
    pub fn with_credentials(&self) -> bool {
        with_xhr(self.id, |s| s.with_credentials).unwrap_or(false)
    }

    #[qjs(set, rename = "withCredentials")]
    pub fn set_with_credentials<'js>(&mut self, ctx: Ctx<'js>, v: f64) -> rquickjs::Result<()> {
        let reject = with_xhr(self.id, |s| {
            if s.sent && v != 0.0 {
                return true;
            }
            s.with_credentials = v != 0.0;
            false
        })
        .unwrap_or(false);
        if reject {
            return Err(throw_invalid_state(&ctx));
        }
        Ok(())
    }

    #[qjs(set, rename = "onreadystatechange")]
    pub fn set_onreadystatechange<'js>(&mut self, ctx: Ctx<'js>, v: Value<'js>) {
        touch::touch_log_record(ApiKey::XHR);
        reg_set_handler(&XHR_REG, self.id, &ctx, v, |s| &mut s.onreadystatechange);
    }

    #[qjs(get, rename = "onreadystatechange")]
    pub fn onreadystatechange<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        reg_handler_value(&XHR_REG, &ctx, self.id, |s| &s.onreadystatechange)
    }

    #[qjs(set, rename = "onload")]
    pub fn set_onload<'js>(&mut self, ctx: Ctx<'js>, v: Value<'js>) {
        touch::touch_log_record(ApiKey::XHR);
        reg_set_handler(&XHR_REG, self.id, &ctx, v, |s| &mut s.onload);
    }

    #[qjs(get, rename = "onload")]
    pub fn onload<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        reg_handler_value(&XHR_REG, &ctx, self.id, |s| &s.onload)
    }

    #[qjs(set, rename = "onerror")]
    pub fn set_onerror<'js>(&mut self, ctx: Ctx<'js>, v: Value<'js>) {
        touch::touch_log_record(ApiKey::XHR);
        reg_set_handler(&XHR_REG, self.id, &ctx, v, |s| &mut s.onerror);
    }

    #[qjs(get, rename = "onerror")]
    pub fn onerror<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        reg_handler_value(&XHR_REG, &ctx, self.id, |s| &s.onerror)
    }

    #[qjs(set, rename = "onloadend")]
    pub fn set_onloadend<'js>(&mut self, ctx: Ctx<'js>, v: Value<'js>) {
        touch::touch_log_record(ApiKey::XHR);
        reg_set_handler(&XHR_REG, self.id, &ctx, v, |s| &mut s.onloadend);
    }

    #[qjs(get, rename = "onloadend")]
    pub fn onloadend<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        reg_handler_value(&XHR_REG, &ctx, self.id, |s| &s.onloadend)
    }

    #[qjs(set, rename = "onabort")]
    pub fn set_onabort<'js>(&mut self, ctx: Ctx<'js>, v: Value<'js>) {
        touch::touch_log_record(ApiKey::XHR);
        reg_set_handler(&XHR_REG, self.id, &ctx, v, |s| &mut s.onabort);
    }

    #[qjs(get, rename = "onabort")]
    pub fn onabort<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        reg_handler_value(&XHR_REG, &ctx, self.id, |s| &s.onabort)
    }

    pub fn open<'js>(
        &mut self,
        ctx: Ctx<'js>,
        method: rquickjs::String<'js>,
        url: rquickjs::String<'js>,
        _rest: Rest<Value<'js>>,
    ) -> rquickjs::Result<()> {
        touch::touch_log_record(ApiKey::XHR);
        let mc = method.to_cstring()?;
        let m: &str = mc.as_str();
        if m.is_empty() {
            return Err(throw_xhr_syntax(&ctx));
        }
        let upper = core_utils::ascii_upper_compact(m);
        let upper: &str = upper.as_str();
        match upper {
            "GET" | "HEAD" | "POST" | "PUT" | "DELETE" | "OPTIONS" | "PATCH" | "TRACE"
            | "CONNECT" => {}
            _ => {
                return Err(throw_xhr_syntax(&ctx));
            }
        }
        let uc = url.to_cstring()?;
        let u: &str = uc.as_str();
        with_xhr(self.id, |s| {
            s.method = CompactString::new(upper);
            s.url = CompactString::new(u);
            s.req_headers.clear();
            s.sent = false;
            s.aborted = false;
            s.completed = false;
            s.reply = None;
            s.ready = 1;
        });
        fire_ready_change(&ctx, self.id);
        Ok(())
    }

    pub fn set_request_header<'js>(
        &mut self,
        ctx: Ctx<'js>,
        name: rquickjs::String<'js>,
        value: rquickjs::String<'js>,
    ) -> rquickjs::Result<()> {
        touch::touch_log_record(ApiKey::XHR);
        let (ready, sent) = with_xhr(self.id, |s| (s.ready, s.sent)).unwrap_or((0, true));
        if ready == 0 || sent {
            return Err(throw_invalid_state(&ctx));
        }
        let nc = name.to_cstring()?;
        let vc = value.to_cstring()?;
        let n: &str = nc.as_str();
        let v: &str = vc.as_str();
        if n.is_empty()
            || !n.bytes().all(|b| b.is_ascii_graphic())
            || v.bytes().any(|b| !b.is_ascii_graphic() && b != b' ')
        {
            return Err(throw_xhr_syntax(&ctx));
        }
        if XHR_BANNED_HEADERS.iter().any(|b| n.eq_ignore_ascii_case(b)) {
            return Ok(());
        }
        with_xhr(self.id, |s| {
            if s.req_headers.len() < XHR_HDR_CAP {
                s.req_headers
                    .push((CompactString::new(n), CompactString::new(v)));
            }
        });
        Ok(())
    }

    pub fn get_response_header<'js>(
        &self,
        ctx: Ctx<'js>,
        name: rquickjs::String<'js>,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::XHR);
        let done = with_xhr(self.id, |s| s.completed).unwrap_or(false);
        if !done {
            return Ok(Value::new_null(ctx));
        }
        let ncs = name.to_cstring()?;
        let v = with_xhr(self.id, |s| {
            s.reply
                .as_ref()
                .and_then(|r| r.header_find(ncs.as_str()))
                .map(|hv| hv.as_str().into_js(&ctx))
        })
        .flatten();
        match v {
            Some(x) => x,
            None => Ok(Value::new_null(ctx)),
        }
    }

    pub fn get_all_response_headers<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::XHR);
        let done = with_xhr(self.id, |s| s.completed).unwrap_or(false);
        if !done {
            return "".into_js(&ctx);
        }
        let mut out: SmallVec<[u8; 512]> = SmallVec::new();
        with_xhr(self.id, |s| {
            if let Some(r) = s.reply.as_ref() {
                for (k, v) in &r.headers {
                    out.extend_from_slice(k.as_bytes());
                    out.extend_from_slice(b": ");
                    out.extend_from_slice(v.as_bytes());
                    out.extend_from_slice(b"\r\n");
                }
            }
        });
        if core_utils::utf8::basic::from_utf8(out.as_slice()).is_ok() {
            return unsafe { std::str::from_utf8_unchecked(out.as_slice()) }.into_js(&ctx);
        }
        String::from_utf8_lossy(out.as_slice())
            .as_ref()
            .into_js(&ctx)
    }
    pub fn send<'js>(&mut self, ctx: Ctx<'js>, rest: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let body: Option<Bytes> = rest.first().and_then(|v| js_value_to_bytes(v));
        touch::touch_log_record(ApiKey::XHR);
        let (ready, sent) =
            with_xhr(self.id, |s| (s.ready, s.sent)).unwrap_or((0, false));
        if ready == 0 {
            return Err(throw_invalid_state(&ctx));
        }
        if sent {
            return Err(throw_invalid_state(&ctx));
        }
        let (method, url, headers, cookie) = with_xhr(self.id, |s| {
            s.sent = true;
            (
                std::mem::take(&mut s.method),
                std::mem::take(&mut s.url),
                std::mem::take(&mut s.req_headers),
                with_prof(|p| p.cookie.clone()),
            )
        })
        .unwrap_or((
            CompactString::const_new(""),
            CompactString::const_new(""),
            SmallVec::new(),
            CompactString::const_new(""),
        ));
        if url.is_empty() {
            return Err(throw_xhr_syntax(&ctx));
        }
        let reply = if fetch_bridge::installed() {
            bridge_fetch(url.as_str(), method.as_str(), headers, body, cookie).map(|mut r| {
                let ok = r.ok();
                let reply = Reply {
                    status: r.status,
                    headers: std::mem::take(&mut r.headers),
                    body: std::mem::take(&mut r.body),
                    ok,
                    url,
                    text: None,
                };
                reply
            })
        } else {
            Some(Reply::offline_204())
        };
        let failed = reply.is_none();
        with_xhr(self.id, |s| {
            s.reply = reply;
            s.completed = !failed;
        });
        let id = self.id;
        timer::defer(&ctx, move |c| {
            if failed {
                fire_failure(c, id);
            } else {
                fire_completion(c, id);
            }
        })?;
        Ok(())
    }

    pub fn abort<'js>(&mut self, ctx: Ctx<'js>) -> rquickjs::Result<()> {
        touch::touch_log_record(ApiKey::XHR);
        let sent = with_xhr(self.id, |s| s.sent).unwrap_or(false);
        with_xhr(self.id, |s| {
            if s.aborted {
                return;
            }
            s.aborted = true;
            s.reply = None;
            s.completed = false;
            s.ready = 0;
        });
        if sent {
            let id = self.id;
            timer::defer(&ctx, move |c| fire_abort(c, id))?;
        } else {
            XHR_REG.with(|m| {
                m.borrow_mut().remove(self.id);
            });
            SELF_OBJ.with(|m| {
                m.borrow_mut().remove(&self.id);
            });
        }
        Ok(())
    }

    pub fn add_event_listener<'js>(
        &mut self,
        ctx: Ctx<'js>,
        kind: rquickjs::String<'js>,
        f: Value<'js>,
    ) -> rquickjs::Result<()> {
        touch::touch_log_record(ApiKey::XHR);
        if let Some(fun) = f.as_function().cloned() {
            let k = CompactString::new(kind.to_cstring()?.as_str());
            with_xhr(self.id, |s| {
                if s.listeners.len() < XHR_LISTENER_CAP {
                    s.listeners.push((k, Persistent::save(&ctx, fun)));
                }
            });
        }
        Ok(())
    }

    pub fn remove_event_listener<'js>(
        &mut self,
        ctx: Ctx<'js>,
        kind: rquickjs::String<'js>,
        f: Value<'js>,
    ) -> rquickjs::Result<()> {
        let kc = kind.to_cstring()?;
        let k: &str = kc.as_str();
        with_xhr(self.id, |s| {
            s.listeners.retain(|(t, p)| {
                t.as_str() != k
                    || p.clone()
                        .restore(&ctx)
                        .map(|stored| !crate::webidl::eq_thunk_same(&ctx, &stored, &f))
                        .unwrap_or(true)
            });
        });
        Ok(())
    }
}

pub(crate) fn bridge_fetch(
    url: &str,
    method: &str,
    headers: crate::task::HeaderList,
    body: Option<Bytes>,
    cookie: CompactString,
) -> Option<crate::task::FetchReply> {
    let reply = fetch_bridge::dispatch(
        url,
        method,
        headers,
        body,
        cookie,
        crate::worker::net_slot(),
        fetch_bridge::timeout_budget(),
    );
    if let Some(r) = reply.as_ref() {
        r.ingest_cookies();
    }
    reply
}

pub(crate) use crate::webidl::value_to_bytes as js_value_to_bytes;

type RegMap<S> = std::thread::LocalKey<RegCell<S>>;

fn reg_set_handler<'js, S>(
    reg: &'static RegMap<S>,
    id: u64,
    ctx: &Ctx<'js>,
    v: Value<'js>,
    slot: fn(&mut S) -> &mut Option<Persistent<Function<'static>>>,
) {
    let f = v.as_function().cloned().map(|f| Persistent::save(ctx, f));
    reg.with(|m| m.borrow_mut().with(id, |s| *slot(s) = f));
}

fn reg_handler_value<'js, S>(
    reg: &'static RegMap<S>,
    ctx: &Ctx<'js>,
    id: u64,
    get: fn(&S) -> &Option<Persistent<Function<'static>>>,
) -> rquickjs::Result<Value<'js>> {
    let stored = reg.with(|m| m.borrow().get_ro(id, |s| get(s).clone()));
    match stored.flatten().and_then(|p| p.restore(ctx).ok()) {
        Some(f) => f.into_value().into_js(ctx),
        None => Ok(Value::new_null(ctx.clone())),
    }
}

fn event_obj<'js>(
    ctx: &Ctx<'js>,
    id: u64,
    kind: &str,
    loaded: u64,
) -> rquickjs::Result<Object<'js>> {
    let ev = Object::new(ctx.clone())?;
    if let Some(target) = self_obj(ctx, id) {
        ev.set("target", target.clone())?;
        ev.set("currentTarget", target)?;
    }
    ev.set("type", kind)?;
    ev.set("lengthComputable", true)?;
    ev.set("loaded", loaded as f64)?;
    ev.set("total", loaded as f64)?;
    ev.prop(
        "isTrusted",
        rquickjs::object::Property::from(true)
            .writable()
            .enumerable(),
    )?;
    ev.set("bubbles", false)?;
    ev.set("cancelable", false)?;
    ev.set("composed", false)?;
    Ok(ev)
}

fn call_all<'js>(
    ctx: &Ctx<'js>,
    id: u64,
    kind: &str,
    handler: fn(&mut XhrState) -> &mut Option<Persistent<Function<'static>>>,
    loaded: u64,
) {
    let own = with_xhr(id, |s| handler(s).clone()).flatten();
    let listeners: SmallVec<[Persistent<Function<'static>>; 4]> = with_xhr(id, |s| {
        s.listeners
            .iter()
            .filter(|(t, _)| t.eq_ignore_ascii_case(kind))
            .map(|(_, p)| p.clone())
            .collect()
    })
    .unwrap_or_default();
    let ev = match event_obj(ctx, id, kind, loaded) {
        Ok(e) => e,
        Err(_) => return,
    };
    let ev_v: Value = ev.into_value();
    if let Some(f) = own.and_then(|p| p.restore(ctx).ok()) {
        let _: rquickjs::Result<Value> = f.call((ev_v.clone(),));
    }
    for l in listeners {
        if let Ok(f) = l.restore(ctx) {
            let _: rquickjs::Result<Value> = f.call((ev_v.clone(),));
        }
    }
}

fn fire_ready_change(ctx: &Ctx<'_>, id: u64) {
    call_all(
        ctx,
        id,
        "readystatechange",
        |s| &mut s.onreadystatechange,
        0,
    );
}

fn fire_completion(ctx: &Ctx<'_>, id: u64) {
    let (loaded, ok) = with_xhr(id, |s| {
        let len = s.reply.as_ref().map(|r| r.body.len()).unwrap_or(0) as u64;
        let ok = s.reply.as_ref().map(|r| r.ok).unwrap_or(false);
        s.ready = 4;
        (len, ok)
    })
    .unwrap_or((0, false));
    fire_ready_change(ctx, id);
    if ok {
        call_all(ctx, id, "load", |s| &mut s.onload, loaded);
    } else {
        call_all(ctx, id, "error", |s| &mut s.onerror, loaded);
    }
    call_all(ctx, id, "loadend", |s| &mut s.onloadend, loaded);
}

fn fire_failure(ctx: &Ctx<'_>, id: u64) {
    with_xhr(id, |s| {
        s.ready = 4;
        s.completed = false;
    });
    fire_ready_change(ctx, id);
    call_all(ctx, id, "error", |s| &mut s.onerror, 0);
    call_all(ctx, id, "loadend", |s| &mut s.onloadend, 0);
}

fn fire_abort(ctx: &Ctx<'_>, id: u64) {
    with_xhr(id, |s| s.ready = 0);
    call_all(ctx, id, "abort", |s| &mut s.onabort, 0);
    call_all(ctx, id, "loadend", |s| &mut s.onloadend, 0);
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "RTCPeerConnection")]
pub(crate) struct Rtc {
    #[qjs(skip_trace)]
    id: u64,
}

fn rtc_seed(id: u64) -> u64 {
    with_prof(|p| {
        let seed = p.seed;
        let origin = xxh3::hash_seeded_tail(seed, p.origin.as_bytes());
        mix_ctx(seed ^ origin, id)
    })
}

fn ice_token(seed: u64, salt: u64, len: usize) -> CompactString {
    use core_utils::rng::SplitMix64Rng;
    let mut rng = SplitMix64Rng::new(core_utils::rng::mix64(seed ^ salt));
    let mut out = CompactString::with_capacity(len);
    for _ in 0..len {
        let x = rng.next_u64();
        out.push(ICE_CHARS[(x >> 33) as usize % ICE_CHARS.len()] as char);
    }
    out
}

fn mdns_host(seed: u64) -> CompactString {
    let bytes = core_utils::rng::SplitMix64Rng::new(seed ^ seeds::SALT_INBOUND)
        .next_u64()
        .to_le_bytes();
    let mut out = core_utils::hex_grouped(&bytes, false, '-', &[8, 4, 4]);
    out.push_str(".local");
    out
}

const DTLS_GROUPS: [usize; 32] = [2; 32];

fn dtls_fingerprint(seed: u64) -> CompactString {
    let d = with_prof(|p| sha256_seed_tail(seed, p.origin.as_bytes()));
    core_utils::hex_grouped(&d, true, ':', &DTLS_GROUPS)
}

fn build_sdp(seed: u64) -> CompactString {
    let ufrag = ice_token(seed, ICE_UFRAG_SALT, 8);
    let pwd = ice_token(seed, ICE_PWD_SALT, 24);
    let fp = dtls_fingerprint(seed);
    let session = (seed ^ seeds::SALT_PLACEMENT) % 9_000_000_000_000_000;
    let mut s = CompactString::with_capacity(512);
    s.push_str("v=0\r\n");
    s.push_str("o=- ");
    core_utils::push_int_into(&mut s, session as i64);
    s.push_str(" 2 IN IP4 127.0.0.1\r\n");
    s.push_str("s=-\r\n");
    s.push_str("t=0 0\r\n");
    s.push_str("a=group:BUNDLE 0\r\n");
    s.push_str("a=msid-semantic: WMS\r\n");
    s.push_str("m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n");
    s.push_str("c=IN IP4 0.0.0.0\r\n");
    s.push_str("a=ice-ufrag:");
    s.push_str(ufrag.as_str());
    s.push_str("\r\na=ice-pwd:");
    s.push_str(pwd.as_str());
    s.push_str("\r\na=ice-options:trickle\r\na=fingerprint:sha-256 ");
    s.push_str(fp.as_str());
    s.push_str(
        "\r\na=setup:actpass\r\na=mid:0\r\na=sctp-port:5000\r\na=max-message-size:262144\r\n",
    );
    s
}

fn ice_candidate(seed: u64) -> CompactString {
    let host = mdns_host(seed);
    let port = 10_000 + (seed % 55_000) as u32;
    let mut s = CompactString::with_capacity(160);
    s.push_str("candidate:");
    core_utils::push_int_into(&mut s, (1 + seed % 8) as i64);
    s.push_str(" 1 UDP ");
    core_utils::push_int_into(&mut s, (2_122_250_000 + seed % 100_000) as i64);
    s.push(' ');
    s.push_str(host.as_str());
    s.push(' ');
    core_utils::push_int_into(&mut s, port as i64);
    s.push_str(" typ host");
    s
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Rtc {
    #[qjs(get, rename = "connectionState")]
    pub fn connection_state(&self) -> &'static str {
        with_rtc(self.id, |s| s.connection_state).unwrap_or("new")
    }

    #[qjs(get, rename = "iceConnectionState")]
    pub fn ice_connection_state(&self) -> &'static str {
        with_rtc(self.id, |s| s.ice_state).unwrap_or("new")
    }

    #[qjs(get, rename = "signalingState")]
    pub fn signaling_state(&self) -> &'static str {
        with_rtc(self.id, |s| s.signaling).unwrap_or("stable")
    }

    #[qjs(get, rename = "localDescription")]
    pub fn local_description<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let sdp = with_rtc(self.id, |s| s.local_sdp.clone()).flatten();
        match sdp.as_ref() {
            Some(s) => sdp_obj(&ctx, "offer", s.as_str())?
                .into_value()
                .into_js(&ctx),
            None => Ok(Value::new_null(ctx)),
        }
    }

    #[qjs(set, rename = "onicecandidate")]
    pub fn set_onicecandidate<'js>(&mut self, ctx: Ctx<'js>, v: Value<'js>) {
        touch::touch_log_record(ApiKey::WEBRTC);
        reg_set_handler(&RTC_REG, self.id, &ctx, v, |s| &mut s.onicecandidate);
    }

    #[qjs(get, rename = "onicecandidate")]
    pub fn onicecandidate<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        reg_handler_value(&RTC_REG, &ctx, self.id, |s| &s.onicecandidate)
    }

    pub fn create_data_channel<'js>(
        &mut self,
        ctx: Ctx<'js>,
        label: rquickjs::String<'js>,
        _rest: Rest<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::WEBRTC);
        let idx = with_rtc(self.id, |s| {
            s.data_channels += 1;
            s.data_channels
        })
        .unwrap_or(1);
        let o = Object::new(ctx.clone())?;
        o.set("label", label)?;
        o.set("id", idx)?;
        o.set("ordered", true)?;
        o.set("maxRetransmits", -1i32)?;
        o.set("readyState", "connecting")?;
        o.set("bufferedAmount", 0)?;
        let send = Function::new(
            ctx.clone(),
            |c: Ctx<'js>, _data: Value<'js>| -> Value<'js> { Value::new_undefined(c) },
        )?;
        crate::stackfmt::set_fn_name(&ctx, &send, "send")?;
        o.set("send", send)?;
        let close = Function::new(ctx.clone(), |c: Ctx<'js>| -> Value<'js> {
            Value::new_undefined(c)
        })?;
        crate::stackfmt::set_fn_name(&ctx, &close, "close")?;
        o.set("close", close)?;
        o.into_value().into_js(&ctx)
    }

    pub fn create_offer<'js>(
        &mut self,
        ctx: Ctx<'js>,
        _rest: Rest<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::WEBRTC);
        let seed = with_rtc(self.id, |s| s.seed).unwrap_or(0);
        let sdp = build_sdp(seed);
        let o = sdp_obj(&ctx, "offer", sdp.as_str())?;
        with_rtc(self.id, |s| {
            s.local_sdp = Some(sdp);
            s.signaling = "have-local-offer";
        });
        let id = self.id;
        timer::defer(&ctx, move |c| trickle_ice(c, id))?;
        promise_resolve(&ctx, o.into_value())
    }

    pub fn create_answer<'js>(
        &mut self,
        ctx: Ctx<'js>,
        _rest: Rest<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::WEBRTC);
        let seed = with_rtc(self.id, |s| s.seed).unwrap_or(0);
        let sdp = build_sdp(seed ^ seeds::SALT_HOP);
        let o = sdp_obj(&ctx, "answer", sdp.as_str())?;
        with_rtc(self.id, |s| {
            s.local_sdp = Some(sdp);
        });
        promise_resolve(&ctx, o.into_value())
    }

    pub fn set_local_description<'js>(
        &mut self,
        ctx: Ctx<'js>,
        _desc: Option<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::WEBRTC);
        with_rtc(self.id, |s| {
            s.connection_state = "connecting";
            s.ice_state = "checking";
        });
        promise_resolve(&ctx, Value::new_undefined(ctx.clone()))
    }

    pub fn set_remote_description<'js>(
        &mut self,
        ctx: Ctx<'js>,
        _desc: Option<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::WEBRTC);
        promise_resolve(&ctx, Value::new_undefined(ctx.clone()))
    }

    pub fn add_ice_candidate<'js>(
        &mut self,
        ctx: Ctx<'js>,
        _cand: Option<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::WEBRTC);
        promise_resolve(&ctx, Value::new_undefined(ctx.clone()))
    }

    pub fn get_stats<'js>(&mut self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::WEBRTC);
        let m = new_map(&ctx)?;
        promise_resolve(&ctx, m.into_value())
    }

    pub fn close(&mut self) -> rquickjs::Result<()> {
        touch::touch_log_record(ApiKey::WEBRTC);
        with_rtc(self.id, |s| {
            s.connection_state = "closed";
            s.ice_state = "closed";
            s.signaling = "closed";
        });
        RTC_REG.with(|m| {
            m.borrow_mut().remove(self.id);
        });
        SELF_OBJ.with(|m| {
            m.borrow_mut().remove(&self.id);
        });
        Ok(())
    }

    pub fn add_event_listener<'js>(
        &mut self,
        _kind: rquickjs::String<'js>,
        _f: Value<'js>,
    ) -> rquickjs::Result<()> {
        touch::touch_log_record(ApiKey::WEBRTC);
        Ok(())
    }

    pub fn remove_event_listener<'js>(
        &mut self,
        _kind: rquickjs::String<'js>,
        _f: Value<'js>,
    ) -> rquickjs::Result<()> {
        Ok(())
    }
}

fn trickle_ice(ctx: &Ctx<'_>, id: u64) {
    let seed = with_rtc(id, |s| s.seed).unwrap_or(0);
    let stored = with_rtc(id, |s| s.onicecandidate.clone()).flatten();
    let Some(f) = stored.and_then(|p| p.restore(ctx).ok()) else {
        return;
    };
    let cand = ice_candidate(seed);
    let ufrag = ice_token(seed, ICE_UFRAG_SALT, 8);
    let Ok(ev) = Object::new(ctx.clone()) else {
        return;
    };
    let Ok(inner) = Object::new(ctx.clone()) else {
        return;
    };
    let _ = inner.set("candidate", cand.as_str());
    let _ = inner.set("sdpMid", "0");
    let _ = inner.set("sdpMLineIndex", 0u32);
    let _ = inner.set("usernameFragment", ufrag.as_str());
    let _ = ev.set("type", "icecandidate");
    if let Some(t) = self_obj(ctx, id) {
        let _ = ev.set("target", t);
    }
    let _ = ev.set("candidate", inner);
    let v: Value = ev.into_value();
    let _: rquickjs::Result<Value> = f.call((v,));
    let Ok(done) = Object::new(ctx.clone()) else {
        with_rtc(id, |s| {
            s.ice_state = "connected";
            s.connection_state = "connected";
        });
        return;
    };
    let _ = done.set("type", "icecandidate");
    let _ = done.set("candidate", Value::new_null(ctx.clone()));
    let dv: Value = done.into_value();
    let _: rquickjs::Result<Value> = f.call((dv,));
    with_rtc(id, |s| {
        s.ice_state = "connected";
        s.connection_state = "connected";
    });
}

fn reg_class_ctor<'js, S, C>(
    ctx: &Ctx<'js>,
    reg: &'static RegMap<S>,
    cap: usize,
    err: &'static str,
    api: u32,
    fresh: fn(u64) -> S,
    make: fn(Ctx<'js>, u64) -> rquickjs::Result<Class<'js, C>>,
    seed_of: fn(u64) -> u64,
) -> rquickjs::Result<Function<'js>>
where
    S: 'static,
    C: rquickjs::class::JsClass<'js> + 'js,
{
    Ok(Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, _rest: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            crate::webidl::registry_ctor(
                &c,
                reg,
                cap,
                err,
                api,
                |id| fresh(seed_of(id)),
                |cc, id| {
                    let class = make(cc.clone(), id)?;
                    let obj: Object = class.into_inner();
                    save_self(cc, id, obj.clone());
                    Ok(obj.into_value())
                },
            )
        },
    )?
    .with_constructor(true))
}

pub(crate) fn install<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let xhr_ctor = reg_class_ctor(
        ctx,
        &XHR_REG,
        XHR_CAP,
        "InternalError: xhr registry saturated",
        ApiKey::XHR,
        |_| XhrState::fresh(),
        |c, id| Class::instance(c, Xhr { id }),
        |id| id as u64,
    )?;
    let xhr_proto = Class::<Xhr>::prototype(ctx)?
        .ok_or_else(|| rquickjs::Exception::throw_message(ctx, "InternalError: xhr prototype"))?;
    install_host_ctor(ctx, &xhr_ctor, &xhr_proto, "XMLHttpRequest", 0, true)?;

    let rtc_ctor = reg_class_ctor(
        ctx,
        &RTC_REG,
        RTC_CAP,
        "InternalError: rtc registry saturated",
        ApiKey::WEBRTC,
        RtcState::fresh,
        |c, id| Class::instance(c, Rtc { id }),
        rtc_seed,
    )?;
    let rtc_proto = Class::<Rtc>::prototype(ctx)?
        .ok_or_else(|| rquickjs::Exception::throw_message(ctx, "InternalError: rtc prototype"))?;
    install_host_ctor(ctx, &rtc_ctor, &rtc_proto, "RTCPeerConnection", 0, true)?;
    let webkit_alias: Option<Object> = globals.get("RTCPeerConnection").ok().flatten();
    if let Some(alias) = webkit_alias {
        globals.prop(
            "webkitRTCPeerConnection",
            rquickjs::object::Property::from(alias)
                .writable()
                .configurable(),
        )?;
    }
    Ok(())
}

fn throw_invalid_state(ctx: &Ctx<'_>) -> rquickjs::Error {
    rquickjs::Exception::throw_message(ctx, "InvalidStateError: The object is in an invalid state")
}

fn throw_xhr_syntax(ctx: &Ctx<'_>) -> rquickjs::Error {
    rquickjs::Exception::throw_message(
        ctx,
        "SyntaxError: An invalid or illegal string was specified",
    )
}

fn sdp_obj<'js>(ctx: &Ctx<'js>, kind: &'static str, sdp: &str) -> rquickjs::Result<Object<'js>> {
    let o = Object::new(ctx.clone())?;
    o.set("type", kind)?;
    o.set("sdp", sdp)?;
    Ok(o)
}
