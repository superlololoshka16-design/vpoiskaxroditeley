use session_state::{Persona, Profile};

use payload_gen::input::{
    ClickCursor, FocusCursor, RawEvent, click_offset, event::kind as input,
    interaction_events_for,
};

fn persona() -> Persona {
    Persona::derive(0xDEAD_BEEF_CAFE_F00D)
}

fn profile_stub() -> Profile {
    Profile::shell()
}

#[test]
fn click_offset_stays_inside_fraction_box() {
    for seed in [0u64, 1, 7, 0xC0FF_EE00, u64::MAX / 2] {
        let (dx, dy) = click_offset(200.0, 80.0, seed);
        assert!(dx.abs() <= 200.0 * 0.30 + 1e-9, "dx out of box: {dx}");
        assert!(dy.abs() <= 80.0 * 0.30 + 1e-9, "dy out of box: {dy}");
    }
    let (dx, dy) = click_offset(0.0, 0.0, 42);
    assert!(dx.abs() <= 1.0);
    assert!(dy.abs() <= 1.0);
}

#[test]
fn focus_cursor_emits_focus_then_blur_then_ends() {
    let mut c = FocusCursor::new(100, 200, 1_000_000, 99);
    let mut saw_focus = false;
    let mut saw_blur = false;
    let mut now = 1_000_000u64;
    for _ in 0..100 {
        if let Some(ev) = c.step(now) {
            if !saw_focus {
                assert_eq!(ev.kind, input::FOCUS, "first emitted event must be FOCUS");
                saw_focus = true;
            } else {
                assert_eq!(ev.kind, input::BLUR, "second emitted event must be BLUR");
                saw_blur = true;
            }
        }
        now = c.next_due_us().max(now + 1);
        if c.done() {
            break;
        }
    }
    assert!(saw_focus, "focus never emitted");
    assert!(saw_blur, "blur never emitted");
    assert!(c.done(), "focus cursor never finished");
}

#[test]
fn focus_cursor_window_variant_uses_window_arg() {
    let mut c = FocusCursor::window(5_000_000, 5);
    let mut now = 5_000_000u64;
    let mut kinds = Vec::new();
    for _ in 0..100 {
        if let Some(ev) = c.step(now) {
            kinds.push(ev.kind);
        }
        now = c.next_due_us().max(now + 1);
        if c.done() {
            break;
        }
    }
    assert_eq!(kinds.len(), 2);
    assert_eq!(kinds[0], input::FOCUS);
    assert_eq!(kinds[1], input::BLUR);
}

#[test]
fn interaction_events_contain_focus_events() {
    let events = interaction_events_for(&profile_stub(), "https://a.example/", 0);
    assert!(!events.is_empty(), "script must emit events");
    assert!(
        events.iter().any(|e| e.kind == input::FOCUS),
        "script must contain focus events now"
    );
    assert!(
        events.iter().any(|e| e.kind == input::BLUR),
        "script must contain blur events now"
    );
    assert!(
        events.iter().any(|e| e.kind == input::PRESS),
        "script must still click"
    );
}

#[test]
fn events_stay_in_screen_bounds() {
    let mut p = profile_stub();
    p.screen_w = 320;
    p.screen_h = 240;
    for ev in interaction_events_for(&p, "https://a.example/", 0) {
        let (x, y) = (u32::from(ev.x), u32::from(ev.y));
        assert!(x <= 320, "x out of tiny viewport: {x}");
        assert!(y <= 240, "y out of tiny viewport: {y}");
    }
}

#[test]
fn click_cursor_finishes_with_plan() {
    let mut c = ClickCursor::new(persona(), 50, 60, 1_000_000, 11);
    let mut now = 1_000_000u64;
    let mut presses = 0;
    for _ in 0..200 {
        if let Some(ev) = c.step(now) {
            if ev.kind == input::PRESS {
                presses += 1;
            }
        }
        now = c.next_due_us().max(now + 1);
        if c.done() {
            break;
        }
    }
    assert!(c.done(), "click never finished");
    assert!(presses >= 1, "no press emitted");
}

#[test]
fn raw_event_layout_is_packed() {
    let ev = RawEvent::new(0x1FF, 0x1FF, 0x1FF, 0x0F, 0x0F);
    let bytes = payload_gen::input::events_bytes(core::slice::from_ref(&ev));
    assert_eq!(bytes.len(), 8, "RawEvent must stay 8 bytes");
}
