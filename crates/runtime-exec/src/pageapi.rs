use crate::deviceapi::stubbed_accessor;
use crate::touch::{self, ApiKey};
use crate::webidl::class_tag;
use crate::worker::with_prof;
use core_utils::BytesExt as _;
use compact_str::CompactString;
use rquickjs::function::{Rest, This};
use rquickjs::{Ctx, Function, Object, Value};
use smallvec::SmallVec;

pub(crate) const COMMON_FEATURES: &[&str] = &[
    "accelerometer",
    "ambient-light-sensor",
    "camera",
    "clipboard-read",
    "clipboard-sanitized-write",
    "clipboard-write",
    "compute-pressure",
    "geolocation",
    "gyroscope",
    "idle-detection",
    "local-fonts",
    "magnetometer",
    "microphone",
    "midi",
    "screen-wake-lock",
    "shared-storage",
    "speaker-selection",
    "storage-access",
    "window-management",
    "xr-spatial-tracking",
];

const FEATURE_ONLY: &[&str] = &[
    "autoplay",
    "bluetooth",
    "cross-origin-isolated",
    "display-capture",
    "encrypted-media",
    "fullscreen",
    "gamepad",
    "hid",
    "keyboard-map",
    "payment",
    "picture-in-picture",
    "publickey-credentials-create",
    "publickey-credentials-get",
    "serial",
    "shared-array-buffer",
    "sync-xhr",
    "usb",
    "vertical-scroll",
];

const fn cat_features<const N: usize>(a: &[&'static str], b: &[&'static str]) -> [&'static str; N] {
    if a.len() + b.len() != N {
        panic!("feature list length mismatch");
    }
    let mut out = [""; N];
    let mut i = 0;
    while i < a.len() {
        out[i] = a[i];
        i += 1;
    }
    let mut j = 0;
    while j < b.len() {
        out[a.len() + j] = b[j];
        j += 1;
    }
    out
}

const FEATURE_LIST: &[&str] = &cat_features::<38>(COMMON_FEATURES, FEATURE_ONLY);

const FEATURE_DEFAULT_OFF: &[&str] = &["idle-detection", "local-fonts"];

