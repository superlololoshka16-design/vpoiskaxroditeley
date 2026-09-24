use std::sync::atomic::Ordering;

pub const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;
pub const SPLITMIX_M1: u64 = 0xBF58_476D_1CE4_E5B9;
pub const SPLITMIX_M2: u64 = 0x94D0_49BB_1331_11EB;

pub trait U64Ext {
    fn unit(self) -> f64;
    fn mix(self) -> u64;
}

impl U64Ext for u64 {
    #[inline(always)]
    fn unit(self) -> f64 {
        (self >> 11) as f64 / 9007199254740992.0
    }

    #[inline(always)]
    fn mix(mut self) -> u64 {
        self = (self ^ (self >> 30)).wrapping_mul(SPLITMIX_M1);
        self = (self ^ (self >> 27)).wrapping_mul(SPLITMIX_M2);
        self ^ (self >> 31)
    }
}

#[inline(always)]
pub fn u64_unit(x: u64) -> f64 {
    x.unit()
}

#[inline(always)]
pub fn splitmix_mix(z: u64) -> u64 {
    z.mix()
}

#[inline(always)]
pub fn mulhi_bounded(v: u64, n: u64) -> u64 {
    (((v as u128) * (n as u128)) >> 64) as u64
}

#[inline(always)]
pub fn mix_ctx(seed: u64, ctx_id: u64) -> u64 {
    seed ^ ctx_id.wrapping_mul(GOLDEN)
}

#[inline(always)]
pub fn mix64(x: u64) -> u64 {
    (x ^ (x >> 33)).wrapping_mul(0xFF51_AFD7_ED55_8CCD)
}

#[derive(Debug, Clone, Copy)]
pub struct SplitMix64Rng {
    state: u64,
    spare_gauss: f64,
}

impl SplitMix64Rng {
    #[inline]
    pub const fn new(seed: u64) -> Self {
        Self {
            state: seed,
            spare_gauss: f64::NAN,
        }
    }

    #[inline]
    pub const fn stepped(mut self, n: u64) -> Self {
        self.state = self.state.wrapping_add(n.wrapping_mul(GOLDEN));
        self
    }

    #[inline(always)]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN);
        self.state.mix()
    }

    #[inline(always)]
    pub fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    #[inline(always)]
    pub fn next_f64(&mut self) -> f64 {
        self.next_u64().unit()
    }

    #[inline]
    pub fn next_range(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        lo + mulhi_bounded(self.next_u64(), u64::from(hi - lo)) as u32
    }

    #[inline]
    pub fn next_below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        mulhi_bounded(self.next_u64(), n)
    }

    #[inline(always)]
    pub fn chance(&mut self, p: f64) -> bool {
        self.next_f64() < p
    }

    #[inline]
    pub fn gauss(&mut self) -> f64 {
        if self.spare_gauss.is_finite() {
            let v = self.spare_gauss;
            self.spare_gauss = f64::NAN;
            return v;
        }
        let (a, b) = gauss_polar_pair(|| self.next_f64());
        self.spare_gauss = b;
        a
    }

    #[inline]
    pub fn lognormal_ms(&mut self, median_ms: f64, sigma_ln: f64) -> u32 {
        let v = (median_ms.ln() + self.gauss() * sigma_ln).exp();
        v.clamp(1.0, 65_535.0) as u32
    }

    #[inline]
    pub fn lognormal_us(&mut self, median_ms: f64, sigma_ln: f64) -> u64 {
        u64::from(self.lognormal_ms(median_ms, sigma_ln)) * 1000
    }
}

#[inline]
fn gauss_polar_pair(mut draw: impl FnMut() -> f64) -> (f64, f64) {
    loop {
        let x = draw() * 2.0 - 1.0;
        let y = draw() * 2.0 - 1.0;
        let s = x * x + y * y;
        if s > 0.0 && s < 1.0 {
            let f = (-2.0 * s.ln() / s).sqrt();
            return (x * f, y * f);
        }
    }
}

#[inline(always)]
pub fn atomic_splitmix_step(cell: &std::sync::atomic::AtomicU64) -> u64 {
    let s = cell.fetch_add(GOLDEN, Ordering::Relaxed);
    s.wrapping_add(GOLDEN).mix()
}

pub struct AtomicSplitMix {
    state: std::sync::atomic::AtomicU64,
}

impl AtomicSplitMix {
    #[inline]
    pub const fn new(seed: u64) -> Self {
        Self {
            state: std::sync::atomic::AtomicU64::new(seed),
        }
    }

    #[inline(always)]
    pub fn next_u64(&self) -> u64 {
        atomic_splitmix_step(&self.state)
    }

    #[inline(always)]
    pub fn next_f64(&self) -> f64 {
        self.next_u64().unit()
    }
}

