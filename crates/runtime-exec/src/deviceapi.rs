use crate::pageapi::COMMON_FEATURES;
use crate::task as fetch_bridge;
use core_utils::BytesExt as _;
use core_utils::StrExt as _;
use crate::touch::{self, ApiKey};
use crate::webidl::class_tag;
use crate::worker::{promise_resolve, prof_host, prof_origin, prof_seed, with_prof};
use bytes::Bytes;
use compact_str::CompactString;
use core_utils::rng::mix_ctx;
use core_utils::xxh3;
use rquickjs::function::Rest;
use rquickjs::function::This;
use rquickjs::{Ctx, Function, IntoJs, Object, Persistent, Value};
use smallvec::SmallVec;
use std::cell::RefCell;

const BEACON_MAX: usize = 65_536;

struct BatteryState {
    charging: bool,
    charging_time: f64,
    discharging_time: f64,
    level: f64,
}

thread_local! {
    static BAT_STATE: RefCell<Option<BatteryState>> = const { RefCell::new(None) };
    static BAT_OBJ: RefCell<Option<Persistent<Object<'static>>>> = const { RefCell::new(None) };
}

pub(crate) fn clear_registry() {
    BAT_STATE.with(|m| *m.borrow_mut() = None);
    BAT_OBJ.with(|m| *m.borrow_mut() = None);
}

fn hex64(seed: u64, salt: u64) -> CompactString {
    core_utils::hex_compact(&core_utils::sha256_seed_tail(seed, &salt.to_le_bytes()), false)
}

pub(crate) fn stubbed_accessor<'js>(
    ctx: &Ctx<'js>,
    proto: &Object<'js>,
    key: &str,
    obj: Object<'js>,
    touch: Option<u32>,
) -> rquickjs::Result<()> {
    let slot = crate::webidl::store_stub(ctx, obj);
    let g = crate::webidl::stub_getter(ctx, slot, touch)?;
    crate::webidl::named_accessor(ctx, proto, key, g, None)
}

fn install_dom_exception<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let proto = Object::new(ctx.clone())?;
    class_tag(ctx, &proto, "DOMException")?;
    let ctor = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         message: rquickjs::function::Opt<Value<'js>>,
         name: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            let mc = message.0.as_ref().and_then(|v| crate::webidl::value_to_str(v));
            let msg: &str = mc.as_ref().map(|x| x.as_str()).unwrap_or("");
            let ncs = name.0.as_ref().and_then(|v| crate::webidl::value_to_str(v));
            let nm: &str = ncs.as_ref().map(|x| x.as_str()).unwrap_or("Error");
            let proto2: Object = c.eval("DOMException.prototype")?;
            let o = Object::new_proto(c.clone(), Some(&proto2))?;
            class_tag(&c, &o, "DOMException")?;
            o.set("message", msg)?;
            o.set("name", nm)?;
            o.set("code", dom_code(nm))?;
            let to_str = Function::new(
                c.clone(),
                move |c2: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                    let ob = this.0.as_object();
                    let nc = ob
                        .and_then(|o| o.get::<_, Value<'js>>("name").ok())
                        .and_then(|v| crate::webidl::value_to_str(&v));
                    let n: &str = nc.as_ref().map(|x| x.as_str()).unwrap_or("Error");
                    let mc = ob
                        .and_then(|o| o.get::<_, Value<'js>>("message").ok())
                        .and_then(|v| crate::webidl::value_to_str(&v));
                    let m: &str = mc.as_ref().map(|x| x.as_str()).unwrap_or("");
                    let mut out = CompactString::with_capacity(n.len() + m.len() + 2);
                    out.push_str(n);
                    out.push_str(": ");
                    out.push_str(m);
                    out.as_str().into_js(&c2)
                },
            )?;
            crate::stackfmt::set_fn_name(&c, &to_str, "toString")?;
            o.set("toString", to_str)?;
            let v: Value = o.into_value();
            Ok(v)
        },
    )?
    .with_constructor(true);
    crate::webidl::install_host_ctor(ctx, &ctor, &proto, "DOMException", 0, true)?;
    Ok(())
}

