use payload_gen::input::{
    Calibration, ClickCursor, InputHub, MotionCursor, MotionStart, Persona, RawEvent, ScrollCursor,
    SessionStart, TabInput, TypingCursor, events_bytes,
};

fn profile_stub() -> session_state::Profile {
    session_state::Profile::shell()
}

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

fn run_motion(seed: u64, trust: i32) -> (Vec<RawEvent>, bool) {
    run_motion_as(seed, trust, 0xA11CE)
}

fn run_motion_as(seed: u64, trust: i32, pseed: u64) -> (Vec<RawEvent>, bool) {
    let persona = Persona::derive(pseed);
    let mut cur = MotionCursor::new(MotionStart::plain(
        persona, 120.0, 90.0, 640.0, 400.0, 36.0, 1_000_000, seed, trust, 60,
    ));
    let mut events = Vec::new();
    let mut now = 1_000_000u64;
    while !cur.done() && now < 60_000_000 {
        if let Some(ev) = cur.step(now) {
            events.push(ev);
        }
        now = cur.next_due_us().max(now + 1);
    }
    let on = cur.on_target();
    (events, on)
}

#[test]
fn tab_session_runs_full_lifecycle_into_idle() {
    let persona = Persona::derive(0x1FEE);
    let mut tab = payload_gen::input::TabSession::start(payload_gen::input::SessionStart {
        persona,
        seed: 0x1FEE,
        ctx_id: 11,
        from: (120.0, 90.0),
        target: (600.0, 400.0),
        target_w: 30.0,
        text: Some("hello gate".to_string()),
        now_us: 0,
        trust: 0,
        display_hz: 60,
        hw: profile_stub().hw,
    });
    let mut seen = std::collections::HashSet::new();
    let mut events = 0u64;
    let mut now = 0u64;
    let mut idle_events = 0u64;
    while now < 120_000_000 {
        let tick = tab.advance(now);
        now = tick.next_due_us.max(now + 1);
        seen.insert(tick.phase);
        events += tick.events.len() as u64;
        if tick.phase == payload_gen::input::TabPhase::Idle {
            idle_events += tick.events.len() as u64;
        }
    }
    assert!(seen.contains(&payload_gen::input::TabPhase::Approach));
    assert!(seen.contains(&payload_gen::input::TabPhase::Click));
    assert!(seen.contains(&payload_gen::input::TabPhase::Typing));
    assert!(seen.contains(&payload_gen::input::TabPhase::Reading));
    assert!(seen.contains(&payload_gen::input::TabPhase::Idle));
    assert!(events > 30, "жизнь вкладки не бывает 5 точками: {events}");
    assert!(idle_events > 0, "идл обязан слать логику, а не молчать");
}

#[test]
fn motion_reaches_target_with_variable_timing() {
    for pseed in [0x1FEEu64, 0xC0FFEE, 0xD0D0] {
        assert_timing_variable(0x5EED_0001, pseed);
    }
}

fn assert_timing_variable(seed: u64, pseed: u64) {
    let (events, on) = run_motion_as(seed, 0, pseed);
    assert!(on, "движение обязано закончиться в цели");
    assert!(
        events.len() > 15,
        "полёт на 600px не бывает 5 точками: {}",
        events.len()
    );
    let dts: Vec<u16> = events.iter().map(|e| e.dt_ms).collect();
    let uniq = dts.iter().collect::<std::collections::HashSet<_>>().len();
    assert!(
        uniq > 3,
        "константный дельта-тайм = маркер бота, уников: {uniq}"
    );
    assert!(dts.iter().all(|&d| d > 0));
    let mut peak = 0.0f64;
    for w in events.windows(2) {
        let dx = (w[1].x as f64 - w[0].x as f64).abs();
        let dy = (w[1].y as f64 - w[0].y as f64).abs();
        let step = dx.hypot(dy) / w[1].dt_ms.max(1) as f64;
        if step > peak {
            peak = step;
        }
    }
    assert!(peak > 0.0, "нулевая скорость на протяжении пути");
}

