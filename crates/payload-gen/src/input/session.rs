use crate::input::event::{
    ClickCursor, FocusCursor, RawEvent, ScrollCursor, VS_HIDDEN, button, click_offset, coord_u16,
    dt_ms_u16, kind as input, visibility_flip,
};
use session_state::MouseHardware;
use crate::input::motion::{MotionCursor, MotionStart};
use session_state::Persona;
use crate::input::typing::TypingCursor;
use crate::input::{InputCursor, pump};
use core_utils::rng::{SplitMix64Rng, mix_ctx, seeds};
use smallvec::SmallVec;

const IDLE_TICK_MIN_US: u64 = 180_000;
const IDLE_TICK_SPAN_US: u64 = 620_000;
const IDLE_EMIT_GAP_US: u64 = 160_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TabPhase {
    Approach,
    Click,
    Typing,
    Reading,
    Idle,
}

pub struct TabTick {
    pub events: SmallVec<[RawEvent; 32]>,
    pub next_due_us: u64,
    pub phase: TabPhase,
}

enum Active {
    Move(MotionCursor),
    Focus(FocusCursor),
    Click(ClickCursor),
    Type(TypingCursor),
    Scroll(ScrollCursor),
    None,
}

impl Active {
    fn slot(&mut self) -> Option<(&mut dyn InputCursor, FinishedKind)> {
        match self {
            Active::Move(c) => Some((c, FinishedKind::Move)),
            Active::Focus(c) => Some((c, FinishedKind::Focus)),
            Active::Click(c) => Some((c, FinishedKind::Interact)),
            Active::Type(c) => Some((c, FinishedKind::Interact)),
            Active::Scroll(c) => Some((c, FinishedKind::Scroll)),
            Active::None => None,
        }
    }
}

#[derive(Clone, Copy)]
enum FinishedKind {
    Move,
    Focus,
    Interact,
    Scroll,
}

#[derive(Clone, Copy)]
struct IdleEnv {
    persona: Persona,
    seed: u64,
    ctx_id: u64,
    display_hz: u32,
    hw: MouseHardware,
}

struct IdleMotor {
    anchor_x: f64,
    anchor_y: f64,
    ax: f64,
    ay: f64,
    x: f64,
    y: f64,
    last_emit_us: u64,
    last_tick_us: u64,
    next_bump_us: u64,
    hop: Option<MotionCursor>,
    last_x: i32,
    last_y: i32,
    phase: u64,
    hidden: bool,
    last_flip_cycle: u64,
    env: IdleEnv,
    rng: SplitMix64Rng,
}

impl IdleMotor {
    fn new(x: f64, y: f64, now_us: u64, next_bump_us: u64, env: IdleEnv) -> Self {
        Self {
            anchor_x: x,
            anchor_y: y,
            ax: 0.0,
            ay: 0.0,
            x,
            y,
            last_emit_us: now_us,
            last_tick_us: now_us,
            next_bump_us,
            hop: None,
            last_x: x.round() as i32,
            last_y: y.round() as i32,
            phase: 0,
            hidden: false,
            last_flip_cycle: u64::MAX,
            env,
            rng: SplitMix64Rng::new(env.seed ^ seeds::SALT_IDLE_MOTOR_RNG),
        }
    }

    fn p(&self) -> Persona {
        self.env.persona
    }

    fn tick(&mut self, now_us: u64, out: &mut SmallVec<[RawEvent; 32]>) {
        let t_ms = now_us / 1000;
        if let Some(flip) = visibility_flip(self.env.seed, self.env.ctx_id, t_ms)
            && flip.cycle_idx != self.last_flip_cycle
        {
            self.last_flip_cycle = flip.cycle_idx;
            self.hidden = flip.new_state == VS_HIDDEN;
            let dt = dt_ms_u16(now_us, self.last_emit_us);
            out.push(RawEvent::new(0, 0, dt, input::VISIBILITY, flip.new_state));
            self.last_emit_us = now_us;
        }
        if self.hidden {
            self.hop = None;
            self.last_tick_us = now_us;
            return;
        }

        if let Some(hop) = self.hop.as_mut() {
            if let Some(ev) = hop.step(now_us) {
                out.push(ev);
                self.last_emit_us = now_us;
            }
            self.x = hop.pos().0 as f64;
            self.y = hop.pos().1 as f64;
            if !hop.done() {
                return;
            }
            self.hop = None;
            self.reschedule(now_us);
        }
        if self.hop.is_none() && now_us >= self.next_bump_us {
            self.phase = self.phase.wrapping_add(1);
            let hop_seed = mix_ctx(self.env.seed, self.phase);
            let reach = if self.rng.chance(0.18) { 22.0 } else { 8.0 };
            let to_x = self.x + self.rng.gauss() * reach;
            let to_y = self.y + self.rng.gauss() * reach;
            self.hop = Some(MotionCursor::new(MotionStart {
                persona: self.env.persona,
                from_x: self.x,
                from_y: self.y,
                to_x,
                to_y,
                target_w: self.p().idle_hop_w_px,
                now_us,
                seed: hop_seed,
                trust: 0,
                display_hz: self.env.display_hz,
                hw: self.env.hw,
            }));
            return;
        }
        self.drift(now_us, out);
    }