fn dom_code(name: &str) -> u16 {
    match name {
        "IndexSizeError" => 1,
        "HierarchyRequestError" => 3,
        "WrongDocumentError" => 4,
        "InvalidCharacterError" => 5,
        "NoModificationAllowedError" => 7,
        "NotFoundError" => 8,
        "NotSupportedError" => 9,
        "InUseAttributeError" => 10,
        "InvalidStateError" => 11,
        "SyntaxError" => 12,
        "InvalidModificationError" => 13,
        "NamespaceError" => 14,
        "InvalidAccessError" => 15,
        "TypeMismatchError" => 17,
        "SecurityError" => 18,
        "NetworkError" => 19,
        "AbortError" => 20,
        "URLMismatchError" => 21,
        "QuotaExceededError" => 22,
        "TimeoutError" => 23,
        "InvalidNodeTypeError" => 24,
        "DataCloneError" => 25,
        _ => 0,
    }
}

fn install_media_devices<'js>(ctx: &Ctx<'js>, nav_proto: &Object<'js>) -> rquickjs::Result<()> {
    let md = Object::new(ctx.clone())?;
    class_tag(ctx, &md, "MediaDevices")?;

    let enumerate = Function::new(ctx.clone(), |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::MEDIA_DEVICES);
        let seed = prof_seed();
        let host = prof_host();
        let origin_hash = xxh3::hash_seeded_tail(seed, host.as_bytes());
        let arr = rquickjs::Array::new(c.clone())?;
        let entries: [(&str, &str, u8); 5] = [
            ("default", "audioinput", 0),
            ("communications", "audioinput", 0),
            ("default", "audiooutput", 1),
            ("communications", "audiooutput", 1),
            ("", "videoinput", 2),
        ];
        for (i, (dev, kind, gi)) in entries.iter().enumerate() {
            let o = Object::new(c.clone())?;
            class_tag(&c, &o, "MediaDeviceInfo")?;
            let device_id = if dev.is_empty() {
                hex64(
                    mix_ctx(origin_hash, i as u64),
                    0xA1B2_C3D4,
                )
            } else {
                CompactString::const_new(dev)
            };
            let group = hex64(origin_hash, 0xF00D_BEE0 + *gi as u64);
            o.set("deviceId", device_id.as_str())?;
            o.set("kind", *kind)?;
            o.set("label", "")?;
            o.set("groupId", group.as_str())?;
            let to_json = Function::new(
                c.clone(),
                move |cc: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                    let o = Object::new(cc.clone())?;
                    if let Some(ob) = this.0.as_object() {
                        for k in ["deviceId", "kind", "label", "groupId"] {
                            let v: Option<Value> = ob.get(k).ok().flatten();
                            if let Some(v) = v {
                                o.set(k, v)?;
                            }
                        }
                    }
                    Ok(o.into_value())
                },
            )?;
            crate::webidl::define_method(&c, &o, "toJSON", to_json)?;
            arr.set(i, o)?;
        }
        promise_resolve(&c, arr.into_value())
    })?;
    crate::webidl::define_method(ctx, &md, "enumerateDevices", enumerate)?;

    let gum = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, cs: rquickjs::function::Opt<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::MEDIA_DEVICES);
            let cs = cs.0.unwrap_or_else(|| Value::new_undefined(c.clone()));
            let opts = if cs.is_undefined() || cs.is_null() {
                None
            } else {
                cs.as_object().cloned()
            };
            let Some(opts) = opts else {
                return Err(rquickjs::Exception::throw_type(
                    &c,
                    "Failed to execute 'getUserMedia' on 'MediaDevices': The provided value is not of type '(MediaStreamConstraints or boolean)'.",
                ));
            };
            let audio: Value = opts.get("audio")?;
            let video: Value = opts.get("video")?;
            let a_on = audio.as_bool().unwrap_or(audio.is_object());
            let v_on = video.as_bool().unwrap_or(video.is_object());
            if !a_on && !v_on {
                return Err(rquickjs::Exception::throw_type(
                    &c,
                    "Failed to execute 'getUserMedia' on 'MediaDevices': At least one of audio or video must be requested",
                ));
            }
            let ex = dom_exception_value(&c, "Permission denied", "NotAllowedError")?;
            crate::worker::promise_reject_value(&c, ex)
        },
    )?;
    crate::webidl::define_method(ctx, &md, "getUserMedia", gum)?;

    let gdm = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, _cs: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::MEDIA_DEVICES);
            let ex = dom_exception_value(&c, "Permission denied", "NotAllowedError")?;
            crate::worker::promise_reject_value(&c, ex)
        },
    )?;
    crate::webidl::define_method(ctx, &md, "getDisplayMedia", gdm)?;

    let gsc = Function::new(
        ctx.clone(),
        |_c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::MEDIA_DEVICES);
            let o = Object::new(_c.clone())?;
            let keys = [
                "aspectRatio",
                "autoGainControl",
                "channelCount",
                "deviceId",
                "displaySurface",
                "echoCancellation",
                "facingMode",
                "frameRate",
                "groupId",
                "height",
                "latency",
                "noiseSuppression",
                "resizeMode",
                "width",
            ];
            for k in keys {
                o.set(k, true)?;
            }
            Ok(o.into_value())
        },
    )?;
    crate::webidl::define_method(ctx, &md, "getSupportedConstraints", gsc)?;

    crate::webidl::install_noop_listeners(ctx, &md)?;

    stubbed_accessor(ctx, nav_proto, "mediaDevices", md, Some(ApiKey::MEDIA_DEVICES))?;
    Ok(())
}

