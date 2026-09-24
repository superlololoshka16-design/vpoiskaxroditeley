use crate::input::event::{RawEvent, button, coord_u16, delta_ms_u16, kind as input};
use session_state::MouseHardware;
use session_state::Persona;
use core_utils::math::{Perlin2D, ou_step};
use core_utils::SplitMix64Rng;
use core_utils::rng::seeds;

const ARRIVE_EPS_PX: f64 = 0.5;
const SETTLE_SPEED_PX_S: f64 = 80.0;
const FINAL_APPROACH_US: u64 = 160_000;
const OU_THETA: f64 = 8.0;
const CURVE_THETA: f64 = 0.35;
const CURVE_SIGMA: f64 = 3.0;
const DT_MIN_S: f64 = 0.001;
const DT_MAX_S: f64 = 0.05;
const EMIT_JITTER_US: u64 = 600;
const SUB_MAX: u8 = 4;
const MISS_POLL_P: f64 = 0.03;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Reacting,
    Moving,
    Dwelling,
    Done,
}

pub struct MotionCursor {
    persona: Persona,
    hw: MouseHardware,
    x: f64,
    y: f64,
    sub_x: f64,
    sub_y: f64,
    start_x: f64,
    start_y: f64,
    sub_t: f64,
    sub_dur: f64,
    vx: f64,
    vy: f64,
    nx: f64,
    ny: f64,
    curve: f64,
    k: f64,
    zeta: f64,
    vmax: f64,
    noise_sigma: f64,
    tremor_amp: f64,
    settle_r: f64,
    aim_x: f64,
    aim_y: f64,
    target_x: f64,
    target_y: f64,
    target_w: f64,
    corrections_left: u8,
    sub_index: u8,
    deadline_us: u64,
    next_due_us: u64,
    quantum_us: u64,
    emit_period_us: u64,
    next_emit_us: u64,
    state: State,
    dwell_until_us: u64,
    rng: SplitMix64Rng,
    tremor: Perlin2D,
    last_emit_x: i32,
    last_emit_y: i32,
    last_emit_us: u64,
    last_int_us: u64,
    emitted: bool,
}

pub struct MotionStart {
    pub persona: Persona,
    pub from_x: f64,
    pub from_y: f64,
    pub to_x: f64,
    pub to_y: f64,
    pub target_w: f64,
    pub now_us: u64,
    pub seed: u64,
    pub trust: i32,
    pub display_hz: u32,
    pub hw: MouseHardware,
}

impl MotionStart {
    pub fn plain(
        persona: Persona,
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        target_w: f64,
        now_us: u64,
        seed: u64,
        trust: i32,
        display_hz: u32,
    ) -> Self {
        let hw = MouseHardware {
            poll_hz: 125,
            dpi: 1000,
            pointer_speed: 10,
            accel: true,
            battery_saver: false,
            kind: session_state::POINTER_MOUSE,
        };
        Self {
            persona,
            from_x,
            from_y,
            to_x,
            to_y,
            target_w,
            now_us,
            seed,
            trust,
            display_hz,
            hw,
        }
    }
}