#[inline(always)]
pub fn mix_to_range(seed: u64, salt: u64, bound: u64) -> u64 {
    if bound == 0 {
        return 0;
    }
    let mix = seed
        .wrapping_mul(GOLDEN)
        .wrapping_add(salt.wrapping_mul(SPLITMIX_M1));
    (((mix >> 32) as u128 * (bound as u128)) >> 32) as u64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity(pub u64);

impl Identity {
    #[inline(always)]
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    #[inline(always)]
    pub fn at(self, salt: u64) -> Identity {
        Identity(self.0.mix() ^ salt.wrapping_mul(GOLDEN))
    }

    #[inline(always)]
    pub fn raw(self) -> u64 {
        self.0.mix()
    }

    #[inline(always)]
    pub fn u64_in(self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        lo + mulhi_bounded(self.raw(), hi - lo)
    }

    #[inline(always)]
    pub fn u32_in(self, lo: u32, hi: u32) -> u32 {
        self.u64_in(u64::from(lo), u64::from(hi)) as u32
    }

    #[inline(always)]
    pub fn f64_unit(self) -> f64 {
        self.raw().unit()
    }

    #[inline(always)]
    pub fn f64_in(self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f64_unit()
    }

    #[inline(always)]
    pub fn chance(self, p: f64) -> bool {
        self.f64_unit() < p
    }

    #[inline(always)]
    pub fn pick<T>(self, items: &[T]) -> &T {
        &items[mulhi_bounded(self.raw(), items.len() as u64) as usize]
    }
}

#[inline(always)]
pub fn ident(seed: u64, salt: u64) -> Identity {
    Identity::new(seed).at(salt)
}

#[derive(Debug, Clone, Copy)]
pub struct Rng {
    s: [u64; 4],
    spare_gauss: f64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut sm = SplitMix64Rng::new(seed).stepped(GOLDEN);
        let s = std::array::from_fn(|_| sm.next_u64());
        if s.iter().all(|&v| v == 0) {
            return Self {
                s: [1, 2, 4, 8],
                spare_gauss: f64::NAN,
            };
        }
        Self {
            s,
            spare_gauss: f64::NAN,
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    #[inline(always)]
    pub fn next_f64(&mut self) -> f64 {
        self.next_u64().unit()
    }

    #[inline]
    pub fn gauss(&mut self) -> f64 {
        if self.spare_gauss.is_finite() {
            let v = self.spare_gauss;
            self.spare_gauss = f64::NAN;
            return v;
        }
        let (a, b) = self.gauss_pair();
        self.spare_gauss = b;
        a
    }

    #[inline]
    fn gauss_pair(&mut self) -> (f64, f64) {
        gauss_polar_pair(|| self.next_f64())
    }

    #[inline]
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        mulhi_bounded(self.next_u64(), n as u64) as usize
    }
}

pub mod seeds {
    pub const SALT_TASKBAR: u64 = 0x7461_736B_6261_7201;
    pub const SALT_CHROME: u64 = 0x6368_726F_6D65_0202;
    pub const SALT_BOOKMARKS: u64 = 0x626F_6F6B_6D61_726B;
    pub const SALT_PERSONA: u64 = 0x0C0D_0E0F_0809_0A0B;
    pub const SALT_HOP: u64 = 0x0405_0607_0001_0203;
    pub const SALT_BATCH_INTERVAL: u64 = 0x6274_6368_7573_0001;
    pub const SALT_TIER: u64 = 0xA54F_F53A_5F1D_36F1;
    pub const SALT_SKILL: u64 = 0x9B05_688C_2B3E_6C1F;
    pub const SALT_SESSION_RNG: u64 = 0x1DE1_0000_0000_0005;
    pub const SALT_IDLE_ENV: u64 = 0x1D1E_0F00_0000_0021;
    pub const SALT_IDLE_MOTOR_RNG: u64 = 0x1DE1_1D1E_0000_0011;
    pub const SALT_TAB_SESSION_RNG: u64 = 0x7E11_0000_0000_0037;
    pub const SALT_PLACEMENT: u64 = 0x7A3F_C0DE_5EED_8101;
    pub const SALT_INBOUND: u64 = 0x1DE1_5C21_0000_0009;
    pub const SALT_INTERACTION_RNG: u64 = 0x1DE1_5C21_0000_0007;
    pub const SALT_CLICK_OFFSET_TAB: u64 = 0x0F0C_5EED_0000_0021;
    pub const SALT_INTERACTION_TARGET: u64 = 0xF0C_5EED_0000_0023;
    pub const SALT_FOCUS_CURSOR: u64 = 0xF0C_05EED_0000_0007;
    pub const SALT_FOCUS_WINDOW: u64 = 0xF0C_05EED_0000_0009;
    pub const SALT_CLICK_CURSOR: u64 = 0x0C1C_5EED_0000_0002;
    pub const SALT_CLICK_PLAN: u64 = 0x0C1C_0000_0000_0055;
    pub const SALT_SCROLL_CURSOR: u64 = 0x5C20_0000_0000_0000;
    pub const SALT_TREMOR_NOISE: u64 = 0x7E11_0000_C0FF_EEEE;
    pub const SALT_MOTION_RNG: u64 = 0x1D2B_A3C4_5E6F_7081;
    pub const SALT_TYPING_RNG: u64 = 0x7966_6557_4E45_4759;
    pub const SALT_FOCUS_ENTER: u64 = 0xF0C_5EED_0000_0017;
    pub const SALT_CLICK_ENTER: u64 = 0xC1C_0000_0000_0031;
    pub const SALT_SCROLL_ENTER: u64 = 0x5C20_0000_0000_0041;
    pub const SALT_CLICK_OFFSET_BASE: u64 = 0xBB67_AE85_84CA_A73B;
    pub const SALT_AUDIO_ROOT: u64 = 0xA0D1_0F1F_0000_0124;
    pub const SALT_AUDIO_CHANNEL: u64 = 0x06;
    pub const SALT_WEBGL_ROOT: u64 = 0x5745_4247_4C53_4649;
    pub const SALT_WEBGL_PARAM: u64 = 0x5745_4247_4C53_4C4F;
    pub const SALT_PIXEL_CHAN: u64 = 0x1656_67B1_9E37_79F9;
    pub const SALT_GLYPH: u64 = 0xD1CE_C0DE_5EED;
    pub const SALT_MEM_BASE: u64 = 0x4D45_4D42_4153_4531;
    pub const SALT_GLYPH_SCALE: u64 = 0x66_6F_6E_74_5F_73;
    pub const SALT_FARBLE: u64 = 0xC2B2_AE3D_27D4_EB4F;
    pub const SALT_FITTS_A: u64 = 0x0F1A_0000_0000_0001;
    pub const SALT_FITTS_B: u64 = 0x0F1A_0000_0000_0002;
    pub const SALT_GRAVITY: u64 = 0x0F1A_0000_0000_0003;
    pub const SALT_WIND: u64 = 0x0F1A_0000_0000_0004;
    pub const SALT_MAX_STEP: u64 = 0x0F1A_0000_0000_0005;
    pub const SALT_WIND_JITTER: u64 = 0x0F1A_0000_0000_0006;
    pub const SALT_TREMOR_AMP: u64 = 0x0F1A_0000_0000_0007;
    pub const SALT_TREMOR_HZ: u64 = 0x0F1A_0000_0000_0008;
    pub const SALT_OVERSHOOT_P: u64 = 0x0F1A_0000_0000_0009;
    pub const SALT_CORRECTIONS: u64 = 0x0F1A_0000_0000_000A;
    pub const SALT_DWELL_MEDIAN: u64 = 0x0F1A_0000_0000_000B;
    pub const SALT_DWELL_SIGMA: u64 = 0x0F1A_0000_0000_000C;
    pub const SALT_WPM: u64 = 0x0F1A_0000_0000_000D;
    pub const SALT_TYPING_SIGMA: u64 = 0x0F1A_0000_0000_000E;
    pub const SALT_BURST_LEN: u64 = 0x0F1A_0000_0000_000F;
    pub const SALT_BURST_PAUSE: u64 = 0x0F1A_0000_0000_0010;
    pub const SALT_BACKSPACE_P: u64 = 0x0F1A_0000_0000_0011;
    pub const SALT_CAPS_ERROR_P: u64 = 0x0F1A_0000_0000_0012;
    pub const SALT_NOTCH_PX: u64 = 0x0F1A_0000_0000_0013;
    pub const SALT_NOTCH_FRICTION: u64 = 0x0F1A_0000_0000_0014;
    pub const SALT_NOTCH_GAP: u64 = 0x0F1A_0000_0000_0015;
    pub const SALT_OVERSCROLL_P: u64 = 0x0F1A_0000_0000_0016;
    pub const SALT_READING_PAUSE_P: u64 = 0x0F1A_0000_0000_0017;
    pub const SALT_DOUBLE_CLICK_P: u64 = 0x0F1A_0000_0000_0018;
    pub const SALT_RIGHT_CLICK_P: u64 = 0x0F1A_0000_0000_0019;
    pub const SALT_HOVER_P: u64 = 0x0F1A_0000_0000_001A;
    pub const SALT_IDLE_THETA: u64 = 0x0F1A_0000_0000_001B;
    pub const SALT_IDLE_NOISE: u64 = 0x0F1A_0000_0000_001C;
    pub const SALT_IDLE_HOP_W: u64 = 0x0F1A_0000_0000_001D;
    pub const SALT_IDLE_SCROLL_P: u64 = 0x0F1A_0000_0000_001E;
    pub const SALT_IDLE_ANCHOR_PULL: u64 = 0x0F1A_0000_0000_001F;
    pub const SALT_IDLE_ANCHOR_WANDER: u64 = 0x0F1A_0000_0000_0020;
    pub const SALT_HOP_INTERVAL: u64 = 0x0F1A_0000_0000_0021;
    pub const SALT_READING_SCROLL: u64 = 0x0F1A_0000_0000_0022;
    pub const SALT_REACTION_SIGMA: u64 = 0x0F1A_0000_0000_0023;
    pub const SALT_ICE_UFRAG: u64 = 0x1111_0000_CAFE_BABE;
    pub const SALT_ICE_PWD: u64 = 0xDEAD_BEEF_F00D_0001;
    pub const SALT_CRYPTO_RESEED: u64 = 0x0C0D_0E0F_0809_0A0B;
}