fn dom_exception_value<'js>(
    c: &Ctx<'js>,
    message: &str,
    name: &str,
) -> rquickjs::Result<Value<'js>> {
    let ctor: Function = c.globals().get("DOMException")?;
    ctor.call((message, name))
}

const PERM_EXTRA: &[&str] = &[
    "background-sync",
    "notifications",
    "payment-handler",
    "persistent-storage",
    "push",
    "top-level-storage-access",
    "video-capture",
];

fn is_perm_name(name: &str) -> bool {
    COMMON_FEATURES.contains(&name) || PERM_EXTRA.contains(&name)
}

fn perm_state_of(name: &str) -> &'static str {
    match name {
        "accelerometer"
        | "gyroscope"
        | "magnetometer"
        | "ambient-light-sensor"
        | "background-sync"
        | "compute-pressure" => "granted",
        _ => "prompt",
    }
}

fn install_permissions<'js>(ctx: &Ctx<'js>, nav_proto: &Object<'js>) -> rquickjs::Result<()> {
    let perms = Object::new(ctx.clone())?;
    class_tag(ctx, &perms, "Permissions")?;

    let query = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, desc: rquickjs::function::Opt<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::PERMISSIONS);
            let desc = desc.0.unwrap_or_else(|| Value::new_undefined(c.clone()));
            let Some(d) = desc.as_object() else {
                return Err(rquickjs::Exception::throw_type(
                    &c,
                    "Failed to execute 'query' on 'Permissions': The provided value is not of type 'PermissionsDescriptor'.",
                ));
            };
            let name_v: Value = d.get("name")?;
            let nc = crate::webidl::value_to_str(&name_v).ok_or_else(|| {
                rquickjs::Exception::throw_type(
                    &c,
                    "Failed to execute 'query' on 'Permissions': Failed to read the 'name' property from 'PermissionsDescriptor': The provided value is not of type 'PermissionName'.",
                )
            })?;
            let name: &str = nc.as_str();
            if !is_perm_name(name) {
                let mut msg = CompactString::const_new(
                    "Failed to execute 'query' on 'Permissions': The provided value '",
                );
                msg.push_str(name);
                msg.push_str("' is not a valid enum value.");
                return Err(rquickjs::Exception::throw_type(&c, msg.as_str()));
            }
            let state = perm_state_of(name);
            let o = Object::new(c.clone())?;
            class_tag(&c, &o, "PermissionStatus")?;
            o.set("state", state)?;
            o.set("name", name)?;
            o.set("onchange", Value::new_null(c.clone()))?;
            crate::webidl::install_noop_listeners(&c, &o)?;
            let disp = Function::new(c.clone(), |_cc: Ctx<'js>, _k: Value<'js>| -> bool { false })?;
            crate::webidl::define_method(&c, &o, "dispatchEvent", disp)?;
            promise_resolve(&c, o.into_value())
        },
    )?;
    crate::stackfmt::set_fn_len(ctx, &query, 1)?;
    crate::webidl::define_method(ctx, &perms, "query", query)?;

    stubbed_accessor(ctx, nav_proto, "permissions", perms, Some(ApiKey::PERMISSIONS))?;
    Ok(())
}

