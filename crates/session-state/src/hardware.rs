use core_utils::rng::{SplitMix64Rng, mix_ctx, seeds};

use crate::persona::{Persona, Tier};
use crate::profile::Profile;
use crate::{POINTER_MOUSE, POINTER_TOUCH, POINTER_TOUCHPAD};

const ACCEL_TABLE: [f64; 31] = [
    0.0315, 0.0625, 0.0975, 0.1325, 0.1750, 0.2225, 0.2775, 0.3400, 0.4100, 0.4875, 0.5750, 0.6725,
    0.7800, 0.8975, 1.0200, 1.1425, 1.2650, 1.3900, 1.5225, 1.6600, 1.8000, 1.9475, 2.0975, 2.2525,
    2.4125, 2.5750, 2.7425, 2.9150, 3.0900, 3.2700, 3.4550,
];

#[derive(Debug, Clone, Copy)]
pub struct MouseHardware {
    pub poll_hz: u32,
    pub dpi: u32,
    pub pointer_speed: u8,
    pub accel: bool,
    pub battery_saver: bool,
    pub kind: u8,
}

impl MouseHardware {
    #[inline]
    pub fn gain(&self, speed_px_s: f64) -> f64 {
        if !self.accel {
            return 1.0;
        }
        let steps = speed_px_s / 6.4;
        let idx = (steps.floor() as i64).clamp(0, 29) as usize;
        ACCEL_TABLE[idx] + (ACCEL_TABLE[idx + 1] - ACCEL_TABLE[idx]) * steps.fract().clamp(0.0, 1.0)
    }

    #[inline]
    pub fn counts_per_px(&self) -> f64 {
        let dpi = self.dpi.max(200) as f64;
        dpi / 96.0
    }

    #[inline]
    pub fn poll_quantum_us(&self) -> u64 {
        1_000_000 / self.poll_hz.clamp(60, 1000) as u64
    }

    #[inline]
    pub fn throttle_factor(&self) -> f64 {
        if self.battery_saver { 1.22 } else { 1.0 }
    }
}

pub fn hardware_of(profile: &Profile, persona: Persona) -> MouseHardware {
    let kind = if profile.platform.is_mobile() {
        POINTER_TOUCH
    } else {
        profile.pointer_kind
    };
    let base = MouseHardware {
        poll_hz: 125,
        dpi: 96,
        pointer_speed: 10,
        accel: true,
        battery_saver: profile.battery_saver,
        kind,
    };
    match kind {
        POINTER_TOUCHPAD => base,
        POINTER_TOUCH => MouseHardware {
            poll_hz: 120,
            dpi: 0,
            accel: false,
            ..base
        },
        POINTER_MOUSE => {
            let mut rng = SplitMix64Rng::new(mix_ctx(profile.canvas_seed, seeds::SALT_HOP));
            let s = persona.skill;
            let ceiling = profile.preset.max_mouse_hz();
            if persona.tier == Tier::Gamer {
                let want = if s < 0.34 { 500 } else { 1000 };
                MouseHardware {
                    poll_hz: want.min(ceiling),
                    dpi: 1600 + rng.next_below(5) as u32 * 200,
                    pointer_speed: profile.pointer_speed.clamp(1, 6),
                    accel: false,
                    ..base
                }
            } else {
                let want = if s < 0.60 { 125 } else { 250 };
                MouseHardware {
                    poll_hz: want.min(ceiling),
                    dpi: 1000 + rng.next_below(3) as u32 * 200,
                    pointer_speed: profile.pointer_speed.clamp(4, 20),
                    ..base
                }
            }
        }
        _ => base,
    }
}

pub fn display_hz_for(profile: &Profile) -> u16 {
    let ceiling = profile.preset.max_display_hz();
    let want = match profile.persona.tier {
        Tier::Gamer => {
            if profile.persona.skill < 0.34 {
                120
            } else {
                144
            }
        }
        Tier::Office => {
            if profile.persona.skill < 0.60 {
                60
            } else {
                75
            }
        }
    };
    want.min(ceiling)
}