#[test]
fn motion_shape_is_not_a_fixed_template() {
    let frac = |seed: u64| {
        let (events, _) = run_motion(seed, 0);
        let n = events.len().max(1);
        let mut peak_i = 0usize;
        let mut peak = 0.0f64;
        for (i, w) in events.windows(2).enumerate() {
            let dx = (w[1].x as f64 - w[0].x as f64).abs();
            let dy = (w[1].y as f64 - w[0].y as f64).abs();
            let step = dx.hypot(dy) / w[1].dt_ms.max(1) as f64;
            if step > peak {
                peak = step;
                peak_i = i;
            }
        }
        peak_i as f64 / n as f64
    };
    let f1 = frac(0x5EED_0002);
    let f2 = frac(0x5EED_0003);
    let f3 = frac(0x5EED_0004);
    assert!(
        (f1 - f2).abs() > 0.02 || (f2 - f3).abs() > 0.02,
        "пик скорости у всех сидов в одной точке = фиксированный шаблон фаз: {f1} {f2} {f3}"
    );
}

#[test]
fn motion_is_deterministic_per_seed() {
    let (a, _) = run_motion(0x5EED_0005, 0);
    let (b, _) = run_motion(0x5EED_0005, 0);
    assert_eq!(a.len(), b.len());
    assert!(
        a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.x == y.x && x.y == y.y && x.dt_ms == y.dt_ms)
    );
}

#[test]
fn persona_is_stable_and_distinct_per_seed() {
    let p1 = Persona::derive(0xBEEF);
    let p1_again = Persona::derive(0xBEEF);
    let p2 = Persona::derive(0xBEE0);
    assert_eq!(p1.wpm, p1_again.wpm);
    assert_ne!(p1.wpm, p2.wpm, "идентичности обязаны отличаться моторикой");
}

#[test]
fn typing_has_burst_rhythm_not_flat_gaps() {
    let persona = Persona::derive(0xC0FFEE);
    let mut cur = TypingCursor::new(
        persona,
        "the quick brown fox jumps over the lazy dog",
        0,
        0x7EED,
    );
    let mut gaps: Vec<u64> = Vec::new();
    let mut now = 0u64;
    while !cur.done() && now < 60_000_000 {
        let due = cur.next_due_us();
        if due > now {
            now = due;
        }
        if cur.step(now).is_some() {
            gaps.push(cur.next_due_us() - now);
        } else if cur.done() {
            break;
        }
    }
    assert!(gaps.len() > 30);
    let mut sorted = gaps.clone();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    let max = *sorted.last().unwrap();
    assert!(
        max > median * 3,
        "без бёрстов все интервалы плоские: медиана {median}, максимум {max}"
    );
}

#[test]
fn click_dwell_is_lognormal_band() {
    let persona = Persona::derive(0xD0D0);
    let mut holds = Vec::new();
    for i in 0..40 {
        let mut cur = ClickCursor::new(persona, 300, 200, 1_000_000, 0x9E37 + i as u64);
        let mut now = 1_000_000u64;
        let mut press_t = 0u64;
        let mut release_t = 0u64;
        while !cur.done() && now < 5_000_000 {
            if let Some(ev) = cur.step(now) {
                if ev.kind == payload_gen::input::event::kind::PRESS {
                    press_t = now;
                }
                if ev.kind == payload_gen::input::event::kind::RELEASE {
                    release_t = now;
                }
            }
            now = cur.next_due_us().max(now + 1);
        }
        holds.push(release_t.saturating_sub(press_t) / 1000);
    }
    let mut sorted = holds.clone();
    sorted.sort_unstable();
    let min = *sorted.first().unwrap();
    let max = *sorted.last().unwrap();
    let p05 = sorted[sorted.len() / 20];
    let p95 = sorted[sorted.len() - 1 - sorted.len() / 20];
    assert!(min >= 25, "dwell ниже 25ms физически невозможен: {min}");
    assert!(max <= 500, "dwell выше 500ms палится: {max}");
    assert!(
        p05 >= 40 && p95 <= 380,
        "рабочая полоса dwell обязана быть человеческой: p05={p05} p95={p95}"
    );
    assert!(max > min, "константный dwell = маркер бота");
}

