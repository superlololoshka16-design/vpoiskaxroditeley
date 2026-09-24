pub mod crypto;
pub mod encoding;
pub mod math;
pub mod rng;
pub mod sys;
pub mod tz;
pub mod url;
pub use crypto::{
    BLAKE2B_G_COLS, H0, K32, KdfError, PBKDF2_MAX_ITERATIONS, SUBTLE_MAX_OUT_BYTES,
    adler32_feed, adler32_zeros, be32_words, blake2b, blake2b_long, canvas_hex_of, compress8,
    cpu_avx2, cpu_avx512cd, cpu_sha, crc32_feed, digest_bytes, hmac_sha256_into, lz_batch16, lz_batch16_ref,
    lz_carry_chain, lz_words_be, md5_hex_into, pair_mut, pbkdf2_sha256_into, sha1_into, sha256,
    sha256_block, sha256_hex_into, sha256_into, sha256_midstate, sha256_seed_tail, sha_tail_pad,
    state_of, words_be32,
};
#[cfg(target_arch = "x86_64")]
pub use crypto::{sha256_block_ni, sha256_block_ni_2x};
pub use encoding::{
    B64Error, B64_STANDARD, BytesExt, ascii_lower_byte, ascii_lower_compact, ascii_upper_byte,
    ascii_upper_compact, b64_decode_cap, b64_encoded_len, form_urlencoded_decode,
    form_urlencoded_encode, hex_compact, hex_grouped, hex_val, percent_decode, percent_decode_cow,
    percent_encode_compact,
    percent_encode_into, push_ascii_case_into,
};
pub use math::bench;
pub use math::{
    FastDivU64, FxBuild, FxHasher64, MS_DAY, MS_HOUR, MS_MIN, MS_SEC, Perlin2D, bump_u32_id,
    bump_u64_id, civil_from_days, days_from_civil, days_in_month, dow_from_days,
    epoch_ms_from_civil, float_to_compact, floor_char_boundary, floor_char_boundary_bytes,
    format_js_float,
    fx_map, int_to_compact, ou_step, push_int_into, push_int_padded_into, push_px_into,
    px_to_compact, truncate_str, u64_digits_fixed_into, u64_digits_into, year_from_days,
};
pub use rng::{
    GOLDEN, SPLITMIX_M1, SPLITMIX_M2, AtomicSplitMix, Identity, Rng, SplitMix64Rng, U64Ext,
    atomic_splitmix_step, ident, mix_ctx, mix_to_range, mulhi_bounded, splitmix_mix,
    u64_unit,
};
pub use sys::{env_flag, env_present, ms, pin_thread, unix_ms, unix_ms_f64, unix_us};
pub use tz::{TzIdx, all_zone_names, country_of, tz_offset_for, tz_offset_zone, zone_of, zone_label_zone};
pub use url::{
    Authority, HrefParts, StrExt, host_of, host_of_into, join_origin, origin_of, path_of,
    split_authority, split_href,
};

pub use simdutf8 as utf8;
pub use sonic_rs as json;

pub mod xxh3 {
    pub use twox_hash::XxHash3_64;

    #[inline(always)]
    pub fn hash(data: &[u8]) -> u64 {
        XxHash3_64::oneshot(data)
    }

    #[inline(always)]
    pub fn hash_seeded(seed: u64, data: &[u8]) -> u64 {
        XxHash3_64::oneshot_with_seed(seed, data)
    }

    #[inline]
    pub fn hash_seeded_tail(seed: u64, tail: &[u8]) -> u64 {
        crate::crypto::xxh3_seed_tail(seed, tail)
    }
}
