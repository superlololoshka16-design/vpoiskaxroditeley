use core_utils::crypto::*;

fn hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

#[test]
fn blake2b_rfc7693_abc() {
    let mut out = [0u8; 64];
    blake2b(&mut out, 64, b"abc");
    assert_eq!(
        &hex(&out[..32]),
        "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d1"
    );

    let mut out = [0u8; 64];
    blake2b(&mut out, 64, b"");
    assert_eq!(
        &hex(&out[..32]),
        "786a02f742015903c6c6fd852552d272912f4740e15847618a86e217f71f5419"
    );
}

#[test]
fn blake2b_outlen_variants_and_contract() {
    let mut a = [0u8; 32];
    let mut b = [0u8; 64];
    blake2b(&mut a, 32, b"abc");
    blake2b(&mut b, 64, b"abc");
    assert_ne!(&a[..], &b[..32]);

    let mut c = [0u8; 5];
    blake2b(&mut c, 5, b"abc");
    assert_eq!(c.len(), 5);

    assert!(
        std::panic::catch_unwind(|| {
            let mut z = [0u8; 8];
            blake2b(&mut z, 0, b"x")
        })
        .is_err()
    );
    assert!(
        std::panic::catch_unwind(|| {
            let mut big = [0u8; 128];
            blake2b(&mut big, 65, b"x")
        })
        .is_err()
    );
}

#[test]
fn blake2b_long_boundaries_no_panic() {
    for n in [
        0usize, 1, 63, 64, 65, 127, 128, 1023, 1024, 1025, 4096, 65536,
    ] {
        let input: Vec<u8> = (0..n).map(|i| (i * 31 % 251) as u8).collect();
        let mut out = [0u8; 96];
        blake2b_long(&mut out, &input);
        assert!(out.iter().any(|&b| b != 0), "n={n}: вырожденный дайджест");
    }

    let input: Vec<u8> = (0..2000u32).map(|i| i as u8).collect();
    let mut x = [0u8; 96];
    let mut y = [0u8; 96];
    blake2b_long(&mut x, &input);
    blake2b_long(&mut y, &input);
    assert_eq!(x, y);
}

#[test]
fn blake2b_long_matches_direct_feed() {
    let input: Vec<u8> = (0..1500u32).map(|i| (i * 7 % 256) as u8).collect();
    let mut via_long = [0u8; 64];
    blake2b_long(&mut via_long, &input);
    let mut direct = [0u8; 64];
    let mut feed = Vec::with_capacity(4 + input.len());
    feed.extend_from_slice(&64u32.to_le_bytes());
    feed.extend_from_slice(&input);
    blake2b(&mut direct, 64, &feed);
    assert_eq!(via_long, direct);
}

#[test]
fn adler32_zlib_vectors() {
    assert_eq!(adler32_feed(1, b"Wikipedia"), 0x11E6_0398);
    assert_eq!(adler32_feed(1, b""), 1);
    assert_eq!(adler32_feed(1, b"a"), 0x0062_0062);

    let big = vec![b'a'; 5552];
    assert_eq!(adler32_feed(1, &big), adler32_feed(1, &big));
    let big2 = vec![b'a'; 5553];
    assert_ne!(adler32_feed(1, &big), adler32_feed(1, &big2));

    let x = vec![0xffu8; 5552];
    let y = vec![0xffu8; 5553];
    assert_ne!(adler32_feed(1, &x), adler32_feed(1, &y));
}

