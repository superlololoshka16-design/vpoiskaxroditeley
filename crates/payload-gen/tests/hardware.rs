use session_state::{MouseHardware, Persona, Platform, Profile, hardware_of};

fn prof(kind: u8, speed: u8, saver: bool, mobile: bool) -> Profile {
    let mut p = Profile::shell();
    p.pointer_kind = kind;
    p.pointer_speed = speed;
    p.battery_saver = saver;
    p.platform = if mobile {
        Platform::Android
    } else {
        Platform::Windows
    };
    p
}

#[test]
fn gain_curve_is_monotone_when_accel_on() {
    let hw = MouseHardware {
        poll_hz: 125,
        dpi: 1000,
        pointer_speed: 10,
        accel: true,
        battery_saver: false,
        kind: 0,
    };
    let mut last = 0.0f64;
    for i in 0..=30 {
        let v = hw.gain(f64::from(i) * 6.4);
        assert!(v >= last - 1e-12);
        last = v;
    }
    assert!(hw.gain(180.0) > hw.gain(1.0));
}

#[test]
fn gain_is_flat_when_accel_off() {
    let hw = MouseHardware {
        accel: false,
        ..MouseHardware {
            poll_hz: 125,
            dpi: 1000,
            pointer_speed: 10,
            accel: true,
            battery_saver: false,
            kind: 0,
        }
    };
    assert_eq!(hw.gain(1.0), 1.0);
    assert_eq!(hw.gain(180.0), 1.0);
}

#[test]
fn poll_quantum_snaps_to_valid_hz_grid() {
    let mut hw = MouseHardware {
        poll_hz: 125,
        dpi: 1000,
        pointer_speed: 10,
        accel: true,
        battery_saver: false,
        kind: 0,
    };
    for (hz, want) in [(125u32, 8000u64), (250, 4000), (500, 2000), (1000, 1000)] {
        hw.poll_hz = hz;
        assert_eq!(hw.poll_quantum_us(), want);
    }
}

#[test]
fn garbage_hz_still_lands_in_clamped_grid() {
    let mut hw = MouseHardware {
        poll_hz: 1,
        dpi: 1000,
        pointer_speed: 10,
        accel: true,
        battery_saver: false,
        kind: 0,
    };
    assert_eq!(hw.poll_quantum_us(), 1_000_000 / 60);
    hw.poll_hz = 10_000;
    assert_eq!(hw.poll_quantum_us(), 1000);
}

#[test]
fn hardware_of_mobile_forces_touch_kind() {
    let mut p = prof(session_state::POINTER_MOUSE, 10, false, true);
    p.persona = Persona::derive(p.canvas_seed);
    let hw = hardware_of(&p, p.persona);
    assert_eq!(hw.kind, session_state::POINTER_TOUCH);
    assert!(!hw.accel);
}

#[test]
fn hardware_of_touchpad_ignores_gamer_roll() {
    let mut p = prof(session_state::POINTER_TOUCHPAD, 20, false, false);
    p.persona = Persona::derive(p.canvas_seed);
    let hw = hardware_of(&p, p.persona);
    assert_eq!(hw.poll_hz, 125);
    assert!(hw.accel);
    assert_eq!(hw.dpi, 96);
}

#[test]
fn hardware_of_mouse_splits_office_and_gamer() {
    let mut p = prof(session_state::POINTER_MOUSE, 10, false, false);
    let mut office = 0usize;
    let mut gamer = 0usize;
    for ctx in 0..512u64 {
        p.canvas_seed = ctx;
        let per = Persona::derive(p.canvas_seed);
        let hw = hardware_of(&p, per);
        if hw.accel {
            office += 1;
        } else {
            gamer += 1;
        }
    }
    assert!(office > 0 && gamer > 0, "both classes must appear");
    assert!(gamer < office, "gamer rigs are the minority");
}

#[test]
fn battery_saver_propagates_and_throttles() {
    let mut p = prof(session_state::POINTER_MOUSE, 10, true, false);
    p.persona = Persona::derive(p.canvas_seed);
    let hw = hardware_of(&p, p.persona);
    assert!(hw.battery_saver);
    assert!((hw.throttle_factor() - 1.22).abs() < 1e-9);
}