impl MotionCursor {
    pub fn new(start: MotionStart) -> Self {
        let MotionStart {
            persona,
            from_x,
            from_y,
            to_x,
            to_y,
            target_w,
            now_us,
            seed,
            trust,
            display_hz,
            hw,
        } = start;
        let mut rng = SplitMix64Rng::new(seed ^ seeds::SALT_MOTION_RNG);
        let dist = ((to_x - from_x).powi(2) + (to_y - from_y).powi(2)).sqrt();
        let budget_ms = persona.movement_time_ms(dist, target_w, &mut rng) as u64;
        let throttle = persona.throttle(trust);
        let cool = throttle.cool;
        let quantum_us = hw.poll_quantum_us();

        let emit_hz = if display_hz == 0 { 60 } else { display_hz };
        let k = persona.gravity * 14.0 * (0.85 + rng.next_f64() * 0.30);
        let zeta_lo = 0.45 - cool * 0.12;
        let zeta_hi = 0.90 - cool * 0.25;
        let zeta = zeta_lo + rng.next_f64() * (zeta_hi - zeta_lo).max(0.05);
        let vmax_hw = persona.max_step * 125.0 * (0.85 + rng.next_f64() * 0.30);
        let budget_s = (budget_ms.max(80) as f64 / 1000.0) * hw.throttle_factor();
        let vmax_fitts = dist / (budget_s * 0.52);
        let vmax = if dist > 1.0 {
            vmax_hw.min(vmax_fitts.max(120.0))
        } else {
            30.0
        };
        let noise_sigma = persona.wind * 32.0 * (1.0 + cool * 0.3);
        let tremor_amp = persona.tremor_amp_px * 1.6;
        let settle_r = (target_w.max(4.0) * 0.25).max(ARRIVE_EPS_PX);
        let reaction_us =
            rng.lognormal_us(persona.fitts_a_ms, persona.reaction_sigma_ln);
        let mut c = Self {
            persona,
            hw,
            x: from_x,
            y: from_y,
            sub_x: from_x,
            sub_y: from_y,
            start_x: from_x,
            start_y: from_y,
            sub_t: 0.0,
            sub_dur: 0.0,
            vx: 0.0,
            vy: 0.0,
            nx: 0.0,
            ny: 0.0,
            curve: 0.0,
            k,
            zeta,
            vmax,
            noise_sigma,
            tremor_amp,
            settle_r,
            aim_x: to_x,
            aim_y: to_y,
            target_x: to_x,
            target_y: to_y,
            target_w: target_w.max(4.0),
            corrections_left: throttle.corrections_max,
            sub_index: 0,
            deadline_us: now_us + budget_ms * 1000 + reaction_us,
            next_due_us: now_us + reaction_us,
            quantum_us,
            emit_period_us: 1_000_000 / emit_hz as u64,
            next_emit_us: now_us + reaction_us,
            state: State::Reacting,
            dwell_until_us: 0,
            rng,
            tremor: Perlin2D::from_seed(seed ^ seeds::SALT_TREMOR_NOISE),
            last_emit_x: from_x.round() as i32,
            last_emit_y: from_y.round() as i32,
            last_emit_us: now_us,
            last_int_us: now_us,
            emitted: false,
        };
        c.rng.next_u64();
        c.begin_submovement(dist);
        c
    }

    fn begin_submovement(&mut self, dist: f64) {
        let overshoot = if self.sub_index == 0 {
            (self.persona.overshoot_p * dist / 4.0).max(0.0)
        } else {
            (self.persona.overshoot_p * dist / 16.0).max(0.0)
        };
        let dir_x = self.target_x - self.x;
        let dir_y = self.target_y - self.y;
        let len = (dir_x * dir_x + dir_y * dir_y).sqrt().max(1.0);
        let over_x = dir_x + (dir_x / len) * overshoot * (1.0 + self.rng.next_f64() * 0.2);
        let over_y = dir_y + (dir_y / len) * overshoot * (1.0 + self.rng.next_f64() * 0.2);
        self.start_x = self.x;
        self.start_y = self.y;
        self.sub_x = self.x + over_x;
        self.sub_y = self.y + over_y;
        let sub_dist = (over_x * over_x + over_y * over_y).sqrt().max(1.0);
        let peak = (self.vmax * 0.62).max(60.0);
        self.sub_dur = (1.875 * sub_dist / peak).clamp(0.045, 0.5);
        self.sub_t = 0.0;
    }

    #[inline]
    pub fn done(&self) -> bool {
        self.state == State::Done
    }

    #[inline]
    pub fn pos(&self) -> (i32, i32) {
        (self.x.round() as i32, self.y.round() as i32)
    }

    #[inline]
    pub fn next_due_us(&self) -> u64 {
        self.next_due_us
    }

    #[inline]
    pub fn on_target(&self) -> bool {
        let dx = self.x - self.target_x;
        let dy = self.y - self.target_y;
        (dx * dx + dy * dy).sqrt() <= self.target_w * 0.5
    }