fn family_available(family: &str) -> bool {
    let f = family.trim();
    if f.is_empty() {
        return false;
    }
    let mut cleaned: SmallVec<[u8; 64]> = SmallVec::new();
    let mut in_quote = false;
    for ch in f.chars() {
        match ch {
            '\'' | '"' => in_quote = !in_quote,
            _ => {
                let mut buf = [0u8; 4];
                cleaned.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    let cleaned = cleaned.as_slice();
    let mut family_buf: SmallVec<[u8; 64]> = SmallVec::new();
    let mut start = 0usize;
    for (i, &b) in cleaned.iter().enumerate() {
        if b.is_ascii_whitespace() {
            if i > start {
                push_font_token(&mut family_buf, &cleaned[start..i]);
            }
            start = i + 1;
        }
    }
    if start < cleaned.len() {
        push_font_token(&mut family_buf, &cleaned[start..]);
    }
    let family = family_buf.as_slice();
    if family.is_empty() {
        return false;
    }
    if payload_gen::GENERIC_FONTS
        .iter()
        .any(|g| g.as_bytes().eq_ignore_ascii_case(family))
    {
        return true;
    }
    let list = platform_fonts();
    list.iter()
        .any(|x| x.as_bytes().eq_ignore_ascii_case(family))
}

fn platform_fonts() -> &'static [&'static str] {
    payload_gen::platform_fonts(with_prof(|p| p.prof().platform.as_str()))
}

#[inline]
fn ends_with_ci_num(hay: &[u8], suffix: &[u8]) -> bool {
    hay.ends_with_ci(suffix)
        && hay[..hay.len() - suffix.len()]
            .iter()
            .all(|&b| b.is_ascii_digit() || b == b'.')
}

#[inline]
fn push_font_token(out: &mut SmallVec<[u8; 64]>, tok: &[u8]) {
    let lower = SmallVec::<[u8; 32]>::from_iter(tok.iter().map(|&b| core_utils::ascii_lower_byte(b)));
    let lower = lower.as_slice();
    let is_size = ends_with_ci_num(lower, b"px")
        || ends_with_ci_num(lower, b"pt")
        || ends_with_ci_num(lower, b"em")
        || ends_with_ci_num(lower, b"rem")
        || ends_with_ci_num(lower, b"%")
        || lower == b"x-small"
        || lower == b"xx-small"
        || lower == b"x-large"
        || lower == b"xx-large";
    let is_style = lower.eq_ignore_ascii_case(b"bold")
        || lower.eq_ignore_ascii_case(b"italic")
        || lower.eq_ignore_ascii_case(b"oblique")
        || lower.eq_ignore_ascii_case(b"normal")
        || lower.eq_ignore_ascii_case(b"small-caps")
        || lower.eq_ignore_ascii_case(b"bolder")
        || lower.eq_ignore_ascii_case(b"lighter")
        || lower.eq_ignore_ascii_case(b"medium")
        || lower.eq_ignore_ascii_case(b"larger")
        || lower.eq_ignore_ascii_case(b"smaller")
        || lower.eq_ignore_ascii_case(b"large")
        || lower.eq_ignore_ascii_case(b"small")
        || (lower.len() <= 3 && lower.iter().all(|&b| b.is_ascii_digit()));
    if is_size || is_style {
        return;
    }
    if !out.is_empty() {
        out.push(b' ');
    }
    out.extend_from_slice(tok);
}

fn push_family(out: &mut SmallVec<[CompactString; 4]>, cur: &[u8]) {
    if cur.iter().any(|&b| !b.is_ascii_whitespace()) && out.len() < 4 {
        let trimmed = cur.trim_ascii_extra(b"");
        out.push(CompactString::new(unsafe { std::str::from_utf8_unchecked(trimmed) }));
    }
}

fn families_of_font_shorthand(shorthand: &str) -> SmallVec<[CompactString; 4]> {
    let mut out: SmallVec<[CompactString; 4]> = SmallVec::new();
    let mut depth = 0usize;
    let mut cur: SmallVec<[u8; 64]> = SmallVec::new();
    for ch in shorthand.chars() {
        match ch {
            '\'' | '"' => {
                let mut buf = [0u8; 4];
                cur.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                depth ^= 1;
            }
            ',' if depth == 0 => {
                push_family(&mut out, cur.as_slice());
                cur.clear();
            }
            _ => {
                let mut buf = [0u8; 4];
                cur.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    push_family(&mut out, cur.as_slice());
    out
}

fn make_policy_object<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let fp = Object::new(ctx.clone())?;
    class_tag(ctx, &fp, "FeaturePolicy")?;
    let allows = Function::new(
        ctx.clone(),
        |_c: Ctx<'js>,
         feature: rquickjs::function::Opt<Value<'js>>,
         _o: Rest<Value<'js>>|
         -> bool {
            let fc = feature.0.as_ref().and_then(|v| crate::webidl::value_to_str(v));
            let f: &str = fc.as_ref().map(|x| x.as_str()).unwrap_or("");
            if !FEATURE_LIST.contains(&f) {
                return false;
            }
            !FEATURE_DEFAULT_OFF.contains(&f)
        },
    )?;
    crate::webidl::define_method(ctx, &fp, "allowsFeature", allows)?;
    let allowed = Function::new(ctx.clone(), |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
        let arr = rquickjs::Array::new(c.clone())?;
        let mut i = 0usize;
        for f in FEATURE_LIST {
            if !FEATURE_DEFAULT_OFF.contains(f) {
                arr.set(i, *f)?;
                i += 1;
            }
        }
        Ok(arr.into_value())
    })?;
    crate::webidl::define_method(ctx, &fp, "allowedFeatures", allowed)?;
    let features = Function::new(ctx.clone(), |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
        let arr = rquickjs::Array::new(c.clone())?;
        for (i, f) in FEATURE_LIST.iter().enumerate() {
            arr.set(i, *f)?;
        }
        Ok(arr.into_value())
    })?;
    crate::webidl::define_method(ctx, &fp, "features", features)?;
    Ok(fp)
}

const FONTFACE_ERR: &str =
    "Failed to construct 'FontFace': The string did not match the expected pattern.";

fn install_font_face<'js>(ctx: &Ctx<'js>, globals: &Object<'js>) -> rquickjs::Result<()> {
    let proto = Object::new(ctx.clone())?;
    class_tag(ctx, &proto, "FontFace")?;
    let ctor = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, family: rquickjs::function::Opt<Value<'js>>, source: rquickjs::function::Opt<Value<'js>>, _d: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::FONTS);
            let family = family.0.unwrap_or_else(|| Value::new_undefined(c.clone()));
            let source = source.0.unwrap_or_else(|| Value::new_undefined(c.clone()));
            let fcc = crate::webidl::value_to_str(&family);
            let Some(fam) = fcc.as_ref().map(|x| x.as_str()) else {
                return Err(rquickjs::Exception::throw_type(&c, FONTFACE_ERR));
            };
            if fam.trim().is_empty() {
                return Err(rquickjs::Exception::throw_type(&c, FONTFACE_ERR));
            }
            let src_is_buf = source.as_object().is_some_and(|o| o.is_array_buffer()) || {
                source
                    .as_object()
                    .and_then(|o| o.get::<_, Option<f64>>("byteLength").ok().flatten())
                    .is_some()
            };
            let src_ok = if src_is_buf {
                true
            } else {
                source
                    .as_string()
                    .and_then(|s| s.clone().to_cstring().ok())
                    .map(|cs| {
                        let t = cs.as_str().trim_start();
                        t.len() >= 4 && t[..4].eq_ignore_ascii_case("url(")
                    })
                    .unwrap_or(false)
            };
            if !src_ok {
                return Err(rquickjs::Exception::throw_type(&c, FONTFACE_ERR));
            }
            let o = Object::new(c.clone())?;
            class_tag(&c, &o, "FontFace")?;
            o.set("family", fam)?;
            o.set("style", "normal")?;
            o.set("weight", "normal")?;
            o.set("stretch", "normal")?;
            o.set("status", "unloaded")?;
            let load = Function::new(c.clone(), |cc: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                if let Some(ob) = this.0.as_object() {
                    ob.set("status", "loaded")?;
                }
                crate::worker::promise_resolve(&cc, this.0.clone())
            })?;
            crate::webidl::define_method(&c, &o, "load", load)?;
            Ok(o.into_value())
        },
    )?
    .with_constructor(true);
    crate::stackfmt::set_fn_name(ctx, &ctor, "FontFace")?;
    crate::stackfmt::set_fn_len(ctx, &ctor, 2)?;
    ctor.prop("prototype", rquickjs::object::Property::from(proto.clone()))?;
    proto.prop(
        "constructor",
        rquickjs::object::Property::from(ctor.clone())
            .writable()
            .configurable(),
    )?;
    globals.prop(
        "FontFace",
        rquickjs::object::Property::from(ctor)
            .writable()
            .configurable(),
    )?;
    Ok(())
}