#[test]
fn scroll_decays_and_lands() {
    let persona = Persona::derive(0xE11);
    let mut cur = ScrollCursor::new(persona, -900.0, 0, 0x5C20);
    let mut deltas = Vec::new();
    let mut now = 0u64;
    let mut last_y = 0i32;
    while !cur.done() && now < 20_000_000 {
        if let Some(ev) = cur.step(now) {
            let y = ev.y as i32;
            deltas.push((y - last_y).abs());
            last_y = y;
        }
        now = cur.next_due_us().max(now + 1);
    }
    assert!(cur.done(), "скролл обязан доехать");
    assert!(deltas.len() > 3);
    let first = deltas.iter().take(3).sum::<i32>();
    let last = deltas.iter().rev().take(3).sum::<i32>();
    assert!(
        first > last,
        "трение обязано гасить скорость: {first} vs {last}"
    );
}

#[test]
fn raw_event_is_packed_pod() {
    assert_eq!(payload_gen::input::RAW_EVENT_LEN, 8);
    let events = [
        RawEvent::new(11, 22, 33, 4, 5),
        RawEvent::new(66, 77, 88, 1, 2),
    ];
    let bytes = events_bytes(&events);
    assert_eq!(bytes.len(), 16);
    assert_eq!(&bytes[..2], &[11, 0]);
}

#[test]
fn hub_prioritises_weight_and_calibration_changes_logic() {
    let mut hub = InputHub::new();
    let site = hub.register_site(3);
    let weak = hub.open_tab(site, 1);
    let strong = hub.open_tab(site, 9);
    let persona = Persona::derive(0xF00D);
    hub.set_input(
        weak,
        TabInput::Move(MotionCursor::new(MotionStart {
            persona,
            from_x: 0.0,
            from_y: 0.0,
            to_x: 500.0,
            to_y: 300.0,
            target_w: 30.0,
            now_us: 1_000,
            seed: 0x2,
            trust: 0,
            display_hz: 60,
            hw: hw_stub(),
        })),
    );
    hub.set_input(
        strong,
        TabInput::Move(MotionCursor::new(MotionStart {
            persona,
            from_x: 0.0,
            from_y: 0.0,
            to_x: 500.0,
            to_y: 300.0,
            target_w: 30.0,
            now_us: 1_000,
            seed: 0x2,
            trust: 0,
            display_hz: 60,
            hw: hw_stub(),
        })),
    );
    let mut out = smallvec::SmallVec::new();
    hub.tick(1_000_000, &mut out);
    assert!(!out.is_empty());
    assert_eq!(out[0].0, strong, "болящий вес берёт слот первым");
    assert_eq!(hub.live_tabs(), 2);

    let calib = Calibration::new(3);
    calib.record(false);
    calib.record(false);
    assert_eq!(calib.trust(), -8);
    assert_eq!(calib.stats().1, 2);
    let throttled = persona.throttle(calib.trust());
    assert!(throttled.corrections_max > persona.throttle(0).corrections_max);
}

#[test]
fn motion_paths_are_curved_not_straight() {
    let straightness = |seed: u64| {
        let (events, _) = run_motion(seed, 0);
        let (x0, y0) = (
            events.first().map(|e| e.x as f64).unwrap_or(0.0),
            events.first().map(|e| e.y as f64).unwrap_or(0.0),
        );
        let (x1, y1) = (
            events.last().map(|e| e.x as f64).unwrap_or(0.0),
            events.last().map(|e| e.y as f64).unwrap_or(0.0),
        );
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let mut area = 0.0f64;
        for e in &events[1..events.len() - 1] {
            let px = (e.x as f64 - x0) * dy - (e.y as f64 - y0) * dx;
            area = area.max(px.abs() / len);
        }
        area
    };
    let d1 = straightness(0x5EED_0011);
    let d2 = straightness(0x5EED_0012);
    let d3 = straightness(0x5EED_0013);
    let best = d1.max(d2).max(d3);
    assert!(
        best > 1.0,
        "хотя бы одна траектория обязана отходить от прямой: {best}"
    );
}