fn crc32_ref(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

#[test]
fn crc32_check_vector() {
    assert_eq!(crc32_ref(b"123456789"), 0xCBF4_3926);

    let mut expect_input = Vec::new();
    expect_input.extend_from_slice(&[1u8, 2, 3, 4]);
    expect_input.extend_from_slice(b"payload");
    let f = |tag: &[u8], data: &[u8]| !crc32_feed(crc32_feed(0xFFFF_FFFF, tag), data);
    assert_eq!(f(&[1, 2, 3, 4], b"payload"), crc32_ref(&expect_input));

    assert_eq!(f(&[0; 4], b""), crc32_ref(&[0, 0, 0, 0]));

    let long: Vec<u8> = (0..100u32).map(|i| (i * 37 % 256) as u8).collect();
    let mut expect_input = vec![9u8, 8, 7, 6];
    expect_input.extend_from_slice(&long);
    assert_eq!(f(&[9, 8, 7, 6], &long), crc32_ref(&expect_input));
}

#[test]
fn hmac_sha256_rfc4231() {
    let key = [0x0bu8; 20];
    let mut out = [0u8; 32];
    hmac_sha256_into(&key, b"Hi There", &mut out);
    assert_eq!(
        hex(&out),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );

    let mut out = [0u8; 32];
    hmac_sha256_into(b"Jefe", b"what do ya want for nothing?", &mut out);
    assert_eq!(
        hex(&out),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
}

#[test]
fn pbkdf2_sha256_vectors() {
    let mut out = [0u8; 32];
    pbkdf2_sha256_into(b"password", b"salt", 1, &mut out).unwrap();
    assert_eq!(
        hex(&out),
        "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
    );
    pbkdf2_sha256_into(b"password", b"salt", 2, &mut out).unwrap();
    assert_eq!(
        hex(&out),
        "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
    );

    assert!(matches!(
        pbkdf2_sha256_into(b"p", b"s", 0, &mut out),
        Err(KdfError::Iterations)
    ));
    assert!(matches!(
        pbkdf2_sha256_into(b"p", b"s", 10_000_000 + 1, &mut out),
        Err(KdfError::Iterations)
    ));
}

#[test]
fn sha256_matches_sha2_crate() {
    let mut seed = 0x243F_6A88_85A3_08D3u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed as u8
    };
    for len in [
        0usize, 1, 31, 32, 55, 56, 57, 63, 64, 65, 127, 128, 129, 1000,
    ] {
        let data: Vec<u8> = (0..len).map(|_| next()).collect();
        let own = sha256(&data);
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(&data);
        let expect: [u8; 32] = h.finalize().into();
        assert_eq!(own, expect, "len={len}");
    }
}

#[test]
fn sha256_seed_tail_streaming_equivalence() {
    let tail: Vec<u8> = (0..77u32).map(|i| (i * 13 % 256) as u8).collect();
    let own = sha256_seed_tail(0xDEAD_BEEF_CAFE_F00D, &tail);
    let mut feed = Vec::with_capacity(8 + tail.len());
    feed.extend_from_slice(&0xDEAD_BEEF_CAFE_F00Du64.to_le_bytes());
    feed.extend_from_slice(&tail);
    assert_eq!(own, sha256(&feed));

    let own = sha256_seed_tail(42, b"");
    assert_eq!(own, sha256(&42u64.to_le_bytes()));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn sha256_ni_cross_check_against_sha2() {
    if !is_x86_feature_detected!("sha") {
        return;
    }
    let mut seed = 0x1319_8A2E_0370_7344u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(2862933555777941757)
            .wrapping_add(3037000493);
        seed
    };
    const DATA_BLOCKS: usize = 6;
    let mut blocks = [[0u8; 64]; DATA_BLOCKS + 1];
    for b in &mut blocks[..DATA_BLOCKS] {
        for w in b.as_chunks_mut::<8>().0 {
            w.copy_from_slice(&next().to_le_bytes());
        }
    }

    let pad = &mut blocks[DATA_BLOCKS];
    pad[0] = 0x80;
    let bit_len = (DATA_BLOCKS * 64 * 8) as u64;
    pad[56..].copy_from_slice(&bit_len.to_be_bytes());

    let mut st = H0;
    unsafe {
        for b in &blocks {
            sha256_block_ni(&mut st, b);
        }
    }
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for b in &blocks[..DATA_BLOCKS] {
        h.update(b);
    }
    let expect: [u8; 32] = h.finalize().into();
    assert_eq!(digest_bytes(&st), expect);

    let mut seq0 = H0;
    unsafe {
        sha256_block_ni(&mut seq0, &blocks[0]);
    }
    let mut seq1 = H0;
    unsafe {
        sha256_block_ni(&mut seq1, &blocks[1]);
    }
    let mut d0 = H0;
    let mut d1 = H0;
    unsafe {
        sha256_block_ni_2x(&mut d0, &mut d1, &blocks[0], &blocks[1]);
    }
    assert_eq!(d0, seq0, "2x lane 0 must match sequential compression");
    assert_eq!(d1, seq1, "2x lane 1 must match sequential compression");
}

#[test]
fn sha1_known_vector() {
    let mut out = [0u8; 20];
    sha1_into(b"abc", &mut out);
    let mut hex = [0u8; 40];
    hex::encode_to_slice(out, &mut hex).unwrap();
    assert_eq!(&hex, b"a9993e364706816aba3e25717850c26c9cd0d89d");
}