    pub fn step(&mut self, now_us: u64) -> Option<RawEvent> {
        if self.state == State::Done || now_us < self.next_due_us {
            return None;
        }
        self.next_due_us = now_us + self.quantum_us;
        match self.state {
            State::Reacting => {
                self.state = State::Moving;
                self.step_moving(now_us)
            }
            State::Moving => self.step_moving(now_us),
            State::Dwelling => self.step_dwelling(now_us),
            State::Done => None,
        }
    }

    fn step_moving(&mut self, now_us: u64) -> Option<RawEvent> {
        if self.arrived(now_us) {
            return None;
        }
        let dt_us = now_us.saturating_sub(self.last_int_us).max(1);
        self.last_int_us = now_us;
        let dt = (dt_us as f64 / 1_000_000.0).clamp(DT_MIN_S, DT_MAX_S);
        self.integrate(dt);
        if now_us < self.next_emit_us {
            return None;
        }
        if self.rng.chance(MISS_POLL_P) {
            self.next_emit_us += self.quantum_us;
            return None;
        }
        let due_us = self.book_emit();
        let spd = (self.vx * self.vx + self.vy * self.vy).sqrt();
        let amp = self.tremor_amp * (0.35 + 2.2 / (1.0 + spd * 0.008));
        self.tremor_emit(now_us, due_us, 1.0, 0.0, amp, true)
    }

    fn step_dwelling(&mut self, now_us: u64) -> Option<RawEvent> {
        if now_us >= self.dwell_until_us {
            self.state = State::Done;
            return None;
        }
        if now_us < self.next_emit_us {
            return None;
        }
        let due_us = self.book_emit();
        self.tremor_emit(now_us, due_us, 1.7, 50.0, self.tremor_amp * 0.9, false)
    }

    #[inline]
    fn book_emit(&mut self) -> u64 {
        let due_us = self.next_emit_us;
        self.next_emit_us = due_us + self.emit_period_us + self.emit_jitter_us();
        due_us
    }

    fn tremor_emit(
        &mut self,
        now_us: u64,
        due_us: u64,
        hz_mul: f64,
        phase: f64,
        amp: f64,
        gate_on_emitted: bool,
    ) -> Option<RawEvent> {
        let t_s = now_us as f64 / 1e6;
        let hz = self.persona.tremor_hz * hz_mul;
        let tx = self.tremor.noise(t_s * hz, phase) * amp;
        let ty = self.tremor.noise(phase, t_s * hz) * amp;
        let ex = (self.x + tx).round() as i32;
        let ey = (self.y + ty).round() as i32;
        if ex == self.last_emit_x && ey == self.last_emit_y && (!gate_on_emitted || self.emitted) {
            return None;
        }
        self.emitted = true;
        Some(self.emit(ex, ey, due_us))
    }

    #[inline]
    fn emit(&mut self, ex: i32, ey: i32, at_us: u64) -> RawEvent {
        let raw_dt = at_us.saturating_sub(self.last_emit_us);
        let quant = self.quantum_us.max(1);
        let snapped = (raw_dt + quant / 2) / quant * quant;
        let dt = delta_ms_u16(snapped).max(1);
        self.last_emit_x = ex;
        self.last_emit_y = ey;
        self.last_emit_us = at_us;
        RawEvent::new(
            coord_u16(ex as f64),
            coord_u16(ey as f64),
            dt,
            input::MOVE,
            button::LEFT,
        )
    }

    #[inline]
    fn emit_jitter_us(&mut self) -> u64 {
        let g = self.rng.gauss();
        let j = (g * EMIT_JITTER_US as f64) as i64;
        let mut out = j.unsigned_abs().min(2_000) + 300;
        if self.rng.chance(0.05) {
            out += 3_000 + self.rng.next_below(7_000);
        }
        out
    }