#[test]
fn idle_hidden_mutes_pointer_noise() {
    let persona = Persona::derive(0x1D1E);
    let mut tab = payload_gen::input::TabSession::start(SessionStart {
        persona,
        seed: 0x1D1E,
        ctx_id: 21,
        from: (300.0, 300.0),
        target: (420.0, 320.0),
        target_w: 30.0,
        text: None,
        now_us: 0,
        trust: 0,
        display_hz: 60,
        hw: hw_stub(),
    });
    let mut now = 0u64;
    let mut flips = 0u32;
    let mut hidden_moves = 0u64;
    let mut visible_moves = 0u64;
    let mut hidden = false;
    while now < 3_600_000_000 {
        let tick = tab.advance(now);
        now = tick.next_due_us.max(now + 1);
        for ev in tick.events {
            if ev.kind == payload_gen::input::event::kind::VISIBILITY {
                flips += 1;
                hidden = ev.arg == 1;
            } else if ev.kind == payload_gen::input::event::kind::MOVE {
                if hidden {
                    hidden_moves += 1;
                } else {
                    visible_moves += 1;
                }
            }
        }
    }
    assert!(flips >= 2, "визibility флипы обязаны случаться");
    assert_eq!(hidden_moves, 0, "в hidden вкладка не шлёт мышь");
    assert!(visible_moves > 0, "в visible мышь живая");
}

#[test]
fn touch_hardware_samples_at_120hz() {
    let mut prof = session_state::Profile::shell();
    prof.platform = session_state::Platform::Android;
    prof.persona = session_state::Persona::derive(prof.canvas_seed);
    let t = session_state::hardware_of(&prof, prof.persona);
    assert_eq!(
        t.poll_hz, 120,
        "мобильный профиль опрашивается тач-скринером 120Hz"
    );
    let mut d = profile_stub();
    d.persona = session_state::Persona::derive(d.canvas_seed);
    let dh = session_state::hardware_of(&d, d.persona);
    assert!(
        dh.poll_hz == 125 || dh.poll_hz == 250 || dh.poll_hz == 500 || dh.poll_hz == 1000,
        "десктопная мышь живёт в классах 125/250/500/1000Hz: {}",
        dh.poll_hz
    );
}

#[test]
fn motion_emit_period_follows_display_hz() {
    let emit_stats = |hz: u32| {
        let persona = Persona::derive(0x9D9);
        let mut cur = MotionCursor::new(MotionStart {
            persona,
            from_x: 100.0,
            from_y: 100.0,
            to_x: 700.0,
            to_y: 500.0,
            target_w: 30.0,
            now_us: 0,
            seed: 0xA,
            trust: 0,
            display_hz: hz,
            hw: hw_stub(),
        });
        let mut events = Vec::new();
        let mut now = 0u64;
        while !cur.done() && now < 30_000_000 {
            if let Some(ev) = cur.step(now) {
                events.push(ev);
            }
            now = cur.next_due_us().max(now + 1);
        }
        let n = events.len() as f64;
        let span_ms = events.iter().map(|e| e.dt_ms as f64).sum::<f64>().max(1.0);
        n / span_ms * 1000.0
    };
    let h60 = emit_stats(60);
    let h120 = emit_stats(120);
    assert!(
        h120 > h60 * 1.5,
        "120Hz дисплей обязан выдавать почти вдвое больше точек: {h60} vs {h120}"
    );
}

#[test]
fn correlated_noise_keeps_axes_alive() {
    let persona = Persona::derive(0x12C0);
    let mut cur = MotionCursor::new(MotionStart {
        persona,
        from_x: 50.0,
        from_y: 50.0,
        to_x: 900.0,
        to_y: 60.0,
        target_w: 24.0,
        now_us: 0,
        seed: 0x3C,
        trust: 0,
        display_hz: 60,
        hw: hw_stub(),
    });
    let mut ys = std::collections::HashSet::new();
    let mut now = 0u64;
    let mut n = 0u64;
    while !cur.done() && now < 30_000_000 {
        if let Some(ev) = cur.step(now) {
            ys.insert(ev.y);
            n += 1;
        }
        now = cur.next_due_us().max(now + 1);
    }
    assert!(n > 20, "траектория не пустая: {n}");
    assert!(
        ys.len() > 5,
        "коррелированный шум не должен убивать вертикаль: {} уников по y",
        ys.len()
    );
}
