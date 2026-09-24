use argon2::{Algorithm, Argon2, Params, Version};
use challenge_solver::argon::ArgonCtx;

fn ref_tag(pw: &[u8], salt: &[u8], m: u32, t: u32) -> [u8; 32] {
    let a = Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(m, t, 1, Some(32)).unwrap(),
    );
    let mut out = [0u8; 32];
    a.hash_password_into(pw, salt, &mut out).unwrap();
    out
}

#[test]
fn argon2id_rfc_shape_matches_reference_crate() {
    let salt = b"somesalt";
    for (m, t) in [(32u32, 3u32), (48, 3), (8, 1), (64, 2)] {
        let ctx = ArgonCtx::new(salt, 0, m, t);
        let mut mem = vec![[0u64; 128]; ctx.m_prime() as usize];
        for nonce in [0u64, 1, 42, 999] {
            let width = if nonce == 0 {
                1
            } else {
                (nonce as f64).log10() as usize + 1
            };
            let mine = ctx.digest(nonce, width, &mut mem);
            let mut pw = salt.to_vec();
            pw.extend_from_slice(nonce.to_string().as_bytes());
            assert_eq!(
                mine,
                ref_tag(&pw, salt, m, t),
                "m={m} t={t} nonce={nonce}"
            );
        }
    }
}