    fn integrate(&mut self, dt: f64) {
        self.sub_t += dt;
        let tau = (self.sub_t / self.sub_dur).clamp(0.0, 1.0);
        let shape = 10.0 * tau.powi(3) - 15.0 * tau.powi(4) + 6.0 * tau.powi(5);
        let guide_x = self.start_x + (self.sub_x - self.start_x) * shape;
        let guide_y = self.start_y + (self.sub_y - self.start_y) * shape;
        let dxr = self.target_x - self.x;
        let dyr = self.target_y - self.y;
        let dl = (dxr * dxr + dyr * dyr).sqrt().max(1.0);
        let ux = dxr / dl;
        let uy = dyr / dl;
        self.curve = ou_step(
            self.curve,
            CURVE_THETA * dt,
            CURVE_SIGMA * dt.sqrt() * self.rng.gauss(),
        );
        let bend =
            self.curve * (dl / 8.0).min(25.0) * (dl / 240.0).min(1.0) * self.persona.wind / 4.0;
        self.aim_x = guide_x - uy * bend;
        self.aim_y = guide_y + ux * bend;
        let dx = self.aim_x - self.x;
        let dy = self.aim_y - self.y;
        let theta_dt = OU_THETA * dt;
        let sqrt_dt = dt.sqrt();
        let dz = self.rng.gauss();
        let dzx = self.rng.gauss();
        let dzy = self.rng.gauss();
        let sx = self.noise_sigma * sqrt_dt * (dz * 0.62 + dzx * 0.79);
        let sy = self.noise_sigma * sqrt_dt * (dz * 0.62 + dzy * 0.79);
        self.nx = ou_step(self.nx, theta_dt, sx);
        self.ny = ou_step(self.ny, theta_dt, sy);
        let spd = (self.vx * self.vx + self.vy * self.vy).sqrt();
        let far = (dl / 150.0).min(1.0);
        let gain = 1.0 + ((spd / 800.0).min(0.85)) * far;
        let c = 2.0 * self.zeta * self.k.sqrt();
        let fx = self.k * gain * dx + self.nx - c * self.vx;
        let fy = self.k * gain * dy + self.ny - c * self.vy;
        self.vx += fx * dt;
        self.vy += fy * dt;
        let spd = (self.vx * self.vx + self.vy * self.vy).sqrt();
        if spd > self.vmax {
            let soft = self.vmax * (1.0 + 0.06 * self.rng.next_f64()) / spd;
            self.vx *= soft;
            self.vy *= soft;
        }
        let hand_vx = self.vx * self.hw.counts_per_px();
        let hand_vy = self.vy * self.hw.counts_per_px();
        let hand_spd = (hand_vx * hand_vx + hand_vy * hand_vy).sqrt();
        let g = self.hw.gain(hand_spd);
        self.x += self.vx * g * dt;
        self.y += self.vy * g * dt;
        self.x = self.x.clamp(0.0, u16::MAX as f64);
        self.y = self.y.clamp(0.0, u16::MAX as f64);
        if tau >= 1.0 && self.sub_index < SUB_MAX {
            let d = ((self.target_x - self.x).powi(2) + (self.target_y - self.y).powi(2)).sqrt();
            if d > self.settle_r {
                self.sub_index += 1;
                self.vmax *= 0.55;
                self.begin_submovement(d);
            }
        }
    }

    fn arrived(&mut self, now_us: u64) -> bool {
        let dx = self.target_x - self.x;
        let dy = self.target_y - self.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let spd = (self.vx * self.vx + self.vy * self.vy).sqrt();
        if dist <= self.settle_r && spd < SETTLE_SPEED_PX_S {
            self.enter_dwell(now_us);
            return true;
        }
        if now_us >= self.deadline_us {
            if self.corrections_left > 0 {
                self.corrections_left -= 1;
                self.k *= 2.1;
                self.zeta = 0.90 + self.rng.next_f64() * 0.08;
                self.vmax *= 0.6;
            } else {
                self.k = 900.0;
                self.zeta = 1.0;
                self.vmax = (self.vmax * 0.5).max(40.0);
            }
            self.deadline_us = now_us + FINAL_APPROACH_US;
        }
        false
    }

    fn enter_dwell(&mut self, now_us: u64) {
        let dwell = self
            .rng
            .lognormal_us(self.persona.dwell_median_ms, self.persona.dwell_sigma_ln);
        self.dwell_until_us = now_us + dwell;
        self.state = State::Dwelling;
        self.next_due_us = now_us + self.quantum_us;
    }
}

crate::input_cursor!(MotionCursor);