    fn drift(&mut self, now_us: u64, out: &mut SmallVec<[RawEvent; 32]>) {
        let p = self.p();
        let dt_us = now_us.saturating_sub(self.last_tick_us).max(1);
        self.last_tick_us = now_us;
        let dt = (dt_us as f64 / 1_000_000.0).clamp(0.001, 0.25);
        let theta_dt = p.idle_theta * dt;
        let sqrt_dt = dt.sqrt();
        self.ax = core_utils::ou_step(
            self.ax,
            theta_dt,
            p.idle_noise_px_s * sqrt_dt * self.rng.gauss(),
        );
        self.ay = core_utils::ou_step(
            self.ay,
            theta_dt,
            p.idle_noise_px_s * sqrt_dt * self.rng.gauss(),
        );
        self.x += self.ax * dt;
        self.y += self.ay * dt;
        self.anchor_x += self.rng.gauss() * p.idle_anchor_wander * dt;
        self.anchor_y += self.rng.gauss() * p.idle_anchor_wander * dt;
        self.x += (self.anchor_x - self.x) * p.idle_anchor_pull * dt;
        self.y += (self.anchor_y - self.y) * p.idle_anchor_pull * dt;
        let ex = self.x.round() as i32;
        let ey = self.y.round() as i32;
        if (ex != self.last_x || ey != self.last_y)
            && now_us - self.last_emit_us >= IDLE_EMIT_GAP_US
        {
            let dt = dt_ms_u16(now_us, self.last_emit_us);
            out.push(RawEvent::new(
                coord_u16(ex as f64),
                coord_u16(ey as f64),
                dt,
                input::MOVE,
                button::LEFT,
            ));
            self.last_x = ex;
            self.last_y = ey;
            self.last_emit_us = now_us;
        }
        if self.rng.chance(p.idle_scroll_p) {
            let dy = 1 + self.rng.next_below(4) as u16;
            let dt = dt_ms_u16(now_us, self.last_emit_us).max(8);
            out.push(RawEvent::new(0, dy, dt, input::WHEEL, 0));
            self.last_emit_us = now_us;
        }
    }

    fn reschedule(&mut self, now_us: u64) {
        self.next_bump_us = now_us + self.rng.lognormal_us(self.p().hop_interval_median_ms, 0.45);
    }
}

pub struct SessionStart {
    pub persona: Persona,
    pub seed: u64,
    pub ctx_id: u64,
    pub from: (f64, f64),
    pub target: (f64, f64),
    pub target_w: f64,
    pub text: Option<String>,
    pub now_us: u64,
    pub trust: i32,
    pub display_hz: u32,
    pub hw: MouseHardware,
}

impl SessionStart {
    pub fn for_profile(profile: &session_state::Profile, ctx_id: u64, now_us: u64) -> Self {
        Self {
            persona: profile.persona,
            seed: mix_ctx(profile.canvas_seed, ctx_id),
            ctx_id,
            from: (0.0, 0.0),
            target: (0.0, 0.0),
            target_w: 30.0,
            text: None,
            now_us,
            trust: 0,
            display_hz: profile.emit_hz(),
            hw: profile.hw,
        }
    }
}

pub struct TabSession {
    persona: Persona,
    phase: TabPhase,
    active: Active,
    x: f64,
    y: f64,
    text: Option<String>,
    seed: u64,
    ctx_id: u64,
    display_hz: u32,
    hw: MouseHardware,
    rng: SplitMix64Rng,
    idle: Option<IdleMotor>,
    next_due_us: u64,
    finished: bool,
}

impl TabSession {
    pub fn start(start: SessionStart) -> Self {
        let SessionStart {
            persona,
            seed,
            ctx_id,
            from,
            target,
            target_w,
            text,
            now_us,
            trust,
            display_hz,
            hw,
        } = start;
        let move_cursor = MotionCursor::new(MotionStart {
            persona,
            from_x: from.0,
            from_y: from.1,
            to_x: target.0,
            to_y: target.1,
            target_w,
            now_us,
            seed,
            trust,
            display_hz,
            hw,
        });
        Self {
            persona,
            phase: TabPhase::Approach,
            active: Active::Move(move_cursor),
            x: from.0,
            y: from.1,
            text,
            seed,
            ctx_id,
            display_hz,
            hw,
            rng: SplitMix64Rng::new(seed ^ seeds::SALT_SESSION_RNG),
            idle: None,
            next_due_us: now_us,
            finished: false,
        }
    }

