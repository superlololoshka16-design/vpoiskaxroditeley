use crate::touch::{self, ApiKey};
use core_utils::crypto::{
    PBKDF2_MAX_ITERATIONS, SUBTLE_MAX_OUT_BYTES, hmac_sha256_into, pbkdf2_sha256_into, sha1_into,
    sha256_into,
};
use crate::worker::{promise_reject, promise_resolve};
use rquickjs::{Ctx, Function, IntoJs, Object, Value, object::Property};
use smallvec::SmallVec;


fn key_raw<'js>(key_obj: &Object<'js>) -> Option<&'js [u8]> {
    let buf: Option<Value<'js>> = key_obj.get("_raw").ok().flatten();
    let buf = buf?;
    crate::webidl::ab_bytes(&buf)
}

fn rand_fill(buf: &mut [u8]) {
    crate::worker::FAST_RNG.with_borrow_mut(|r| {
        let mut i = 0;
        while i + 8 <= buf.len() {
            buf[i..i + 8].copy_from_slice(&r.next_u64().to_le_bytes());
            i += 8;
        }
        if i < buf.len() {
            let tail = r.next_u64().to_le_bytes();
            let n = buf.len() - i;
            buf[i..].copy_from_slice(&tail[..n]);
        }
    });
}

fn random_uuid<'js>(c: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    touch::touch_log_record(ApiKey::CIPHERS);
    let mut b = [0u8; 16];
    rand_fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    core_utils::hex_grouped(&b, false, '-', &[8, 4, 4, 4, 12])
        .as_str()
        .into_js(c)
}

fn subtle_digest<'js>(
    c: Ctx<'js>,
    algo: rquickjs::String<'js>,
    data: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    touch::touch_log_record(ApiKey::CIPHERS);
    let ac = algo.to_cstring()?;
    let algo: &str = ac.as_str();
    let borrowed = crate::webidl::ta_bytes(&data).or_else(|| crate::webidl::ab_bytes(&data));
    let owned = if borrowed.is_none() {
        crate::webidl::value_to_bytes(&data)
    } else {
        None
    };
    let bytes: &[u8] = borrowed.unwrap_or_else(|| owned.as_deref().unwrap_or_default());
    let ab = if algo.eq_ignore_ascii_case("SHA-1") || algo.eq_ignore_ascii_case("SHA1") {
        let mut d = [0u8; 20];
        sha1_into(bytes, &mut d);
        rquickjs::ArrayBuffer::new_copy(c.clone(), &d)?
    } else {
        let mut d = [0u8; 32];
        sha256_into(bytes, &mut d);
        rquickjs::ArrayBuffer::new_copy(c.clone(), &d)?
    };
    promise_resolve(&c, ab.into_value())
}

fn subtle_sign<'js>(
    c: Ctx<'js>,
    algo: rquickjs::String<'js>,
    key: Value<'js>,
    data: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    touch::touch_log_record(ApiKey::CIPHERS);
    let ac = algo.to_cstring()?;
    if !ac.as_str().eq_ignore_ascii_case("HMAC") {
        return promise_reject(&c, "TypeError: Unsupported algorithm");
    }
    let key_obj = match key.as_object() {
        Some(o) => o,
        None => {
            return promise_reject(&c, "TypeError: key is not a CryptoKey");
        }
    };
    let kb = match key_raw(key_obj) {
        Some(k) => k,
        None => return promise_reject(&c, "TypeError: key is not a raw-imported CryptoKey"),
    };
    let borrowed = crate::webidl::ta_bytes(&data).or_else(|| crate::webidl::ab_bytes(&data));
    let owned = if borrowed.is_none() {
        crate::webidl::value_to_bytes(&data)
    } else {
        None
    };
    let bytes: &[u8] = borrowed.unwrap_or_else(|| owned.as_deref().unwrap_or_default());
    let mut mac = [0u8; 32];
    hmac_sha256_into(kb, bytes, &mut mac);
    let ab = rquickjs::ArrayBuffer::new_copy(c.clone(), &mac)?;
    promise_resolve(&c, ab.into_value())
}

fn subtle_import_key<'js>(
    c: Ctx<'js>,
    fmt: rquickjs::String<'js>,
    key_data: Value<'js>,
    algo: Value<'js>,
    _extractable: Value<'js>,
    _usages: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    touch::touch_log_record(ApiKey::CIPHERS);
    let fc = fmt.to_cstring()?;
    if !fc.as_str().eq_ignore_ascii_case("raw") {
        return promise_reject(&c, "TypeError: Unsupported key format");
    }
    let bytes = crate::webidl::ta_bytes(&key_data).unwrap_or_default();
    if bytes.len() > 1 << 20 {
        return promise_reject(&c, "TypeError: key too large");
    }
    let ck = Object::new(c.clone())?;
    ck.set("type", "secret")?;
    ck.set("extractable", true)?;
    ck.set("algorithm", algo)?;
    let raw_ab = rquickjs::ArrayBuffer::new_copy(c.clone(), bytes)?;
    ck.prop("_raw", Property::from(raw_ab).writable().configurable())?;
    promise_resolve(&c, ck.into_value())
}