fn make_font_set<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let ffs = Object::new(ctx.clone())?;
    class_tag(ctx, &ffs, "FontFaceSet")?;
    ffs.set("status", "loaded")?;
    ffs.set("size", 0u32)?;
    for key in [
        "onloading",
        "onloadingdone",
        "onloadingerror",
        "onloadstart",
    ] {
        ffs.set(key, Value::new_null(ctx.clone()))?;
    }
    let check = Function::new(
        ctx.clone(),
        |_c: Ctx<'js>, font: rquickjs::function::Opt<Value<'js>>, _t: Rest<Value<'js>>| -> bool {
            touch::touch_log_record(ApiKey::FONTS);
            let fc = font.0.as_ref().and_then(|v| crate::webidl::value_to_str(v));
            let f: &str = fc.as_ref().map(|c| c.as_str()).unwrap_or("");
            let families = families_of_font_shorthand(f);
            if families.is_empty() {
                return false;
            }
            families.iter().any(|fam| family_available(fam.as_str()))
        },
    )?;
    crate::stackfmt::set_fn_len(ctx, &check, 1)?;
    crate::webidl::define_method(ctx, &ffs, "check", check)?;
    for key in ["has", "delete"] {
        let f = Function::new(ctx.clone(), |_c: Ctx<'js>, _f: Value<'js>| -> bool {
            false
        })?;
        crate::webidl::define_method(ctx, &ffs, key, f)?;
    }
    let add = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, f: Value<'js>| -> rquickjs::Result<()> {
            touch::touch_log_record(ApiKey::FONTS);
            let is_ff = f
                .as_object()
                .and_then(|o| {
                    let tag: Option<String> = o
                        .ctx()
                        .eval::<rquickjs::Symbol, _>("Symbol.toStringTag")
                        .ok()
                        .and_then(|sym| o.get::<_, String>(sym).ok());
                    tag
                })
                .is_some_and(|t| t == "FontFace");
            if !is_ff {
                return Err(rquickjs::Exception::throw_type(
                    &c,
                    "Failed to execute 'add' on 'FontFaceSet': parameter 1 is not of type 'FontFace'.",
                ));
            }
            Ok(())
        },
    )?;
    crate::webidl::define_method(ctx, &ffs, "add", add)?;
    let clear = Function::new(ctx.clone(), |_c: Ctx<'js>| {})?;
    crate::webidl::define_method(ctx, &ffs, "clear", clear)?;
    let iter = Function::new(ctx.clone(), |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
        let arr = rquickjs::Array::new(c.clone())?;
        crate::webidl::iterable_of_array(&c, &arr)
    })?;
    let sym: rquickjs::Symbol = ctx.eval("Symbol.iterator")?;
    ffs.set(sym, iter)?;
    let ready = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            crate::worker::promise_resolve(&c, this.0.clone())
        },
    )?;
    crate::webidl::named_accessor(ctx, &ffs, "ready", ready, None)?;
    let load_f = Function::new(ctx.clone(), |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
        crate::worker::promise_resolve(&c, Value::new_undefined(c.clone()))
    })?;
    crate::webidl::define_method(ctx, &ffs, "load", load_f)?;
    Ok(ffs)
}

pub(crate) fn install<'js>(ctx: &Ctx<'js>, doc_proto: &Object<'js>) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    install_font_face(ctx, &globals)?;
    let fonts = make_font_set(ctx)?;
    stubbed_accessor(ctx, doc_proto, "fonts", fonts, Some(ApiKey::FONTS))?;
    for key in ["featurePolicy", "permissionsPolicy"] {
        let fp = make_policy_object(ctx)?;
        stubbed_accessor(ctx, doc_proto, key, fp, None)?;
    }
    Ok(())
}