fn battery_fresh() -> BatteryState {
    let seed = prof_seed();
    let charging = (seed >> 13) & 1 == 0;
    let level = (((seed >> 7) % 101) as f64 / 100.0 * 100.0).round() / 100.0;
    let charging_time = if charging {
        ((seed % 3600) as f64).round()
    } else {
        f64::INFINITY
    };
    let discharging_time = if charging {
        f64::INFINITY
    } else {
        (3600.0 * 2.0 + (seed % 7200) as f64).round()
    };
    BatteryState {
        charging,
        charging_time,
        discharging_time,
        level,
    }
}

fn install_battery<'js>(ctx: &Ctx<'js>, nav_proto: &Object<'js>) -> rquickjs::Result<()> {
    let get_battery = Function::new(ctx.clone(), |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::BATTERY);
        let existing = BAT_OBJ.with(|m| m.borrow().is_some());
        if !existing {
            let bm = Object::new(c.clone())?;
            class_tag(&c, &bm, "BatteryManager")?;
            BAT_STATE.with(|s| *s.borrow_mut() = Some(battery_fresh()));
            crate::webidl::install_noop_listeners(&c, &bm)?;
            BAT_OBJ.with(|m| *m.borrow_mut() = Some(Persistent::save(&c, bm)));
        }
        let Some(bm) = crate::webidl::restore_slot(&c, &BAT_OBJ) else {
            return promise_resolve(&c, Value::new_undefined(c.clone()));
        };
        let (charging, ct, dt, level) = BAT_STATE
            .with(|s| {
                s.borrow()
                    .as_ref()
                    .map(|st| (st.charging, st.charging_time, st.discharging_time, st.level))
            })
            .unwrap_or((true, 0.0, f64::INFINITY, 1.0));
        bm.set("charging", charging)?;
        bm.set("chargingTime", ct)?;
        bm.set("dischargingTime", dt)?;
        bm.set("level", level)?;
        for key in [
            "onchargingchange",
            "onchargingtimechange",
            "ondischargingtimechange",
            "onlevelchange",
        ] {
            bm.set(key, Value::new_null(c.clone()))?;
        }
        promise_resolve(&c, bm.into_value())
    })?;
    crate::webidl::define_method(ctx, nav_proto, "getBattery", get_battery)?;
    Ok(())
}

fn beacon_bytes(v: &Value<'_>) -> Option<(Bytes, Option<&'static str>)> {
    if v.is_string() {
        return crate::webidl::value_to_bytes(v).map(|b| (b, Some("text/plain;charset=UTF-8")));
    }
    if let Some(o) = v.as_object()
        && o.as_array_buffer().is_some()
    {
        let bytes = crate::webidl::value_to_bytes(v)?;
        return Some((bytes, None));
    }
    if let Some(o) = v.as_object() {
        let byte_len: Option<f64> = o.get("byteLength").ok().flatten();
        if let Some(n) = byte_len
            && n.fract() == 0.0
            && (0.0..1e9).contains(&n)
        {
            let view: Option<Value> = o.get("buffer").ok().flatten();
            let off: Option<f64> = o.get("byteOffset").ok().flatten();
            if let Some(bv) = view
                && let Some(ab) = bv.as_object().and_then(|o| o.as_array_buffer())
            {
                let off = off.unwrap_or(0.0) as usize;
                let len = n as usize;
                let bytes = ab.as_bytes()?;
                if off + len <= bytes.len() {
                    return Some((Bytes::copy_from_slice(&bytes[off..off + len]), None));
                }
            }
            return None;
        }
        let append: Option<Function> = o.get("append").ok().flatten();
        let to_string: Option<Function> = o.get("toString").ok().flatten();
        if append.is_some() && to_string.is_some() {
            let s: String = to_string.and_then(|f| f.call(()).ok()).unwrap_or_default();
            return Some((
                Bytes::from(s.into_bytes()),
                Some("application/x-www-form-urlencoded;charset=UTF-8"),
            ));
        }
        if let Some(ts) = to_string {
            let s: String = ts.call(()).unwrap_or_default();
            return Some((
                Bytes::from(s.into_bytes()),
                Some("text/plain;charset=UTF-8"),
            ));
        }
    }
    None
}

