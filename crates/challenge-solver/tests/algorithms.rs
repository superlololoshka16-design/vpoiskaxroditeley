use challenge_solver::{Algorithm, argon, chain, hmac};
use core_utils::BytesExt as _;
use sha2::{Digest, Sha256};

fn sha_ref(msg: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(msg);
    let out = h.finalize();
    let mut b = [0u8; 32];
    b.copy_from_slice(&out);
    b
}

fn hmac_ref(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let h = sha_ref(key);
        k[..32].copy_from_slice(&h);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = k;
    let mut opad = k;
    for i in 0..64 {
        ipad[i] ^= 0x36;
        opad[i] ^= 0x5c;
    }
    let mut inner = ipad.to_vec();
    inner.extend_from_slice(msg);
    let h1 = sha_ref(&inner);
    let mut outer = opad.to_vec();
    outer.extend_from_slice(&h1);
    sha_ref(&outer)
}

#[test]
fn unsupported_argon_variants_are_not_silently_solved_as_id() {
    assert_eq!(Algorithm::parse(b"argon2"), Some(Algorithm::Argon2));
    assert_eq!(Algorithm::parse(b"argon2id"), Some(Algorithm::Argon2));
    assert!(Algorithm::parse(b"argon2i").is_none());
    assert!(Algorithm::parse(b"argon2d").is_none());
}

#[test]
fn hmac_rfc4231_vector() {
    let ctx = hmac::HmacCtx::new(b"key", b"The quick brown fox jumps over the lazy dog", 0);
    let d = ctx.digest(0, 0);
    let mut hex = [0u8; 64];
    d.hex_lower_into(&mut hex);
    assert_eq!(
        std::str::from_utf8(&hex).unwrap(),
        "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
    );
}

#[test]
fn hmac_solver_matches_reference() {
    let key = b"server-secret-key";
    let msg = b"challenge-nonce-42";
    let (n, h) = hmac::solve(key, msg, 3, 2).expect("solve");
    let mut m = msg.to_vec();
    m.extend_from_slice(n.to_string().as_bytes());
    assert_eq!(h, hmac_ref(key, &m));
    let st = core_utils::be32_words(&h);
    assert!(core_utils::lz_words_be(&st) >= 12);
}

#[test]
fn hmac_digest_path_with_nonce() {
    let ctx = hmac::HmacCtx::new(b"key", b"msg", 0);
    let d = ctx.digest(42, 2);
    let mut m = b"msg".to_vec();
    m.extend_from_slice(b"42");
    assert_eq!(d, hmac_ref(b"key", &m));
}

#[test]
fn chain_matches_reference() {
    let salt = b"salt-data-abc";
    let ctx = chain::ChainCtx::new(salt, 3, 0);
    let mut msg = salt.to_vec();
    msg.extend_from_slice(b"12345");
    let mut st = sha_ref(&msg).to_vec();
    for _ in 1..3 {
        st = sha_ref(&st).to_vec();
    }
    let mine = ctx.digest(12345, 5);
    assert_eq!(mine.to_vec(), st);
}

#[test]
fn chain_solver_matches_reference() {
    let salt = b"chain-solver-consistency";
    let (n, h) = chain::solve(salt, 3, 3, 2).expect("solve");
    let mut msg = salt.to_vec();
    msg.extend_from_slice(n.to_string().as_bytes());
    let mut st = sha_ref(&msg).to_vec();
    for _ in 1..3 {
        st = sha_ref(&st).to_vec();
    }
    assert_eq!(h.to_vec(), st);
    let cst = core_utils::be32_words(&h);
    assert!(core_utils::lz_words_be(&cst) >= 12);
}

#[test]
fn chain_single_round_is_plain_pow() {
    let salt = b"single-round";
    let (n, h) = chain::solve(salt, 1, 3, 2).expect("solve");
    let mut msg = salt.to_vec();
    msg.extend_from_slice(n.to_string().as_bytes());
    assert_eq!(h, sha_ref(&msg));
}

#[test]
fn argon2id_matches_reference_crate() {
    use argon2::{Algorithm, Argon2, Params, Version};
    let salt = b"0123456789abcdef";
    let m_cost = 8u32;
    let t_cost = 1u32;
    let a = Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(m_cost, t_cost, 1, Some(32)).unwrap(),
    );
    let ctx = argon::ArgonCtx::new(salt, 0, m_cost, t_cost);
    let mut mem = vec![[0u64; 128]; ctx.m_prime() as usize];
    for nonce in [0u64, 1, 7, 1234] {
        let width = if nonce == 0 {
            1
        } else {
            (nonce as f64).log10() as usize + 1
        };
        let mine = ctx.digest(nonce, width, &mut mem);
        let mut pw = salt.to_vec();
        pw.extend_from_slice(nonce.to_string().as_bytes());
        let mut theirs = [0u8; 32];
        a.hash_password_into(&pw, salt, &mut theirs).unwrap();
        assert_eq!(mine.to_vec(), theirs.to_vec(), "nonce {nonce}");
    }
}

#[test]
fn argon2_solver_finds_valid_nonce() {
    let salt = b"argon-solve-check";
    let (n, tag) = argon::solve(salt, 2, 8, 1, 2).expect("solve");
    let st = core_utils::be32_words(&tag);
    assert!(core_utils::lz_words_be(&st) >= 8, "nonce={n}");
}
