use core_utils::rng::{SplitMix64Rng, seeds};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Gamer,
    Office,
}

#[derive(Debug, Clone, Copy)]
pub struct Persona {
    pub tier: Tier,
    pub skill: f64,
    pub fitts_a_ms: f64,
    pub fitts_b_ms: f64,
    pub gravity: f64,
    pub wind: f64,
    pub max_step: f64,
    pub wind_jitter_ln: f64,
    pub tremor_amp_px: f64,
    pub tremor_hz: f64,
    pub overshoot_p: f64,
    pub corrections_max: u8,
    pub dwell_median_ms: f64,
    pub dwell_sigma_ln: f64,
    pub wpm: f64,
    pub typing_sigma_ln: f64,
    pub burst_len: u32,
    pub burst_pause_median_ms: f64,
    pub backspace_p: f64,
    pub caps_error_p: f64,
    pub notch_px: f64,
    pub notch_friction: f64,
    pub notch_gap_median_ms: f64,
    pub overscroll_p: f64,
    pub reading_pause_p: f64,
    pub double_click_p: f64,
    pub right_click_p: f64,
    pub hover_p: f64,
    pub idle_theta: f64,
    pub idle_noise_px_s: f64,
    pub idle_hop_w_px: f64,
    pub idle_scroll_p: f64,
    pub idle_anchor_pull: f64,
    pub idle_anchor_wander: f64,
    pub hop_interval_median_ms: f64,
    pub reading_scroll_px: f64,
    pub reaction_sigma_ln: f64,
}

#[inline]
fn sample_band(b: (f64, f64), u: f64, rng: &mut SplitMix64Rng) -> f64 {
    b.0 + (b.1 - b.0) * (u + rng.gauss() * 0.10).clamp(0.0, 1.0)
}

