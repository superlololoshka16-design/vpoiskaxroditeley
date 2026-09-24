pub mod event;
pub mod motion;
pub mod scheduler;
pub mod session;
pub mod typing;

pub use event::{
    ClickCursor, ClickPlan, FocusCursor, RAW_EVENT_LEN, RawEvent, ScrollCursor, VisibilityFlip,
    button, click_offset, coord_u16, dt_ms_u16, events_bytes, focus_arg, visibility_flip,
};
pub use motion::{MotionCursor, MotionStart};
pub use scheduler::{Calibration, InputHub, SlotId, TabEvents, TabInput};
pub use session::{
    BATCH_CAP, SessionStart, TabPhase, TabSession, TabTick, TelemetryBatcher, batch_interval_for,
    interaction_events_for,
};
pub use session_state::{MouseHardware, Persona, ThrottledPersona, Tier};
pub use typing::{TypingCursor, bigram_latency_ms};

pub trait InputCursor {
    fn step(&mut self, now_us: u64) -> Option<RawEvent>;
    fn next_due_us(&self) -> u64;
    fn done(&self) -> bool;
}

#[macro_export]
macro_rules! input_cursor {
    ($ty:ty) => {
        impl $crate::input::InputCursor for $ty {
            fn step(&mut self, now_us: u64) -> Option<$crate::input::event::RawEvent> {
                self.step(now_us)
            }
            fn next_due_us(&self) -> u64 {
                self.next_due_us()
            }
            fn done(&self) -> bool {
                self.done()
            }
        }
    };
}

pub fn pump(
    cur: &mut dyn InputCursor,
    now: &mut u64,
    deadline_us: u64,
    out: &mut smallvec::SmallVec<[RawEvent; 128]>,
) {
    while !cur.done() && *now < deadline_us {
        if let Some(ev) = cur.step(*now) {
            out.push(ev);
        }
        *now = cur.next_due_us().max(*now + 1);
    }
}
