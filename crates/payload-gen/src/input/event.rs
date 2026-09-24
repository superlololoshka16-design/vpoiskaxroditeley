pub mod kind {
    pub const MOVE: u8 = 0;
    pub const PRESS: u8 = 1;
    pub const RELEASE: u8 = 2;
    pub const WHEEL: u8 = 3;
    pub const KEY_DOWN: u8 = 4;
    pub const KEY_UP: u8 = 5;
    pub const FOCUS: u8 = 6;
    pub const BLUR: u8 = 7;
    pub const VISIBILITY: u8 = 8;
}

pub mod focus_arg {
    pub const WINDOW: u8 = 0;
    pub const ELEMENT: u8 = 1;
}

pub mod button {
    pub const LEFT: u8 = 0;
    pub const RIGHT: u8 = 2;
}

#[inline]
pub fn dt_ms_u16(now_us: u64, last_us: u64) -> u16 {
    delta_ms_u16(now_us.saturating_sub(last_us))
}

#[inline]
pub fn delta_ms_u16(delta_us: u64) -> u16 {
    (delta_us / 1000).min(u16::MAX as u64) as u16
}

#[inline]
pub fn coord_u16(v: f64) -> u16 {
    v.clamp(0.0, u16::MAX as f64) as u16
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RawEvent {
    pub x: u16,
    pub y: u16,
    pub dt_ms: u16,
    pub kind: u8,
    pub arg: u8,
}

impl RawEvent {
    #[inline]
    pub const fn new(x: u16, y: u16, dt_ms: u16, kind: u8, arg: u8) -> Self {
        Self {
            x,
            y,
            dt_ms,
            kind,
            arg,
        }
    }
}

pub const RAW_EVENT_LEN: usize = core::mem::size_of::<RawEvent>();

#[inline]
pub fn events_bytes(events: &[RawEvent]) -> &[u8] {
    bytemuck::cast_slice(events)
}

use session_state::Persona;
use core_utils::SplitMix64Rng;
use core_utils::rng::seeds;

#[derive(Debug, Clone, Copy)]
pub struct ClickPlan {
    pub button: u8,
    pub is_double: bool,
    pub is_right: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Press,
    Hold,
    Release,
    Done,
}

pub struct ClickCursor {
    persona: Persona,
    x: u16,
    y: u16,
    stage: Stage,
    next_due_us: u64,
    plan: ClickPlan,
    second_pending: bool,
    rng: SplitMix64Rng,
}

impl ClickCursor {
    fn plan(persona: &Persona, rng: &mut SplitMix64Rng) -> ClickPlan {
        let is_right = rng.chance(persona.right_click_p);
        let is_double = !is_right && rng.chance(persona.double_click_p);
        ClickPlan {
            button: if is_right { button::RIGHT } else { button::LEFT },
            is_double,
            is_right,
        }
    }

    pub fn new(persona: Persona, x: u16, y: u16, now_us: u64, seed: u64) -> Self {
        let mut rng = SplitMix64Rng::new(seed ^ seeds::SALT_CLICK_PLAN);
        let plan = Self::plan(&persona, &mut rng);
        let pre_press = rng.lognormal_us(40.0, 0.3);
        Self {
            persona,
            x,
            y,
            stage: Stage::Press,
            next_due_us: now_us + pre_press,
            plan,
            second_pending: false,
            rng,
        }
    }

    #[inline]
    pub fn done(&self) -> bool {
        self.stage == Stage::Done
    }

    #[inline]
    pub fn next_due_us(&self) -> u64 {
        self.next_due_us
    }


    fn nudge(&mut self) {
        let dx = self.rng.next_range(0, 3) as i32 - 1;
        let dy = self.rng.next_range(0, 3) as i32 - 1;
        self.x = (self.x as i32 + dx).clamp(0, u16::MAX as i32) as u16;
        self.y = (self.y as i32 + dy).clamp(0, u16::MAX as i32) as u16;
    }

    pub fn step(&mut self, now_us: u64) -> Option<RawEvent> {
        if self.stage == Stage::Done || now_us < self.next_due_us {
            return None;
        }
        match self.stage {
            Stage::Press => {
                self.stage = Stage::Hold;
                let hold = self
                    .rng
                    .lognormal_us(self.persona.dwell_median_ms, self.persona.dwell_sigma_ln);
                self.next_due_us = now_us + hold;
                Some(RawEvent::new(
                    self.x,
                    self.y,
                    0,
                    kind::PRESS,
                    self.plan.button,
                ))
            }
            Stage::Hold => {
                self.stage = Stage::Release;
                let gap = self.rng.lognormal_us(70.0, 0.35);
                self.next_due_us = now_us + gap;
                self.nudge();
                Some(RawEvent::new(
                    self.x,
                    self.y,
                    0,
                    kind::RELEASE,
                    self.plan.button,
                ))
            }
            Stage::Release => {
                if self.plan.is_double && !self.second_pending {
                    self.second_pending = true;
                    self.stage = Stage::Press;
                    let dbl_gap = self.rng.lognormal_us(110.0, 0.25);
                    self.next_due_us = now_us + dbl_gap;
                    return None;
                }
                self.stage = Stage::Done;
                None
            }
            Stage::Done => None,
        }
    }
}

crate::input_cursor!(ClickCursor);

const FRAME_US: u64 = 16_667;

pub struct ScrollCursor {
    persona: Persona,
    velocity: f64,
    remaining_px: f64,
    frame_due_us: u64,
    notch_due_us: u64,
    finished: bool,
    rng: SplitMix64Rng,
    last_emit_y: i32,
    last_emit_us: u64,
    y: f64,
}

impl ScrollCursor {
    pub fn new(persona: Persona, total_px: f64, now_us: u64, seed: u64) -> Self {
        let mut rng = SplitMix64Rng::new(seed ^ seeds::SALT_SCROLL_CURSOR);
        let overscroll = if rng.chance(persona.overscroll_p) {
            total_px.signum() * rng.next_f64() * persona.notch_px * 1.5
        } else {
            0.0
        };
        Self {
            persona,
            velocity: 0.0,
            remaining_px: total_px + overscroll,
            frame_due_us: now_us,
            notch_due_us: now_us,
            finished: false,
            rng,
            last_emit_y: 0,
            last_emit_us: now_us,
            y: 0.0,
        }
    }

    #[inline]
    pub fn done(&self) -> bool {
        self.finished
    }

    #[inline]
    pub fn next_due_us(&self) -> u64 {
        self.frame_due_us
    }

    pub fn step(&mut self, now_us: u64) -> Option<RawEvent> {
        if self.finished || now_us < self.frame_due_us {
            return None;
        }
        self.frame_due_us = now_us + FRAME_US;
        if self.remaining_px != 0.0 {
            if now_us >= self.notch_due_us {
                let notches = 1 + self.rng.next_below(3);
                let impulse = self.persona.notch_px * notches as f64 * self.remaining_px.signum();
                self.velocity += impulse;
                let gap = self
                    .rng
                    .lognormal_us(self.persona.notch_gap_median_ms, 0.30);
                self.notch_due_us = now_us + gap;
            }
            self.velocity *= self.persona.notch_friction;
            let step = self.velocity;
            if self.remaining_px.abs() <= step.abs().max(1.0) {
                self.y += self.remaining_px;
                self.remaining_px = 0.0;
                self.velocity = 0.0;
                self.finished = true;
            } else {
                self.y += step;
                self.remaining_px -= step;
            }
        }
        let ey = self.y.round() as i32;
        if ey == self.last_emit_y {
            return None;
        }
        let dt = delta_ms_u16(now_us.saturating_sub(self.last_emit_us)).max(1);
        self.last_emit_y = ey;
        self.last_emit_us = now_us;
        Some(RawEvent::new(
            0,
            ey.unsigned_abs() as u16,
            dt,
            kind::WHEEL,
            0,
        ))
    }
}

crate::input_cursor!(ScrollCursor);

const VISIBILITY_FLIP_INTERVAL_MS: u64 = 15 * 60 * 1_000;
const VISIBILITY_FLIP_JITTER_MS: u64 = 60_000;
pub(crate) const VS_VISIBLE: u8 = 0;
pub(crate) const VS_HIDDEN: u8 = 1;

const CLICK_OFFSET_FRACTION: f64 = 0.30;

pub fn click_offset(target_w: f64, target_h: f64, seed: u64) -> (f64, f64) {
    let mut rng = SplitMix64Rng::new(seed ^ seeds::SALT_CLICK_OFFSET_BASE);
    let max_dx = (target_w * CLICK_OFFSET_FRACTION).max(1.0);
    let max_dy = (target_h * CLICK_OFFSET_FRACTION).max(1.0);
    let dx = (rng.next_f64() * 2.0 - 1.0) * max_dx;
    let dy = (rng.next_f64() * 2.0 - 1.0) * max_dy;
    (dx, dy)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusStage {
    FocusIn,
    Dwell,
    FocusOut,
    Done,
}

pub struct FocusCursor {
    x: u16,
    y: u16,
    stage: FocusStage,
    next_due_us: u64,
    arg: u8,
    rng: SplitMix64Rng,
}

impl FocusCursor {
    fn at(x: u16, y: u16, arg: u8, now_us: u64, seed: u64, med_ms: f64, sigma: f64) -> Self {
        let mut rng = SplitMix64Rng::new(seed);
        let pre = rng.lognormal_us(med_ms, sigma);
        Self {
            x,
            y,
            stage: FocusStage::FocusIn,
            next_due_us: now_us + pre,
            arg,
            rng,
        }
    }

    pub fn new(x: u16, y: u16, now_us: u64, seed: u64) -> Self {
        Self::at(
            x,
            y,
            focus_arg::ELEMENT,
            now_us,
            seed ^ seeds::SALT_FOCUS_CURSOR,
            62.0,
            0.3,
        )
    }

    pub fn window(now_us: u64, seed: u64) -> Self {
        Self::at(0, 0, focus_arg::WINDOW, now_us, seed ^ seeds::SALT_FOCUS_WINDOW, 90.0, 0.35)
    }

    #[inline]
    pub fn done(&self) -> bool {
        self.stage == FocusStage::Done
    }

    #[inline]
    pub fn next_due_us(&self) -> u64 {
        self.next_due_us
    }

    pub fn step(&mut self, now_us: u64) -> Option<RawEvent> {
        if self.stage == FocusStage::Done || now_us < self.next_due_us {
            return None;
        }
        match self.stage {
            FocusStage::FocusIn => {
                Some(self.advance(now_us, FocusStage::Dwell, 320.0, 0.4, kind::FOCUS))
            }
            FocusStage::Dwell => {
                Some(self.advance(now_us, FocusStage::FocusOut, 48.0, 0.28, kind::BLUR))
            }
            FocusStage::FocusOut => {
                self.stage = FocusStage::Done;
                None
            }
            FocusStage::Done => None,
        }
    }

    fn advance(
        &mut self,
        now_us: u64,
        next: FocusStage,
        median_ms: f64,
        sigma_ln: f64,
        ev_kind: u8,
    ) -> RawEvent {
        self.stage = next;
        let dwell = self.rng.lognormal_us(median_ms, sigma_ln);
        self.next_due_us = now_us + dwell;
        RawEvent::new(self.x, self.y, 0, ev_kind, self.arg)
    }
}

crate::input_cursor!(FocusCursor);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibilityFlip {
    pub new_state: u8,
    pub cycle_idx: u64,
}

pub fn visibility_flip(farble_seed: u64, ctx_id: u64, t_ms: u64) -> Option<VisibilityFlip> {
    let cycle_len = VISIBILITY_FLIP_INTERVAL_MS;
    let cycle_idx = t_ms / cycle_len;
    let mut lcg = SplitMix64Rng::new(farble_seed ^ ctx_id).stepped(cycle_idx);
    let jitter = lcg.next_below(VISIBILITY_FLIP_JITTER_MS as u64).max(1);
    let this_cycle_start = cycle_idx * cycle_len;
    let flip_at = this_cycle_start.saturating_add(jitter);
    if t_ms < flip_at || t_ms >= this_cycle_start.saturating_add(cycle_len) {
        return None;
    }
    let base_parity = SplitMix64Rng::new(farble_seed ^ ctx_id).next_u64() & 1;
    let new_state = if (base_parity ^ (cycle_idx & 1)) == 0 {
        VS_HIDDEN
    } else {
        VS_VISIBLE
    };
    Some(VisibilityFlip {
        new_state,
        cycle_idx,
    })
}