impl Persona {
    pub fn derive(seed: u64) -> Self {
        let root = core_utils::Identity::new(seed).at(seeds::SALT_PERSONA);
        let skill = root.at(seeds::SALT_SKILL).f64_unit();
        let n = |salt: u64| root.at(salt).f64_unit();
        let gamer = root.at(seeds::SALT_TIER).f64_unit() < 0.22;
        let (tier, band) = if gamer {
            (Tier::Gamer, 0.0f64)
        } else {
            (Tier::Office, 0.5f64)
        };
        let half = |lo: f64, hi: f64| {
            (
                lo + band * (hi - lo),
                lo + (band + 0.5) * (hi - lo),
            )
        };
        let mut rng = SplitMix64Rng::new(root.raw());
        let m = |salt: u64, lo: f64, hi: f64, rng: &mut SplitMix64Rng| {
            let u = (skill * 0.6 + n(salt) * 0.4).clamp(0.0, 1.0);
            sample_band(half(lo, hi), u, rng)
        };
        Self {
            tier,
            skill,
            fitts_a_ms: m(seeds::SALT_FITTS_A, 140.0, 300.0, &mut rng),
            fitts_b_ms: m(seeds::SALT_FITTS_B, 90.0, 220.0, &mut rng),
            gravity: m(seeds::SALT_GRAVITY, 6.5, 12.0, &mut rng),
            wind: m(seeds::SALT_WIND, 2.0, 5.5, &mut rng),
            max_step: m(seeds::SALT_MAX_STEP, 10.0, 22.0, &mut rng),
            wind_jitter_ln: m(seeds::SALT_WIND_JITTER, 0.12, 0.28, &mut rng),
            tremor_amp_px: m(seeds::SALT_TREMOR_AMP, 0.25, 1.15, &mut rng),
            tremor_hz: m(seeds::SALT_TREMOR_HZ, 7.5, 12.0, &mut rng),
            overshoot_p: m(seeds::SALT_OVERSHOOT_P, 0.10, 0.40, &mut rng),
            corrections_max: 1 + (m(seeds::SALT_CORRECTIONS, 1.0, 3.0, &mut rng) as u8),
            dwell_median_ms: m(seeds::SALT_DWELL_MEDIAN, 78.0, 112.0, &mut rng),
            dwell_sigma_ln: m(seeds::SALT_DWELL_SIGMA, 0.22, 0.32, &mut rng),
            wpm: m(seeds::SALT_WPM, 28.0, 90.0, &mut rng),
            typing_sigma_ln: m(seeds::SALT_TYPING_SIGMA, 0.28, 0.42, &mut rng),
            burst_len: 3 + (m(seeds::SALT_BURST_LEN, 0.0, 4.0, &mut rng) as u32),
            burst_pause_median_ms: m(seeds::SALT_BURST_PAUSE, 180.0, 420.0, &mut rng),
            backspace_p: m(seeds::SALT_BACKSPACE_P, 0.02, 0.08, &mut rng),
            caps_error_p: m(seeds::SALT_CAPS_ERROR_P, 0.01, 0.06, &mut rng),
            notch_px: m(seeds::SALT_NOTCH_PX, 90.0, 145.0, &mut rng),
            notch_friction: m(seeds::SALT_NOTCH_FRICTION, 0.82, 0.93, &mut rng),
            notch_gap_median_ms: m(seeds::SALT_NOTCH_GAP, 70.0, 160.0, &mut rng),
            overscroll_p: m(seeds::SALT_OVERSCROLL_P, 0.05, 0.17, &mut rng),
            reading_pause_p: m(seeds::SALT_READING_PAUSE_P, 0.08, 0.20, &mut rng),
            double_click_p: m(seeds::SALT_DOUBLE_CLICK_P, 0.03, 0.10, &mut rng),
            right_click_p: m(seeds::SALT_RIGHT_CLICK_P, 0.02, 0.06, &mut rng),
            hover_p: m(seeds::SALT_HOVER_P, 0.70, 0.98, &mut rng),
            idle_theta: m(seeds::SALT_IDLE_THETA, 1.2, 2.0, &mut rng),
            idle_noise_px_s: m(seeds::SALT_IDLE_NOISE, 8.25, 13.75, &mut rng),
            idle_hop_w_px: m(seeds::SALT_IDLE_HOP_W, 18.2, 33.8, &mut rng),
            idle_scroll_p: m(seeds::SALT_IDLE_SCROLL_P, 0.03, 0.102, &mut rng),
            idle_anchor_pull: m(seeds::SALT_IDLE_ANCHOR_PULL, 0.64, 0.96, &mut rng),
            idle_anchor_wander: m(seeds::SALT_IDLE_ANCHOR_WANDER, 0.48, 0.72, &mut rng),
            hop_interval_median_ms: m(seeds::SALT_HOP_INTERVAL, 4800.0, 8000.0, &mut rng),
            reading_scroll_px: m(seeds::SALT_READING_SCROLL, 240.0, 560.0, &mut rng),
            reaction_sigma_ln: m(seeds::SALT_REACTION_SIGMA, 0.16, 0.28, &mut rng),
        }
    }

    #[inline]
    pub fn movement_time_ms(
        &self,
        distance_px: f64,
        target_w_px: f64,
        rng: &mut SplitMix64Rng,
    ) -> u32 {
        let w = target_w_px.max(4.0);
        let id = (distance_px / w).sqrt() + 0.5 * (distance_px / w + 1.0).log2();
        let base = self.fitts_a_ms + self.fitts_b_ms * id;
        rng.lognormal_ms(base, 0.18)
    }

    #[inline]
    pub fn throttle(&self, trust: i32) -> ThrottledPersona {
        let cool = trust.min(0).unsigned_abs() as f64 * 0.25;
        ThrottledPersona {
            cool,
            overshoot_p: (self.overshoot_p * (1.0 + cool)).min(0.9),
            corrections_max: (self.corrections_max + cool as u8).min(4),
            dwell_sigma_ln: (self.dwell_sigma_ln * (1.0 + cool * 0.5)).min(0.6),
            burst_len: (self.burst_len.saturating_sub(cool as u32)).max(2),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThrottledPersona {
    pub cool: f64,
    pub overshoot_p: f64,
    pub corrections_max: u8,
    pub dwell_sigma_ln: f64,
    pub burst_len: u32,
}
