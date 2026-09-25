use compact_str::CompactString;
use core::fmt::Write as _;
use std::hash::{BuildHasherDefault, Hasher};

use crate::rng::SplitMix64Rng;

#[inline(always)]
pub fn u64_digits_into(mut uv: u64, buf: &mut [u8; 20]) -> usize {
    let mut i = buf.len();
    loop {
        i -= 1;
        let q = ((uv as u128 * 0xCCCC_CCCC_CCCC_CCCD) >> 67) as u64;
        unsafe {
            *buf.get_unchecked_mut(i) = b'0' + (uv - q * 10) as u8;
        }
        uv = q;
        if uv == 0 {
            break;
        }
    }
    i
}

#[inline]
pub fn push_int_into(out: &mut CompactString, v: i64) {
    let neg = v < 0;
    let mut buf = [0u8; 20];
    let i = u64_digits_into(v.unsigned_abs(), &mut buf);
    if neg {
        out.push('-');
    }
    out.push_str(unsafe { std::str::from_utf8_unchecked(buf.get_unchecked(i..)) });
}

#[inline]
pub fn int_to_compact(v: i64) -> CompactString {
    let mut s = CompactString::with_capacity(20);
    push_int_into(&mut s, v);
    s
}

#[inline]
pub fn float_to_compact(v: f64) -> CompactString {
    compact_str::ToCompactString::to_compact_string(&v)
}

#[inline]
pub fn format_js_float(v: f64) -> CompactString {
    use core::fmt::Write;
    if !v.is_finite() || v <= 0.0 {
        return CompactString::const_new("1");
    }
    let mut s = CompactString::with_capacity(16);
    let _ = write!(&mut s, "{v:.3}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s.is_empty() {
        s.push('1');
    }
    s
}

#[inline]
pub fn push_px_into(out: &mut CompactString, v: f64) {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e15 {
        push_int_into(out, v as i64);
    } else {
        let _ = write!(out, "{v}");
    }
}

#[inline]
pub fn px_to_compact(v: f64) -> CompactString {
    let mut s = CompactString::with_capacity(24);
    push_px_into(&mut s, v);
    s.push_str("px");
    s
}

#[inline]
pub fn floor_char_boundary(s: &str, limit: usize) -> usize {
    floor_char_boundary_bytes(s.as_bytes(), limit)
}

#[inline]
pub fn floor_char_boundary_bytes(s: &[u8], limit: usize) -> usize {
    if s.len() <= limit {
        return s.len();
    }
    let mut i = limit;
    while i > 0 && (s[i] & 0xC0) == 0x80 {
        i -= 1;
    }
    i
}

#[inline]
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y2 = y - u64::from(m <= 2) as i64;
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[inline]
pub fn year_from_days(z: i64) -> i32 {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    y as i32
}

pub fn civil_from_days(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[inline]
pub fn dow_from_days(days: i64) -> u8 {
    (days + 4).rem_euclid(7) as u8
}

#[inline]
pub fn days_in_month(y: i32, m: u8) -> u8 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
            u8::from(leap) + 28
        }
        _ => 30,
    }
}

#[inline]
pub fn truncate_str(s: &str, cap: usize) -> &str {
    let cut = floor_char_boundary(s, cap);
    &s[..cut]
}

#[inline(always)]
pub fn bump_u32_id(n: u32) -> u32 {
    n.wrapping_add(1).max(1)
}

#[inline(always)]
pub fn bump_u64_id(n: u64) -> u64 {
    n.wrapping_add(1).max(1)
}

pub fn push_int_padded_into(out: &mut CompactString, v: i64, width: usize) {
    let mag = 10u64.checked_pow(width as u32).unwrap_or(u64::MAX);
    let a = v.unsigned_abs();
    let mut buf = [0u8; 20];
    let start = u64_digits_into(a, &mut buf);
    let digits = buf.len() - start;
    if a < mag {
        if v < 0 {
            out.push('-');
        }
        let body = digits + usize::from(v < 0);
        for _ in body..width {
            out.push('0');
        }
    } else {
        out.push(if v < 0 { '-' } else { '+' });
    }
    out.push_str(unsafe { std::str::from_utf8_unchecked(buf.get_unchecked(start..)) });
}

const FX_K: u64 = 0x517c_c1b7_2722_0a95;

#[derive(Default)]
pub struct FxHasher64 {
    hash: u64,
}

impl FxHasher64 {
    #[inline(always)]
    fn add(&mut self, w: u64) {
        self.hash = (self.hash.rotate_left(5) ^ w).wrapping_mul(FX_K);
    }
}

impl Hasher for FxHasher64 {
    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for c in chunks {
            self.add(u64::from_le_bytes(*c));
        }
        if !rest.is_empty() {
            let mut w = [0u8; 8];
            w[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(w));
        }
    }

    #[inline(always)]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }

    #[inline(always)]
    fn write_u16(&mut self, i: u16) {
        self.add(i as u64);
    }

    #[inline(always)]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }

    #[inline(always)]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline(always)]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    #[inline(always)]
    fn finish(&self) -> u64 {
        self.hash.rotate_left(5)
    }
}

pub type FxBuild = BuildHasherDefault<FxHasher64>;

