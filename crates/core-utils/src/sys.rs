use std::time::{Duration, SystemTime};

pub fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| {
            s.eq_ignore_ascii_case("1")
                || s.eq_ignore_ascii_case("true")
                || s.eq_ignore_ascii_case("yes")
                || s.eq_ignore_ascii_case("on")
        })
}

#[cfg(target_os = "linux")]
pub fn pin_thread(core: usize) {
    unsafe {
        unsafe extern "C" {
            fn sched_setaffinity(pid: i32, cpusetsize: usize, mask: *const u8) -> i32;
        }
        if core >= 1024 {
            return;
        }
        let mut set = [0u8; 128];
        set[core / 8] |= 1u8 << (core % 8);
        let _ = sched_setaffinity(0, set.len(), set.as_ptr());
    }
}

#[cfg(not(target_os = "linux"))]
pub fn pin_thread(core: usize) {
    let _ = core;
}

#[inline(always)]
fn epoch() -> Duration {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
}

#[inline]
pub fn unix_ms() -> u64 {
    epoch().as_millis() as u64
}

#[inline]
pub fn unix_us() -> u64 {
    epoch().as_micros() as u64
}

#[inline]
pub fn unix_ms_f64() -> f64 {
    epoch().as_secs_f64() * 1000.0
}

#[inline]
pub fn ms(a: std::time::Instant, b: std::time::Instant) -> u64 {
    b.saturating_duration_since(a).as_millis() as u64
}

pub fn env_present(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| v != "0")
}
