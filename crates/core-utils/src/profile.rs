use crate::rng::mix_ctx;
use crate::BytesExt as _;

pub const BLANK_HASH: u64 = 0x0B1A_4B1A_4C0D_E511;

#[inline(always)]
pub fn raster_seed(canvas_seed: u64, vendor: &[u8], renderer: &[u8]) -> u64 {
    let mut feed = [0u8; 160];
    let vl = vendor.len().min(feed.len() - 1);
    feed[..vl].copy_from_slice(&vendor[..vl]);
    feed[vl] = 0xFF;
    let rl = renderer.len().min(feed.len() - vl - 1);
    feed[vl + 1..vl + 1 + rl].copy_from_slice(&renderer[..rl]);
    crate::xxh3::hash_seeded(canvas_seed, &feed[..vl + 1 + rl])
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

pub fn canvas_hex_into(canvas_seed: u64, vendor: &[u8], renderer: &[u8], out: &mut [u8; 64]) {
    use md5::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(canvas_seed.to_le_bytes());
    h.update((vendor.len() as u64).to_le_bytes());
    h.update(vendor);
    h.update((renderer.len() as u64).to_le_bytes());
    h.update(renderer);
    let digest: [u8; 32] = h.finalize().into();
    digest.hex_lower_into(out);
}