#[inline]
pub fn fx_map<K, V>() -> std::collections::HashMap<K, V, FxBuild> {
    std::collections::HashMap::default()
}
pub const MS_SEC: i64 = 1_000;
pub const MS_MIN: i64 = 60_000;
pub const MS_HOUR: i64 = 3_600_000;
pub const MS_DAY: i64 = 86_400_000;

pub fn epoch_ms_from_civil(y: i32, mo: u8, d: u8, h: u8, mi: u8, s: u8, ms: u16) -> i64 {
    let days = days_from_civil(i64::from(y), u32::from(mo), u32::from(d));
    days * MS_DAY
        + i64::from(h) * MS_HOUR
        + i64::from(mi) * MS_MIN
        + i64::from(s) * MS_SEC
        + i64::from(ms)
}
#[inline]
pub const fn u64_digits_fixed_into(mut n: u64, buf: &mut [u8; 20], width: usize) {
    debug_assert!(width <= buf.len());
    let mut i = width;
    while i > 0 {
        i -= 1;
        let q = ((n as u128 * 0xCCCC_CCCC_CCCC_CCCD) >> 67) as u64;
        buf[i] = b'0' + n.wrapping_sub(q.wrapping_mul(10)) as u8;
        n = q;
    }
}

#[inline(always)]
pub fn ou_step(v: f64, theta_dt: f64, diffusion: f64) -> f64 {
    v + (-theta_dt * v + diffusion)
}

#[inline(always)]
pub fn hypot2(dx: f64, dy: f64) -> f64 {
    (dx * dx + dy * dy).sqrt()
}

pub struct Perlin2D {
    perm: [u8; 512],
}

impl Perlin2D {
    pub fn from_seed(seed: u64) -> Self {
        let mut rng = SplitMix64Rng::new(seed);
        let mut p = [0u8; 256];
        for (i, slot) in p.iter_mut().enumerate() {
            *slot = i as u8;
        }
        for i in (1..256).rev() {
            let j = rng.next_below(i as u64 + 1) as usize;
            p.swap(i, j);
        }
        let mut perm = [0u8; 512];
        perm[..256].copy_from_slice(&p);
        perm[256..].copy_from_slice(&p);
        Self { perm }
    }

    #[inline]
    pub fn noise(&self, x: f64, y: f64) -> f64 {
        let fx = x.trunc();
        let fy = y.trunc();
        let xi = fx as i32 & 255;
        let yi = fy as i32 & 255;
        let xf = x - fx;
        let yf = y - fy;
        let u = fade(xf);
        let v = fade(yf);
        let base = self.perm[xi as usize] as usize;
        let base1 = self.perm[(xi as usize + 1) & 0xFF] as usize;
        let x1 = lerp(
            grad(self.perm[(base + yi as usize) & 0x1FF], xf, yf),
            grad(self.perm[(base1 + yi as usize) & 0x1FF], xf - 1.0, yf),
            u,
        );
        let x2 = lerp(
            grad(self.perm[(base + yi as usize + 1) & 0x1FF], xf, yf - 1.0),
            grad(
                self.perm[(base1 + yi as usize + 1) & 0x1FF],
                xf - 1.0,
                yf - 1.0,
            ),
            u,
        );
        lerp(x1, x2, v) * std::f64::consts::FRAC_1_SQRT_2
    }
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

#[inline]
fn grad(hash: u8, x: f64, y: f64) -> f64 {
    const GRADS: [(f64, f64); 12] = [
        (1.0, 1.0),
        (-1.0, 1.0),
        (1.0, -1.0),
        (-1.0, -1.0),
        (1.0, 0.0),
        (-1.0, 0.0),
        (1.0, 0.0),
        (-1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (0.0, 1.0),
        (0.0, -1.0),
    ];
    let (gx, gy) = GRADS[(hash % 12) as usize];
    gx * x + gy * y
}

#[derive(Clone, Copy, Debug)]
pub struct FastDivU64 {
    d: u64,
    m: u128,
}

impl FastDivU64 {
    #[inline]
    pub const fn new(d: u64) -> Self {
        debug_assert!(d != 0 && d < (1u64 << 31));
        let m = (1u128 << 64) / (d as u128) + (((1u128 << 64) % (d as u128)) != 0) as u128;
        Self { d, m }
    }

    #[inline(always)]
    pub const fn div(&self, n: u64) -> u64 {
        (((n as u128) * self.m) >> 64) as u64
    }

    #[inline(always)]
    pub const fn rem(&self, n: u64) -> u64 {
        n - self.div(n).wrapping_mul(self.d)
    }
}


pub mod bench {
    pub const CPU_SCALE_MIN: f64 = 0.5;
    pub const CPU_SCALE_MAX: f64 = 2.5;
    pub const JITTER_MID: f64 = 0.015;

    #[inline]
    pub fn bench_jitter(gauss: f64) -> f64 {
        (gauss * 0.0135 + JITTER_MID).clamp(-0.02, 0.05)
    }

    #[inline]
    pub fn scale_clamp(cpu_scale: f64) -> f64 {
        if cpu_scale.is_finite() {
            cpu_scale.clamp(CPU_SCALE_MIN, CPU_SCALE_MAX)
        } else {
            1.0
        }
    }

    #[inline]
    pub fn bench_scale(cpu_scale: f64, jitter: f64) -> f64 {
        if !cpu_scale.is_finite() {
            return 1.0;
        }
        scale_clamp(cpu_scale) * (1.0 + jitter)
    }
}

