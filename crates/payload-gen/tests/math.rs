use core_utils::rng::Rng;
use core_utils::xxh3;
use core_utils::{md5_hex_into, sha1_into, sha256_hex_into};
use payload_gen::webgl_param;
use session_state::{NetKind, asn_info};

#[test]
fn sha256_known_vector() {
    let mut out = [0u8; 64];
    sha256_hex_into(b"abc", &mut out);
    assert_eq!(
        &out,
        b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn md5_known_vector() {
    let mut out = [0u8; 32];
    md5_hex_into(b"abc", &mut out);
    assert_eq!(&out, b"900150983cd24fb0d6963f7d28e17f72");
}

#[test]
fn canvas_hash_deterministic_and_profile_sensitive() {
    let hex = |seed: u64, v: &str, r: &str| -> String {
        let mut buf = [0u8; 64];
        core_utils::profile::canvas_hex_into(seed, v.as_bytes(), r.as_bytes(), &mut buf);
        String::from_utf8(buf.to_vec()).unwrap()
    };
    let a = hex(7, "Google Inc.", "ANGLE (NVIDIA)");
    let b = hex(7, "Google Inc.", "ANGLE (NVIDIA)");
    let c = hex(8, "Google Inc.", "ANGLE (NVIDIA)");
    let d = hex(7, "Google Inc.", "ANGLE (Intel)");
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, d);
    assert_eq!(a.len(), 64);
    assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(webgl_param(7, 0) >= 1.0 && webgl_param(7, 0) <= 2048.0);
    assert_ne!(webgl_param(7, 0), webgl_param(9, 0));
}

#[test]
fn xxh3_stable_and_seeded() {
    assert_eq!(xxh3::hash(b"silo"), xxh3::hash(b"silo"));
    assert_ne!(xxh3::hash(b"silo"), xxh3::hash(b"siol"));
    assert_ne!(xxh3::hash_seeded(1, b"silo"), xxh3::hash_seeded(2, b"silo"));
}

#[test]
fn asn_table_maps_kinds() {
    assert_eq!(asn_info(15169).net, NetKind::Datacenter);
    assert_eq!(asn_info(7922).net, NetKind::Residential);
    assert_eq!(asn_info(7018).net, NetKind::Mobile);
    assert_eq!(asn_info(999999).net, NetKind::Residential);
    assert_eq!(asn_info(3209).locale, "de-DE");
}

#[test]
fn rng_distribution_sane() {
    let mut rng = Rng::new(7);
    let mut ones = 0u32;
    for _ in 0..100_000 {
        if rng.next_f64() < 0.5 {
            ones += 1;
        }
    }
    let ratio = ones as f64 / 100_000.0;
    assert!((ratio - 0.5).abs() < 0.02, "coin flip ratio {ratio}");
    let g: f64 = (0..10_000).map(|_| rng.gauss()).sum::<f64>() / 10_000.0;
    assert!(g.abs() < 0.1, "gauss mean drifted: {g}");
}

#[test]
fn png_data_url_is_structurally_valid() {
    let mut px = vec![0u8; 8 * 4 * 4];
    payload_gen::fill_pixels(&mut px, 8, 12345, 999);
    let url = payload_gen::png_data_url_pixels(8, 4, &px);
    assert!(url.starts_with("data:image/png;base64,"));
    let b64 = &url["data:image/png;base64,".len()..];
    let bytes = base64_turbo::STANDARD.decode(b64.as_bytes()).expect("b64");
    assert_eq!(
        &bytes[..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
    );
    assert_eq!(&bytes[12..16], b"IHDR");
    assert_eq!(&bytes[16..20], &8u32.to_be_bytes());
    assert_eq!(&bytes[20..24], &4u32.to_be_bytes());
    assert!(bytes.windows(4).any(|w| w == b"IDAT"));
    assert!(bytes.windows(4).any(|w| w == b"IEND"));
}

#[test]
fn png_data_url_draw_hash_sensitive_and_stable() {
    let a = seed_url(6, 6, 77, 1);
    let b = seed_url(6, 6, 77, 1);
    let c = seed_url(6, 6, 77, 2);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

fn seed_url(w: u32, h: u32, seed: u64, draw_hash: u64) -> String {
    let mut px = vec![0u8; w as usize * h as usize * 4];
    payload_gen::fill_pixels(&mut px, w, seed, draw_hash);
    payload_gen::png_data_url_pixels(w, h, &px)
}

#[test]
fn pixel_at_alpha_is_opaque_and_channel_distinct() {
    assert_eq!(payload_gen::pixel_at(1, 1, 0, 0, 3), 255);
    let r = payload_gen::pixel_at(1, 1, 5, 5, 0);
    let g = payload_gen::pixel_at(1, 1, 5, 5, 1);
    let b2 = payload_gen::pixel_at(1, 1, 5, 5, 2);
    assert!(r != g || g != b2);
}

#[test]
fn audio_fp_lands_in_chrome_band() {
    let v = payload_gen::audio_fp(42);
    assert!((124.0..124.1).contains(&v), "audio fp out of band: {v}");
    assert_eq!(v, payload_gen::audio_fp(42));
}

#[test]
fn canvas_cost_is_scaled_and_positive() {
    let base = payload_gen::canvas_time_cost_us(payload_gen::CANVAS_OP_TO_URL, 1.0, 0.0);
    let slow = payload_gen::canvas_time_cost_us(payload_gen::CANVAS_OP_TO_URL, 1.5, 0.0);
    assert!(base > 0);
    assert!(slow > base);
}

#[test]
fn sha1_known_vector() {
    let mut out = [0u8; 20];
    sha1_into(b"abc", &mut out);
    let mut hex = [0u8; 40];
    hex::encode_to_slice(out, &mut hex).unwrap();
    assert_eq!(&hex, b"a9993e364706816aba3e25717850c26c9cd0d89d");
}
