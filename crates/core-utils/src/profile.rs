use std::hash::Hasher as _;

use crate::rng::mix_ctx;

pub const BLANK_HASH: u64 = 0x0B1A_4B1A_4C0D_E511;

#[inline(always)]
pub fn raster_seed(canvas_seed: u64, vendor: &[u8], renderer: &[u8]) -> u64 {
    let mut h = crate::xxh3::XxHash3_64::new();
    h.write(&canvas_seed.to_le_bytes());
    h.write(vendor);
    h.write(&[0xFF]);
    h.write(renderer);
    h.finish()
}

#[inline(always)]
pub fn canvas_seed_of(canvas_id: u32, raster_seed: u64) -> u64 {
    mix_ctx(raster_seed, u64::from(canvas_id))
}

#[inline(always)]
pub fn gl_seed_of(canvas_seed: u64, canvas_id: u32, epoch: u32) -> u64 {
    mix_ctx(mix_ctx(canvas_seed, u64::from(canvas_id)), u64::from(epoch))
}

#[inline(always)]
pub fn draw_hash(hasher_state: u64, ops: u32, blank: bool) -> u64 {
    if blank {
        BLANK_HASH
    } else {
        hasher_state.wrapping_add(u64::from(ops))
    }
}

pub const CANVAS_COST_MS: [f64; 5] = [0.0016, 0.0042, 0.028, 0.055, 0.0009];

#[inline]
pub fn canvas_time_cost_us(op: u8, cpu_scale: f64, gauss: f64) -> u64 {
    let i = usize::from(op) % CANVAS_COST_MS.len();
    let jitter = crate::math::bench::bench_jitter(gauss);
    let ms = (CANVAS_COST_MS[i] * crate::math::bench::bench_scale(cpu_scale, jitter)).max(0.0001);
    (ms * 1000.0).max(1.0) as u64
}

pub const BASE_HASHES_PER_MS: f64 = 3000.0;
pub const POW_OVERHEAD_BASE_MS: f64 = 25.0;
pub const POW_JITTER_SIGMA: f64 = 0.05;

#[inline]
pub fn pow_elapsed_ms(attempts: u64, cpu_scale: f64, jitter: f64) -> f64 {
    let scale = crate::math::bench::scale_clamp(cpu_scale);
    let calc_ms = attempts as f64 * scale / BASE_HASHES_PER_MS;
    let jitter = jitter.clamp(-2.0, 2.0);
    (calc_ms + POW_OVERHEAD_BASE_MS) * (1.0 + POW_JITTER_SIGMA * jitter)
}

pub const CLICK_PRE_PRESS_MEDIAN_MS: f64 = 40.0;
pub const CLICK_PRE_PRESS_SIGMA: f64 = 0.3;
pub const CLICK_GAP_MEDIAN_MS: f64 = 70.0;
pub const CLICK_GAP_SIGMA: f64 = 0.35;
pub const CLICK_DOUBLE_GAP_MEDIAN_MS: f64 = 110.0;
pub const CLICK_DOUBLE_GAP_SIGMA: f64 = 0.25;
pub const SCROLL_NOTCH_GAP_SIGMA: f64 = 0.30;

pub fn canvas_hex_into(canvas_seed: u64, vendor: &[u8], renderer: &[u8], out: &mut [u8; 64]) {
    let mut st = crate::crypto::H0;
    let mut off = 0usize;
    let mut blk = [0u8; 64];
    let mut total = 0usize;
    let mut feed = |chunk: &[u8],
                    st: &mut [u32; 8],
                    off: &mut usize,
                    total: &mut usize,
                    blk: &mut [u8; 64]| {
        let mut rest = chunk;
        while !rest.is_empty() {
            let take = rest.len().min(64 - *off);
            blk[*off..*off + take].copy_from_slice(&rest[..take]);
            *off += take;
            *total += take;
            rest = &rest[take..];
            if *off == 64 {
                crate::crypto::sha256_block(st, blk);
                *blk = [0u8; 64];
                *off = 0;
            }
        }
    };
    feed(&canvas_seed.to_le_bytes(), &mut st, &mut off, &mut total, &mut blk);
    feed(
        &(vendor.len() as u64).to_le_bytes(),
        &mut st,
        &mut off,
        &mut total,
        &mut blk,
    );
    feed(vendor, &mut st, &mut off, &mut total, &mut blk);
    feed(
        &(renderer.len() as u64).to_le_bytes(),
        &mut st,
        &mut off,
        &mut total,
        &mut blk,
    );
    feed(renderer, &mut st, &mut off, &mut total, &mut blk);
    let total_bits = (total as u64) * 8;
    blk[off] = 0x80;
    if off < 56 {
        blk[56..64].copy_from_slice(&total_bits.to_be_bytes());
        crate::crypto::sha256_block(&mut st, &blk);
    } else {
        crate::crypto::sha256_block(&mut st, &blk);
        let mut tail = [0u8; 64];
        tail[56..64].copy_from_slice(&total_bits.to_be_bytes());
        crate::crypto::sha256_block(&mut st, &tail);
    }
    let digest = crate::crypto::words_be32(&st);
    let hex = b"0123456789abcdef";
    for (i, b) in digest.iter().enumerate() {
        out[i * 2] = hex[usize::from(b >> 4)];
        out[i * 2 + 1] = hex[usize::from(b & 0xF)];
    }
}