    #[inline]
    pub fn finished(&self) -> bool {
        self.finished
    }

    #[inline]
    pub fn next_due_us(&self) -> u64 {
        self.next_due_us
    }

    pub fn advance(&mut self, now_us: u64) -> TabTick {
        let mut events: SmallVec<[RawEvent; 32]> = SmallVec::new();
        self.drive(now_us, &mut events);
        TabTick {
            events,
            next_due_us: self.next_due_us,
            phase: self.phase,
        }
    }

    fn drive(&mut self, now_us: u64, out: &mut SmallVec<[RawEvent; 32]>) {
        if self.finished || now_us < self.next_due_us {
            return;
        }
        let is_move = matches!(self.active, Active::Move(_));
        let Some((cursor, kind)) = self.active.slot() else {
            self.idle_tick(now_us, out);
            return;
        };
        if let Some(ev) = cursor.step(now_us) {
            if is_move {
                self.x = f64::from(ev.x);
                self.y = f64::from(ev.y);
            }
            out.push(ev);
        }
        self.next_due_us = cursor.next_due_us();
        if cursor.done() {
            match kind {
                FinishedKind::Move => self.enter_focus(now_us),
                FinishedKind::Focus => self.enter_click(now_us),
                FinishedKind::Interact => self.enter_next(now_us),
                FinishedKind::Scroll => self.enter_idle(now_us),
            }
        }
    }

    fn set(&mut self, phase: TabPhase, active: Active, due_us: u64) {
        self.phase = phase;
        self.active = active;
        self.next_due_us = due_us;
    }

    fn enter_focus(&mut self, now_us: u64) {
        let (ox, oy) = click_offset(60.0, 24.0, self.seed ^ seeds::SALT_CLICK_OFFSET_TAB);
        self.x = (self.x + ox).clamp(0.0, u16::MAX as f64);
        self.y = (self.y + oy).clamp(0.0, u16::MAX as f64);
        let focus = FocusCursor::new(
            coord_u16(self.x),
            coord_u16(self.y),
            now_us,
            self.seed ^ seeds::SALT_FOCUS_ENTER,
        );
        self.set(TabPhase::Approach, Active::Focus(focus), now_us);
    }

    fn enter_click(&mut self, now_us: u64) {
        let click = ClickCursor::new(
            self.persona,
            coord_u16(self.x),
            coord_u16(self.y),
            now_us,
            self.seed ^ seeds::SALT_CLICK_ENTER,
        );
        self.set(TabPhase::Click, Active::Click(click), now_us);
    }

    fn enter_next(&mut self, now_us: u64) {
        match self.phase {
            TabPhase::Click => {
                if let Some(text) = self.text.take() {
                    let typing = TypingCursor::new(
                        self.persona,
                        &text,
                        now_us,
                        self.seed ^ seeds::SALT_TAB_SESSION_RNG,
                    );
                    self.set(TabPhase::Typing, Active::Type(typing), now_us);
                    return;
                }
                self.enter_reading(now_us);
            }
            TabPhase::Typing => self.enter_reading(now_us),
            _ => self.enter_idle(now_us),
        }
    }

    fn enter_reading(&mut self, now_us: u64) {
        let scroll_px = -(self.persona.reading_scroll_px * (0.85 + self.rng.next_f64() * 0.30));
        let scroll = ScrollCursor::new(
            self.persona,
            scroll_px,
            now_us,
            self.seed ^ seeds::SALT_SCROLL_ENTER,
        );
        self.set(TabPhase::Reading, Active::Scroll(scroll), now_us);
    }

    fn enter_idle(&mut self, now_us: u64) {
        let bump = now_us + self.rng.next_range(4000, 12000) as u64 * 1000;
        let env = IdleEnv {
            persona: self.persona,
            seed: self.seed,
            ctx_id: self.ctx_id,
            display_hz: self.display_hz,
            hw: self.hw,
        };
        self.idle = Some(IdleMotor::new(self.x, self.y, now_us, bump, env));
        self.set(TabPhase::Idle, Active::None, now_us + 40_000);
    }