fn subtle_derive_bits<'js>(
    c: Ctx<'js>,
    algo: Value<'js>,
    base: Value<'js>,
    bits: f64,
) -> rquickjs::Result<Value<'js>> {
    touch::touch_log_record(ApiKey::CIPHERS);
    let a = algo.as_object().cloned();
    let name: Option<String> = a.as_ref().and_then(|o| o.get("name").ok());
    if !name
        .as_deref()
        .is_some_and(|n| n.eq_ignore_ascii_case("PBKDF2"))
    {
        return promise_reject(&c, "TypeError: Unsupported algorithm");
    }
    let iterations: f64 = a
        .as_ref()
        .and_then(|o| o.get("iterations").ok())
        .unwrap_or(0.0);
    if !(iterations.is_finite() && iterations >= 1.0) {
        return promise_reject(&c, "TypeError: Invalid iterations");
    }
    let iterations = iterations.min(PBKDF2_MAX_ITERATIONS as f64) as u32;
    let salt: Option<&[u8]> = a
        .as_ref()
        .and_then(|o| o.get::<_, Value<'js>>("salt").ok())
        .and_then(|v| crate::webidl::ta_bytes(&v));
    let pass: Option<&[u8]> = base.as_object().and_then(|o| {
        let buf: Option<Value<'js>> = o.get("_raw").ok().flatten();
        buf.as_ref().and_then(|b| crate::webidl::ab_bytes(b))
    });
    if !bits.is_finite() || bits <= 0.0 || bits as usize > SUBTLE_MAX_OUT_BYTES * 8 {
        return promise_reject(&c, "TypeError: Invalid length");
    }
    let mut out: SmallVec<[u8; 128]> = SmallVec::new();
    out.resize((bits / 8.0).ceil() as usize, 0);
    let ok = pbkdf2_sha256_into(
        pass.unwrap_or_default(),
        salt.unwrap_or_default(),
        iterations,
        out.as_mut_slice(),
    );
    if ok.is_err() {
        return promise_reject(&c, "OperationError: deriveBits failed");
    }
    let ab = rquickjs::ArrayBuffer::new_copy(c.clone(), out.as_slice())?;
    promise_resolve(&c, ab.into_value())
}

fn get_random_values<'js>(c: Ctx<'js>, val: Value<'js>) -> rquickjs::Result<Value<'js>> {
    touch::touch_log_record(ApiKey::CIPHERS);
    let ta = val
        .as_object()
        .and_then(|o| o.as_typed_array::<u8>())
        .map(|t| t.len());
    let Some(len) = ta else {
        return Err(rquickjs::Exception::throw_message(
            &c,
            "TypeMismatchError: Argument 1 of Crypto.getRandomValues is not an ArrayBufferView",
        ));
    };
    if len > 65536 {
        return Err(rquickjs::Exception::throw_message(
            &c,
            "QuotaExceededError: The requested length exceeds 65,536 bytes",
        ));
    }
    let view = unsafe { crate::webidl::ta_bytes_mut(&val).unwrap_unchecked() };
    rand_fill(view);
    Ok(val)
}

pub(crate) fn build<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let crypto = Object::new(ctx.clone())?;
    let grv = Function::new(ctx.clone(), get_random_values)?;
    crate::webidl::define_method(ctx, &crypto, "getRandomValues", grv)?;
    let uuid = Function::new(ctx.clone(), |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
        random_uuid(&c)
    })?;
    crate::webidl::define_method(ctx, &crypto, "randomUUID", uuid)?;
    let subtle = Object::new(ctx.clone())?;
    let digest = Function::new(ctx.clone(), subtle_digest)?;
    crate::webidl::define_method(ctx, &subtle, "digest", digest)?;
    let sign = Function::new(ctx.clone(), subtle_sign)?;
    crate::webidl::define_method(ctx, &subtle, "sign", sign)?;
    let import_key = Function::new(ctx.clone(), subtle_import_key)?;
    crate::webidl::define_method(ctx, &subtle, "importKey", import_key)?;
    let derive_bits = Function::new(ctx.clone(), subtle_derive_bits)?;
    crate::webidl::define_method(ctx, &subtle, "deriveBits", derive_bits)?;
    crypto.prop(
        "subtle",
        rquickjs::object::Property::from(subtle)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    Ok(crypto)
}