fn resolve_beacon_url(url: &str) -> Option<CompactString> {
    if url.is_empty() {
        return None;
    }
    let bytes = url.as_bytes();
    let cut = url.len().min(2048);
    if bytes.starts_with_ci(b"http://") || bytes.starts_with_ci(b"https://") {
        return Some(CompactString::new(&url[..cut]));
    }
    if bytes.starts_with_ci(b"javascript:")
        || bytes.starts_with_ci(b"data:")
        || bytes.starts_with_ci(b"file:")
    {
        return None;
    }
    let origin = prof_origin();
    if origin.is_empty() {
        return None;
    }
    let joined = if url.starts_with('/') || url.starts_with("//") {
        origin.join_origin(&url[..cut], false)
    } else {
        let dir: CompactString = with_prof(|p| {
            let href = p.href.as_str();
            match href.rfind('/') {
                Some(i) => CompactString::new(&href[..i + 1]),
                None => CompactString::const_new("/"),
            }
        });
        let mut rel = dir;
        rel.push_str(url);
        origin.join_origin(&rel, true)
    };
    Some(joined)
}

fn install_send_beacon<'js>(ctx: &Ctx<'js>, nav_proto: &Object<'js>) -> rquickjs::Result<()> {
    let f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         url: rquickjs::function::Opt<Value<'js>>,
         data: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::SEND_BEACON);
            let url = url.0.ok_or_else(|| {
                rquickjs::Exception::throw_type(
                    &c,
                    "Failed to execute 'sendBeacon' on 'Navigator': 1 argument required, but only 0 present.",
                )
            })?;
            let data = data.0.unwrap_or_else(|| Value::new_undefined(c.clone()));
            if url.is_undefined() {
                return Err(rquickjs::Exception::throw_type(
                    &c,
                    "Failed to execute 'sendBeacon' on 'Navigator': 1 argument required, but only 0 present.",
                ));
            }
            if !url.is_string() {
                return Err(rquickjs::Exception::throw_type(
                    &c,
                    "Failed to execute 'sendBeacon' on 'Navigator': The provided value is not of type '(string or URL)'.",
                ));
            }
            let rc = crate::webidl::value_to_str(&url);
            let raw: &str = rc.as_ref().map(|x| x.as_str()).unwrap_or("");
            let Some(full) = resolve_beacon_url(raw) else {
                return Ok(Value::new_bool(c, false));
            };
            let body = if data.is_undefined() || data.is_null() {
                (Bytes::new(), Some("text/plain;charset=UTF-8"))
            } else {
                beacon_bytes(&data).unwrap_or((Bytes::new(), Some("text/plain;charset=UTF-8")))
            };
            if body.0.len() > BEACON_MAX {
                return Ok(Value::new_bool(c, false));
            }
            let mut headers: crate::task::HeaderList = SmallVec::new();
            if let Some(ct) = body.1 {
                headers.push((
                    CompactString::const_new("Content-Type"),
                    CompactString::const_new(ct),
                ));
            }
            if fetch_bridge::installed() {
                crate::netapi::bridge_fetch(fetch_bridge::FetchCtx {
                    url: full.as_str(),
                    method: "POST",
                    headers,
                    body: Some(body.0),
                    cookie: with_prof(|p| p.cookie.clone()),
                    net_slot: crate::worker::net_slot(),
                    timeout: fetch_bridge::timeout_budget(),
                });
            }
            Ok(Value::new_bool(c, true))
        },
    )?;
    crate::stackfmt::set_fn_len(ctx, &f, 1)?;
    crate::webidl::define_method(ctx, nav_proto, "sendBeacon", f)?;
    Ok(())
}

pub(crate) fn install<'js>(ctx: &Ctx<'js>, nav_proto: &Object<'js>) -> rquickjs::Result<()> {
    install_dom_exception(ctx)?;
    install_media_devices(ctx, nav_proto)?;
    install_permissions(ctx, nav_proto)?;
    install_battery(ctx, nav_proto)?;
    install_send_beacon(ctx, nav_proto)?;
    Ok(())
}
