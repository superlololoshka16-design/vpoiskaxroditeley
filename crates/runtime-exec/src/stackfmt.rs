use core_utils::{FxBuild, fx_map};
use rquickjs::function::{Rest, This};
use rquickjs::{Ctx, Function, Object, Persistent, Value};
use std::cell::RefCell;

const V8_LIMIT: u32 = 10;
const APPLY_SRC: &str = "(function (f, t, a) { return f.apply(t, a); })";
const CTOR_SRC: &str = "(function (C, a) { return new C(...a); })";

type ThunkSlot = std::thread::LocalKey<RefCell<Option<Persistent<Function<'static>>>>>;

thread_local! {
    static CTOR_THUNK: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
    static APPLY_THUNK: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
    static NAME_THUNK: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
    static LEN_THUNK: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
    static ORIG_CTORS: RefCell<std::collections::HashMap<&'static str, Persistent<Function<'static>>, FxBuild>> = RefCell::new(fx_map());
    static ORIG_TOSTRING: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
}

fn wrap_error_ctor<'js>(ctx: &Ctx<'js>, name: &'static str) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let orig: Function = globals.get(name)?;
    ORIG_CTORS.with(|c| c.borrow_mut().insert(name, Persistent::save(ctx, orig)));
    let wrapped = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let Some(ctor) = crate::webidl::restore_slot(&c, &CTOR_THUNK) else {
                return Ok(Value::new_null(c));
            };
            let Some(orig) = ORIG_CTORS.with(|m| m.borrow().get(name).cloned()) else {
                return Ok(Value::new_null(c));
            };
            let Ok(orig) = orig.restore(&c) else {
                return Ok(Value::new_null(c));
            };
            let arr = rquickjs::Array::new(c.clone())?;
            for (i, a) in args.0.iter().enumerate() {
                arr.set(i, a.clone())?;
            }
            let built: Value<'js> = ctor.call((orig, arr))?;
            if let Some(obj) = built.as_object() {
                normalize_error_stack(&c, obj);
            }
            Ok(built)
        },
    )?
    .with_constructor(true);
    set_fn_name(ctx, &wrapped, name)?;
    set_fn_len(ctx, &wrapped, 1)?;
    let _ = globals.set(name, wrapped);
    Ok(())
}

pub(crate) fn set_fn_name<'js>(
    ctx: &Ctx<'js>,
    f: &Function<'js>,
    name: &str,
) -> rquickjs::Result<()> {
    def_prop_thunk(
        ctx,
        &NAME_THUNK,
        "(function (f, n) { Object.defineProperty(f, \"name\", { value: n, writable: false, configurable: true }); return f; })",
        f,
        name,
    )
}

pub(crate) fn set_fn_len<'js>(ctx: &Ctx<'js>, f: &Function<'js>, len: i32) -> rquickjs::Result<()> {
    def_prop_thunk(
        ctx,
        &LEN_THUNK,
        "(function (f, v) { Object.defineProperty(f, \"length\", { value: v, writable: false, configurable: true }); return f; })",
        f,
        len,
    )
}

fn def_prop_thunk<'js, A>(
    ctx: &Ctx<'js>,
    slot: &'static ThunkSlot,
    js: &str,
    f: &Function<'js>,
    arg: A,
) -> rquickjs::Result<()>
where
    A: rquickjs::IntoJs<'js>,
{
    let thunk = crate::webidl::cached_persistent(ctx, slot, |c| c.eval(js))?;
    let _: Value<'js> = thunk.call((f.clone(), arg))?;
    Ok(())
}

pub(crate) fn apply_caller<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    crate::webidl::cached_persistent(ctx, &APPLY_THUNK, |c| c.eval(APPLY_SRC))
}

pub(crate) fn init<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let ctor: Function = ctx.eval(CTOR_SRC)?;
    CTOR_THUNK.with(|p| *p.borrow_mut() = Some(Persistent::save(ctx, ctor)));
    Ok(())
}

pub(crate) fn clear() {
    CTOR_THUNK.with(|c| *c.borrow_mut() = None);
    APPLY_THUNK.with(|c| *c.borrow_mut() = None);
    NAME_THUNK.with(|c| *c.borrow_mut() = None);
    LEN_THUNK.with(|c| *c.borrow_mut() = None);
    ORIG_CTORS.with(|c| c.borrow_mut().clear());
    ORIG_TOSTRING.with(|c| *c.borrow_mut() = None);
}

fn norm_into(out: &mut String, rest: &str) {
    let b = rest.as_bytes();
    let mut run = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'('
            && b.len() - i >= 13
            && &b[i..i + 13] == b"(eval_script:"
        {
            push_run(out, &b[run..i]);
            out.push_str("(<eval>:");
            i += 13;
            run = i;
        } else if b[i] == b'(' && b.len() - i >= 8 && &b[i..i + 8] == b"(module:" {
            push_run(out, &b[run..i]);
            out.push_str("(<module>:");
            i += 8;
            run = i;
        } else {
            i += 1;
        }
    }
    push_run(out, &b[run..]);
}

