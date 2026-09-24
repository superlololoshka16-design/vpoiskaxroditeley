use challenge_solver::pow;
use core_utils::crypto as arch;
use sha2::{Digest, Sha256};

fn sha_ref(msg: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(msg);
    let out = h.finalize();
    let mut b = [0u8; 32];
    b.copy_from_slice(&out);
    b
}

fn solve_ref(data: &[u8], difficulty: u8) -> (u64, [u8; 32]) {
    let need = difficulty as u32 * 4;
    let mut n = 0u64;
    loop {
        let mut msg = data.to_vec();
        msg.extend_from_slice(n.to_string().as_bytes());
        let h = sha_ref(&msg);
        let st = arch::state_of(&h);
        if core_utils::lz_words_be(&st) >= need {
            return (n, h);
        }
        n += 1;
    }
}

#[test]
fn sha256_ref_vectors() {
    assert_eq!(core_utils::sha256(b""), sha_ref(b""));
    assert_eq!(core_utils::sha256(b"abc"), sha_ref(b"abc"));
    let long: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
    assert_eq!(core_utils::sha256(&long), sha_ref(&long));
    let exact64: Vec<u8> = (0..64u32).map(|i| i as u8).collect();
    assert_eq!(core_utils::sha256(&exact64), sha_ref(&exact64));
    let tail56: Vec<u8> = (0..56u32).map(|i| i as u8).collect();
    assert_eq!(core_utils::sha256(&tail56), sha_ref(&tail56));
    let tail57: Vec<u8> = (0..57u32).map(|i| (i * 7) as u8).collect();
    assert_eq!(core_utils::sha256(&tail57), sha_ref(&tail57));
}

#[test]
fn sha256_ni_matches_reference() {
    for msg in [
        Vec::new(),
        b"abc".to_vec(),
        b"anubis self test vector 0123456789".to_vec(),
        (0..500u32).map(|i| (i % 253) as u8).collect(),
    ] {
        assert_eq!(core_utils::sha256(&msg), sha_ref(&msg), "len={}", msg.len());
    }
}

#[test]
fn digest_lz_is_branchless_semantics() {
    for w in 0u32..=0xFFFF {
        let lz = if w == 0 { 32 } else { w.leading_zeros() };
        assert_eq!(lz, w.leading_zeros());
    }
    let st = [0u32, 0u32, 5, 0, 0, 0, 0, 0];
    assert_eq!(core_utils::lz_words_be(&st), 93);
    let st = [0u32; 8];
    assert_eq!(core_utils::lz_words_be(&st), 256);
}

#[test]
fn lz_batch16_matches_scalar() {
    if !arch::cpu_avx512cd() {
        return;
    }
    for t in 0..64u32 {
        let mut words = [0u32; 16];
        for (i, w) in words.iter_mut().enumerate() {
            *w = t.wrapping_mul(0x9E37_79B9).wrapping_add((i as u32) << 13);
        }
        let r = unsafe { arch::lz_batch16(&words) };
        for g in 0..2 {
            let st: [u32; 8] = std::array::from_fn(|i| words[g * 8 + i]);
            assert_eq!(r[g], core_utils::lz_words_be(&st), "t={t} g={g}");
        }
    }
}

#[test]
fn pow_matches_reference_short_salt() {
    for (data, diff) in [
        (b"abc".as_slice(), 2u8),
        (b"anubis-test-12345".as_slice(), 2u8),
        (b"0123456789abcdef".as_slice(), 3u8),
    ] {
        let (n_ref, h_ref) = solve_ref(data, diff);
        let ctx = pow::PowCtx::new(data, diff);
        let width = if n_ref == 0 {
            1
        } else {
            (n_ref as f64).log10() as usize + 1
        };
        assert_eq!(
            ctx.digest(n_ref, width),
            h_ref,
            "data={:?} diff={}",
            data,
            diff
        );
        let (n, h) = pow::solve(data, diff, 1).expect("solve");
        assert_eq!(h, h_ref);
        assert_eq!(n, n_ref);
        let (n4, h4) = pow::solve(data, diff, 4).expect("solve t4");
        let mut msg = data.to_vec();
        msg.extend_from_slice(n4.to_string().as_bytes());
        assert_eq!(h4, sha_ref(&msg), "multithreaded nonce must stay valid");
    }
}

#[test]
fn pow_works_when_nonce_straddles_block_boundary() {
    let data: Vec<u8> = (0..63u8).collect();
    let (n_ref, h_ref) = solve_ref(&data, 2);
    assert!(
        n_ref >= 100,
        "reference nonce must have width>=3 to straddle"
    );
    let (n, h) = pow::solve(&data, 2, 1).expect("solve");
    assert_eq!(h, h_ref);
    assert_eq!(n, n_ref);
    let ctx = pow::PowCtx::new(&data, 2);
    let width = if n_ref == 0 {
        1
    } else {
        (n_ref as f64).log10() as usize + 1
    };
    assert!(ctx.tail_len() + width > 64, "must actually straddle");
    assert_eq!(ctx.digest(n_ref, width), h_ref);
}

#[test]
fn pow_zero_difficulty_returns_first_nonce() {
    let (n, _) = pow::solve(b"whatever", 0, 1).expect("solve");
    assert_eq!(n, 0);
}

#[test]
fn pow_multithread_finds_same_solution() {
    let data = b"parallel-consistency-check";
    let (n1, h1) = pow::solve(data, 4, 1).expect("solve t1");
    let (n2, h2) = pow::solve(data, 4, 4).expect("solve t4");
    assert_eq!(n1, n2);
    assert_eq!(h1, h2);
}

#[test]
fn pow_publish_race_stress_low_difficulty() {

    for rep in 0..100u32 {
        let data = format!("race-stress-{rep}");
        let diff = if rep % 2 == 0 { 1u8 } else { 2 };
        let (n, h) = pow::solve(data.as_bytes(), diff, 8).expect("solve");
        let mut msg = data.into_bytes();
        msg.extend_from_slice(n.to_string().as_bytes());
        assert_eq!(h, sha_ref(&msg), "rep {rep}: digest не соответствует nonce");
        let st = arch::state_of(&h);
        assert!(
            core_utils::lz_words_be(&st) >= diff as u32 * 4,
            "rep {rep}: nonce {n} не решает сложность"
        );
    }
}