    fn idle_tick(&mut self, now_us: u64, out: &mut SmallVec<[RawEvent; 32]>) {
        let Some(idle) = self.idle.as_mut() else {
            self.next_due_us = now_us + IDLE_TICK_MIN_US;
            return;
        };
        idle.tick(now_us, out);
        self.x = idle.x;
        self.y = idle.y;
        if let Some(hop) = idle.hop.as_ref() {
            self.next_due_us = hop.next_due_us().max(now_us + 1);
        } else {
            let jitter = self.rng.next_below(IDLE_TICK_SPAN_US as u64);
            self.next_due_us = (now_us + IDLE_TICK_MIN_US + jitter)
                .min(idle.next_bump_us.max(now_us + IDLE_TICK_MIN_US));
        }
    }
}

pub fn placement(profile: &session_state::Profile, ctx_id: u64) -> ((f64, f64), (f64, f64)) {
    let (vw, vh) = profile.viewport();
    let mut rng = SplitMix64Rng::new(mix_ctx(profile.canvas_seed, ctx_id));
    (
        view_point(&mut rng, vw, vh, 0.08, 0.1, 0.1, 0.12),
        view_point(&mut rng, vw, vh, 0.45, 0.4, 0.25, 0.25),
    )
}

#[inline]
fn view_point(
    rng: &mut SplitMix64Rng,
    vw: f64,
    vh: f64,
    x0: f64,
    y0: f64,
    x_span: f64,
    y_span: f64,
) -> (f64, f64) {
    (
        vw * (x0 + rng.next_f64() * x_span),
        vh * (y0 + rng.next_f64() * y_span),
    )
}

pub fn interaction_events_for(
    profile: &session_state::Profile,
    href: &str,
    trust: i32,
) -> SmallVec<[RawEvent; 128]> {
    let (vw, vh) = profile.viewport();
    let seed = mix_ctx(profile.canvas_seed, core_utils::xxh3::hash(href.as_bytes()));
    let mut rng = SplitMix64Rng::new(seed ^ seeds::SALT_INBOUND);
    let target = view_point(&mut rng, vw, vh, 0.35, 0.28, 0.30, 0.34);
    let mut rng = SplitMix64Rng::new(seed ^ seeds::SALT_INTERACTION_RNG);
    let from = view_point(&mut rng, vw, vh, 0.05, 0.06, 0.12, 0.14);
    let mut out: SmallVec<[RawEvent; 128]> = SmallVec::new();
    let now_us = 0u64;
    let mut now = now_us;
    let (ox, oy) = click_offset(60.0, 24.0, seed ^ seeds::SALT_CLICK_OFFSET_TAB);
    let target = (target.0 + ox, target.1 + oy);
    let mut cur = MotionCursor::new(MotionStart {
        persona: profile.persona,
        from_x: from.0,
        from_y: from.1,
        to_x: target.0,
        to_y: target.1,
        target_w: 60.0,
        now_us: now,
        seed: seed ^ seeds::SALT_MOTION_RNG,
        trust,
        display_hz: profile.emit_hz(),
        hw: profile.hw,
    });
    pump(&mut cur, &mut now, now_us + 5_000_000, &mut out);
    let (ex, ey) = cur.pos();
    let mut focus = FocusCursor::new(
        coord_u16(ex as f64),
        coord_u16(ey as f64),
        now,
        seed ^ seeds::SALT_INTERACTION_TARGET,
    );
    pump(&mut focus, &mut now, now_us + 6_000_000, &mut out);
    let mut click = ClickCursor::new(
        profile.persona,
        coord_u16(ex as f64),
        coord_u16(ey as f64),
        now,
        seed ^ seeds::SALT_CLICK_CURSOR,
    );
    pump(&mut click, &mut now, now_us + 7_000_000, &mut out);
    out
}

pub struct TelemetryBatcher {
    cap: usize,
    interval_us: u64,
    len: usize,
    started_us: u64,
}

impl TelemetryBatcher {
    pub fn new(cap: usize, interval_us: u64) -> Self {
        Self {
            cap,
            interval_us,
            len: 0,
            started_us: 0,
        }
    }

    #[inline]
    pub fn begin(&mut self, now_us: u64) {
        self.len = 0;
        self.started_us = now_us;
    }

    #[inline]
    pub fn feed(&mut self, n: usize) {
        self.len += n;
    }

    #[inline]
    pub fn ready(&self, now_us: u64) -> bool {
        self.len >= self.cap
            || (self.len > 0 && now_us.saturating_sub(self.started_us) >= self.interval_us)
    }
}

pub const BATCH_CAP: usize = 64;
const BATCH_INTERVAL_MIN_US: u64 = 1_200_000;
const BATCH_INTERVAL_MAX_US: u64 = 2_800_000;

#[inline]
pub fn batch_interval_for(seed: u64) -> u64 {
    core_utils::Identity::new(seed)
        .at(seeds::SALT_BATCH_INTERVAL)
        .u64_in(BATCH_INTERVAL_MIN_US, BATCH_INTERVAL_MAX_US)
}