#[inline]
fn push_run(out: &mut String, run: &[u8]) {
    if !run.is_empty() {
        out.push_str(std::str::from_utf8(run).unwrap_or(""));
    }
}

fn emit_frame(out: &mut String, frames: &mut u32, rest: &str) {
    let body = rest
        .strip_prefix("anonymous (")
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(rest);
    out.push_str("    at ");
    norm_into(out, body);
    out.push('\n');
    *frames += 1;
}

pub(crate) fn v8_stack(raw: &str, limit: u32) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut frames = 0u32;
    let mut first = true;
    for line in raw.lines() {
        let l = line.trim();
        if first {
            out.push_str(line);
            out.push('\n');
            first = false;
            continue;
        }
        if frames >= limit {
            break;
        }
        if let Some(rest) = l.strip_prefix("at ") {
            emit_frame(&mut out, &mut frames, rest);
        } else if !l.is_empty() {
            emit_frame(&mut out, &mut frames, l);
        }
    }
    out.pop();
    out
}

fn stack_limit(ctx: &Ctx<'_>) -> u32 {
    let err_ctor: Option<Function> = ctx.globals().get("Error").ok().flatten();
    let v: Option<f64> = err_ctor.and_then(|c| c.get("stackTraceLimit").ok().flatten());
    v.filter(|x| x.is_finite() && *x > 0.0)
        .map(|x| V8_LIMIT.max(x as u32))
        .unwrap_or(V8_LIMIT)
}

fn set_stack(ctx: &Ctx<'_>, obj: &Object<'_>, raw: &str, limit_adj: u32) {
    let limit = stack_limit(ctx);
    let _ = obj.set("stack", v8_stack(raw, limit.saturating_sub(limit_adj)));
}

fn normalize_error_stack(ctx: &Ctx<'_>, obj: &Object<'_>) {
    let raw: Option<rquickjs::String> = obj.get("stack").ok().flatten();
    if let Some(raw) = raw {
        if let Ok(cstr) = raw.to_cstring() {
            set_stack(ctx, obj, cstr.as_str(), 0);
        }
    }
}

pub(crate) fn install_error_stack<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    for name in [
        "Error",
        "TypeError",
        "RangeError",
        "ReferenceError",
        "SyntaxError",
        "URIError",
        "EvalError",
    ] {
        wrap_error_ctor(ctx, name)?;
    }
    let globals = ctx.globals();
    let err_ctor: Function = globals.get("Error")?;
    err_ctor.set("stackTraceLimit", V8_LIMIT as f64)?;
    let capture = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, target: Value<'js>| -> rquickjs::Result<Value<'js>> {
            if target.is_object()
                && let Some(obj) = target.as_object()
            {
                let fresh: Option<String> = match crate::touch::stack_grab_fn(&c) {
                    Some(f) => f.call(())?,
                    None => None,
                };
                if let Some(raw) = fresh {
                    set_stack(&c, &obj, raw.as_str(), 1);
                }
            }
            Ok(Value::new_undefined(c))
        },
    )?;
    set_fn_name(ctx, &capture, "captureStackTrace")?;
    err_ctor.set("captureStackTrace", capture)?;
    Ok(())
}

pub(crate) fn install_tostring<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let fproto: Object = ctx.eval::<Function, _>("Function")?.get("prototype")?;
    let orig: Function = fproto.get("toString")?;
    ORIG_TOSTRING.with(|c| *c.borrow_mut() = Some(Persistent::save(ctx, orig)));
    let ts = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<String> {
            if !this.0.is_function() {
                return Err(rquickjs::Exception::throw_message(
                    &c,
                    "TypeError: Function.prototype.toString called on incompatible receiver",
                ));
            }
            let Some(orig) = crate::webidl::restore_slot(&c, &ORIG_TOSTRING) else {
                return Ok(String::new());
            };
            let Ok(apply) = apply_caller(&c) else {
                return Ok(String::new());
            };
            let empty = rquickjs::Array::new(c.clone())?;
            let raw: Value<'js> = apply.call((orig, this.0.clone(), empty))?;
            let raw: String = raw.get()?;
            if raw.contains("[native code]") {
                let name: String = this
                    .0
                    .as_object()
                    .and_then(|o| o.get::<_, Option<String>>("name").ok().flatten())
                    .unwrap_or_default();
                let mut out = String::with_capacity(24 + name.len());
                out.push_str("function ");
                out.push_str(name.as_str());
                out.push_str("() { [native code] }");
                return Ok(out);
            }
            Ok(raw)
        },
    )?;
    set_fn_name(ctx, &ts, "toString")?;
    fproto.set("toString", ts)?;
    Ok(())
}
