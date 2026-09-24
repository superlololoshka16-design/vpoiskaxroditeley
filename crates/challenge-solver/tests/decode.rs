use challenge_solver::decode::{DecodeError, Scratch, TaskKind, decode};
use core_utils::BytesExt as _;

fn envelope(alg: &str, len: u32, payload: &[u8]) -> Vec<u8> {
    let meta = format!("{{\"alg\":\"{alg}\",\"len\":{len}}}");
    let mut mid: Vec<u8> = Vec::with_capacity(2 + meta.len() + 32 + payload.len() * 2);
    mid.extend_from_slice(&(meta.len() as u16).to_be_bytes());
    mid.extend_from_slice(meta.as_bytes());
    mid.extend_from_slice(&core_utils::sha256(payload));
    let cap = core_utils::b64_encoded_len(payload.len());
    let mut b64 = vec![0u8; cap];
    let n = payload.b64_encode_into(&mut b64).expect("b64 payload");
    b64.truncate(n);
    mid.extend_from_slice(&b64);
    let cap_mid = core_utils::b64_encoded_len(mid.len());
    let mut out = vec![0u8; cap_mid];
    let m = mid.b64_encode_into(&mut out).expect("b64 mid");
    out.truncate(m);
    out
}

#[test]
fn roundtrip_standard() {
    let payload: Vec<u8> = (0..300u32).map(|i| (i % 251) as u8).collect();
    let env = envelope("slider", payload.len() as u32, &payload);
    let mut s = Scratch::with_capacity(4096);
    let (meta, out) = decode(&env, &mut s).expect("decode");
    assert_eq!(meta.kind, TaskKind::Slider);
    assert_eq!(meta.len, payload.len() as u32);
    assert_eq!(out, payload.as_slice());
}

#[test]
fn roundtrip_pow_kind() {
    let payload = b"anubis-pow-challenge-bytes".to_vec();
    let env = envelope("fast", payload.len() as u32, &payload);
    let mut s = Scratch::default();
    let (meta, out) = decode(&env, &mut s).expect("decode");
    assert_eq!(meta.kind, TaskKind::Pow);
    assert_eq!(out, payload.as_slice());
}

#[test]
fn tampered_envelope_fails_digest() {
    let payload = b"integrity-check".to_vec();
    let mut env = envelope("fast", payload.len() as u32, &payload);
    let mid = env.len() / 2;
    env[mid] = env[mid].wrapping_add(1);
    let mut s = Scratch::default();
    assert!(matches!(
        decode(&env, &mut s),
        Err(DecodeError::Base64 | DecodeError::Digest)
    ));
}

#[test]
fn len_field_mismatch_rejected() {
    let payload = b"size-check".to_vec();

    let env = envelope("fast", payload.len() as u32, &payload);
    let mut s = Scratch::default();
    let (meta, _) = decode(&env, &mut s).expect("decode");
    assert_eq!(meta.len, payload.len() as u32);

    let forged = envelope("fast", 999, &payload);
    let mut s2 = Scratch::default();
    assert!(matches!(decode(&forged, &mut s2), Err(DecodeError::Size)));

    let forged2 = envelope("fast", 1, &payload);
    let mut s3 = Scratch::default();
    assert!(matches!(decode(&forged2, &mut s3), Err(DecodeError::Size)));
}

#[test]
fn whitespace_envelope_accepted() {
    let payload = b"ws-check".to_vec();
    let env = envelope("fast", payload.len() as u32, &payload);
    let mut padded = Vec::with_capacity(env.len() + 4);
    padded.push(b'\n');
    padded.extend_from_slice(&env);
    padded.push(b'\r');
    let mut s = Scratch::default();
    let (_, out) = decode(&padded, &mut s).expect("decode");
    assert_eq!(out, payload.as_slice());
}

#[test]
fn empty_payload_envelope_rejected() {
    let env = envelope("fast", 0, b"");
    let mut s = Scratch::default();
    assert!(matches!(decode(&env, &mut s), Err(DecodeError::Alphabet)));
}

#[test]
fn truncated_envelope_rejected() {
    let payload = b"cut-me".to_vec();
    let env = envelope("fast", payload.len() as u32, &payload);
    let mut s = Scratch::default();
    for cut in [1, 2, 4, env.len() / 2] {
        assert!(
            decode(&env[..cut], &mut s).is_err(),
            "обрезка до {cut} обязана падать"
        );
    }
}

#[test]
fn oversized_meta_header_rejected() {
    let mut mid: Vec<u8> = vec![0x02, 0x01];
    mid.extend_from_slice(b"{\"alg\":\"fast\",\"len\":1}");
    mid.extend_from_slice(&[0u8; 32]);
    mid.extend_from_slice(b"aGVsbG8=");
    let cap = core_utils::b64_encoded_len(mid.len());
    let mut env = vec![0u8; cap];
    let n = mid.b64_encode_into(&mut env).expect("b64 mid");
    env.truncate(n);
    let mut s = Scratch::default();
    assert!(matches!(decode(&env, &mut s), Err(DecodeError::Meta)));
}

#[test]
fn garbage_rejected_without_panic() {
    let mut s = Scratch::with_capacity(256);
    for bad in [
        &b""[..],
        b"!!!!",
        b"AAAA",
        b"////",
        b"aGVsbG8=",
        &vec![0u8; 64][..],
    ] {
        let r = decode(bad, &mut s);
        assert!(r.is_err(), "input {bad:?} should fail");
    }
}
