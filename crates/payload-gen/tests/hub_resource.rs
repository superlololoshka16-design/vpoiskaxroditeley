fn hw_stub() -> payload_gen::input::MouseHardware {
    payload_gen::input::MouseHardware {
        poll_hz: 125,
        dpi: 1000,
        pointer_speed: 10,
        accel: true,
        battery_saver: false,
        kind: 0,
    }
}

use payload_gen::input::{InputHub, MotionCursor, MotionStart, Persona, SlotId, TabInput};
use smallvec::SmallVec;

fn motion(due: u64, seed: u64) -> TabInput {
    TabInput::Move(MotionCursor::new(MotionStart {
        persona: Persona::derive(1),
        from_x: 0.0,
        from_y: 0.0,
        to_x: 100.0,
        to_y: 50.0,
        target_w: 20.0,
        now_us: due,
        seed,
        trust: 0,
        display_hz: 60,
        hw: hw_stub(),
    }))
}

#[test]
fn replacing_input_keeps_one_live_driver() {
    let mut hub = InputHub::new();
    let site = hub.register_site(1);
    let tab = hub.open_tab(site, 1);
    for due in 0..1000 {
        hub.set_input(tab, motion(due, due));
    }
    let mut out: SmallVec<[(SlotId, payload_gen::input::RawEvent); 32]> = SmallVec::new();
    hub.tick(1_000_000, &mut out);
    assert!(!out.is_empty());
    assert!(out.iter().all(|(t, _)| *t == tab));
    assert_eq!(hub.live_tabs(), 1);
    assert!(hub.next_due_us() < u64::MAX);
}

#[test]
fn closed_tab_does_not_leave_work_for_a_reused_slot() {
    let mut hub = InputHub::new();
    let site = hub.register_site(1);
    let first = hub.open_tab(site, 1);
    hub.set_input(first, motion(10, 1));
    hub.close_tab(first);
    assert_eq!(hub.next_due_us(), u64::MAX);
    let second = hub.open_tab(site, 1);
    assert_eq!(first, second);
    assert_eq!(hub.next_due_us(), u64::MAX);
    assert_eq!(hub.live_tabs(), 1);
}

#[test]
fn unused_site_storage_is_reused() {
    let mut hub = InputHub::new();
    for _ in 0..1000 {
        let site = hub.register_site(1);
        let tab = hub.open_tab(site, 1);
        hub.set_input(tab, motion(10, 2));
        assert!(!hub.unregister_site(site));
        hub.close_tab(tab);
        assert!(hub.unregister_site(site));
        assert!(!hub.unregister_site(site));
        assert_eq!(hub.live_tabs(), 0);
    }
    assert_eq!(hub.next_due_us(), u64::MAX);
}
