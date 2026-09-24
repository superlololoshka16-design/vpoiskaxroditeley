pub(crate) mod clock {
    use std::cell::Cell;
    use std::time::Instant;

    #[derive(Clone, Copy)]
    pub(crate) struct VirtualClock {
        start: Instant,
        origin_ms: f64,
        offset_us: u64,
        last_us: u64,
    }

    impl VirtualClock {
        fn new() -> Self {
            Self {
                start: Instant::now(),
                origin_ms: core_utils::unix_ms_f64(),
                offset_us: 0,
                last_us: 0,
            }
        }
    }

    fn elapsed_us(clk: &VirtualClock) -> u64 {
        clk.start.elapsed().as_micros() as u64 + clk.offset_us
    }

    thread_local! {
        static CLOCK: Cell<VirtualClock> = Cell::new(VirtualClock::new());
    }

    pub(crate) fn reset() {
        CLOCK.with(|c| c.set(VirtualClock::new()));
    }

    pub(crate) fn now_us() -> u64 {
        CLOCK.with(|c| elapsed_us(&c.get()))
    }

    pub(crate) fn now_ms_quantized() -> f64 {
        CLOCK.with(|c| {
            let mut clk = c.get();
            let q = elapsed_us(&clk) / 100 * 100;
            if q > clk.last_us {
                clk.last_us = q;
                c.set(clk);
            }
            clk.last_us as f64 / 1000.0
        })
    }

    pub(crate) fn time_origin_ms() -> f64 {
        CLOCK.with(|c| c.get().origin_ms)
    }

    pub(crate) fn add_offset_us(us: u64) {
        CLOCK.with(|c| {
            let mut clk = c.get();
            clk.offset_us = clk.offset_us.saturating_add(us);
            c.set(clk);
        })
    }
}


use core_utils::{FxBuild, bump_u32_id, fx_map};
use rquickjs::{Ctx, Function, Persistent, Value};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

const TIMER_CAP: usize = 4096;
const RAF_CAP: usize = 128;
const FIRE_LIMIT: usize = 4096;
const MICRO_LIMIT: usize = 1_000_000;
const RAF_JITTER_FRAC: u64 = 50;

#[inline]
fn frame_cadence_us(display_hz: u32, slot: u32) -> u64 {
    let hz = display_hz as u64;
    let base = 1_000_000u64 / hz;
    if hz == 60 {
        [17_000u64, 17_000, 16_000][slot as usize % 3]
    } else if hz == 120 {
        [8_333u64, 8_333, 8_334][slot as usize % 3]
    } else {
        base
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TimerKey {
    due_us: u64,
    seq: u64,
    id: u32,
    interval_us: u64,
}

impl Ord for TimerKey {
    #[inline]
    fn cmp(&self, o: &Self) -> Ordering {
        o.due_us.cmp(&self.due_us).then_with(|| o.seq.cmp(&self.seq))
    }
}

impl PartialOrd for TimerKey {
    #[inline]
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

struct TimerTail {
    cb: Persistent<Value<'static>>,
    args: Option<Persistent<Value<'static>>>,
}

struct TimerState {
    next_id: u32,
    seq: u64,
    live: usize,
    heap: BinaryHeap<TimerKey>,
    tails: HashMap<u32, TimerTail, FxBuild>,
}

struct RafState {
    frame_us: u64,
    cadence: u32,
    seed: u64,
    cbs: Vec<(u32, Persistent<Value<'static>>)>,
    next_id: u32,
    display_hz: u32,
}

thread_local! {
    static TIMERQ: RefCell<TimerState> = RefCell::new(TimerState { next_id: 1, seq: 0, live: 0, heap: BinaryHeap::new(), tails: fx_map() });
    static RAFQ: RefCell<RafState> = const { RefCell::new(RafState { frame_us: 0, cadence: 0, seed: 0, cbs: Vec::new(), next_id: 1, display_hz: 60 }) };
    static APPLY: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
    static MICRO: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
    static EMPTY_ARGS: RefCell<Option<Persistent<rquickjs::Array<'static>>>> = const { RefCell::new(None) };
}

pub(crate) fn init(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let f = crate::stackfmt::apply_caller(ctx)?;
    APPLY.with(|c| *c.borrow_mut() = Some(Persistent::save(ctx, f)));
    let micro: Function = ctx.eval("(function (f) { return Promise.resolve().then(f); })")?;
    MICRO.with(|c| *c.borrow_mut() = Some(Persistent::save(ctx, micro)));
    let empty = rquickjs::Array::new(ctx.clone())?;
    EMPTY_ARGS.with(|c| *c.borrow_mut() = Some(Persistent::save(ctx, empty)));
    Ok(())
}

pub(crate) fn microtask<'js>(ctx: &Ctx<'js>, cb: Value<'js>) -> rquickjs::Result<()> {
    if let Some(f) = crate::webidl::restore_slot(ctx, &MICRO) {
        let _: rquickjs::Result<Value<'js>> = f.call((cb,));
    }
    Ok(())
}

pub(crate) fn defer_in<'js, F>(ctx: &Ctx<'js>, delay_ms: f64, f: F) -> rquickjs::Result<()>
where
    F: Fn(&Ctx<'js>) + 'js,
{
    let fire = Function::new(ctx.clone(), move |c: Ctx<'js>| -> rquickjs::Result<()> {
        f(&c);
        Ok(())
    })?;
    let _ = schedule(ctx, fire.into_value(), delay_ms, None, false);
    Ok(())
}

#[inline]
pub(crate) fn defer<'js, F>(ctx: &Ctx<'js>, f: F) -> rquickjs::Result<()>
where
    F: Fn(&Ctx<'js>) + 'js,
{
    defer_in(ctx, 0.0, f)
}

pub(crate) fn defer_frame<'js, F>(ctx: &Ctx<'js>, f: F) -> rquickjs::Result<()>
where
    F: Fn(&Ctx<'js>) + 'js,
{
    let (hz, seed) = crate::worker::with_prof(|p| (p.display_hz, p.seed));
    let base = frame_cadence_us(hz, 0);
    let jitter = base / RAF_JITTER_FRAC + 1;
    let delay_us = base + core_utils::rng::mix_to_range(seed, 0x0B5E_12FA_CE00_0001, jitter);
    defer_in(ctx, delay_us as f64 / 1000.0, f)
}

pub(crate) fn clear_all() {
    TIMERQ.with(|q| {
        let mut q = q.borrow_mut();
        q.heap.clear();
        q.tails.clear();
        q.seq = 0;
        q.live = 0;
        q.next_id = 1;
    });
    RAFQ.with(|r| {
        let mut r = r.borrow_mut();
        r.cbs.clear();
        r.frame_us = 0;
        r.cadence = 0;
        r.seed = 0;
        r.next_id = 1;
    });
}

pub(crate) fn clear_thunks() {
    APPLY.with(|c| *c.borrow_mut() = None);
    MICRO.with(|c| *c.borrow_mut() = None);
    EMPTY_ARGS.with(|c| *c.borrow_mut() = None);
}

fn push_timer(q: &mut TimerState, due_us: u64, id: u32, interval_us: u64, tail: TimerTail) {
    if q.live >= TIMER_CAP {
        return;
    }
    if q.heap.len() >= TIMER_CAP {
        let tails = &q.tails;
        q.heap.retain(|k| tails.contains_key(&k.id));
    }
    q.seq = q.seq.wrapping_add(1);
    q.live += 1;
    q.tails.insert(id, tail);
    q.heap.push(TimerKey { due_us, seq: q.seq, id, interval_us });
}

pub(crate) fn schedule<'js>(
    ctx: &Ctx<'js>,
    cb: Value<'js>,
    delay_ms: f64,
    args: Option<Value<'js>>,
    interval: bool,
) -> u32 {
    let id = TIMERQ.with(|q| {
        let mut q = q.borrow_mut();
        let id = q.next_id;
        q.next_id = bump_u32_id(q.next_id);
        id
    });
    let delay_us = if delay_ms.is_finite() && delay_ms > 0.0 {
        (delay_ms * 1000.0).min(3_600_000.0) as u64
    } else {
        1
    };
    let due = clock::now_us().saturating_add(delay_us);
    let tail = TimerTail {
        cb: Persistent::save(ctx, cb),
        args: args.map(|a| Persistent::save(ctx, a)),
    };
    TIMERQ.with(|q| {
        let mut q = q.borrow_mut();
        push_timer(&mut q, due, id, if interval { delay_us } else { 0 }, tail);
    });
    id
}

fn valid_timer_id(id: f64) -> Option<u32> {
    if id.is_finite() && id > 0.0 && id <= u32::MAX as f64 {
        Some(id as u32)
    } else {
        None
    }
}

pub(crate) fn unschedule(id: f64) {
    if let Some(id) = valid_timer_id(id) {
        TIMERQ.with(|q| {
            let mut q = q.borrow_mut();
            if q.tails.remove(&id).is_some() {
                q.live -= 1;
            }
        });
    }
}

pub(crate) fn raf_schedule<'js>(ctx: &Ctx<'js>, cb: Value<'js>) -> u32 {
    let id = RAFQ.with(|r| {
        let mut r = r.borrow_mut();
        let id = r.next_id;
        r.next_id = bump_u32_id(r.next_id);
        if r.frame_us == 0 {
            let (seed, hz) = crate::worker::with_prof(|p| (p.seed, p.display_hz));
            r.seed = seed;
            r.display_hz = hz;
            r.frame_us = clock::now_us().saturating_add(frame_cadence_us(hz, 0));
        }
        id
    });
    RAFQ.with(|r| {
        let mut r = r.borrow_mut();
        if r.cbs.len() < RAF_CAP {
            r.cbs.push((id, Persistent::save(ctx, cb)));
        }
    });
    id
}

pub(crate) fn raf_cancel(id: f64) {
    if let Some(id) = valid_timer_id(id) {
        RAFQ.with(|r| r.borrow_mut().cbs.retain(|(i, _)| *i != id));
    }
}

fn raf_frame_interval_us(cadence: u32, seed: u64, display_hz: u32) -> u64 {
    let base = frame_cadence_us(display_hz, cadence);
    let jitter = base / RAF_JITTER_FRAC + 1;
    base + core_utils::rng::mix_to_range(seed, cadence as u64, jitter)
}

fn drain_microtasks(ctx: &Ctx<'_>, deadline: std::time::Instant) -> usize {
    let mut n = 0usize;
    while ctx.execute_pending_job() {
        n += 1;
        if n >= MICRO_LIMIT || (n & 63 == 0 && std::time::Instant::now() >= deadline) {
            break;
        }
    }
    crate::webidl::flush_mutations(ctx);
    n
}

fn fire_task<'js>(ctx: &Ctx<'js>, task: &TimerTail) -> bool {
    let Ok(cb) = task.cb.clone().restore(ctx) else {
        return false;
    };
    if cb.is_function() {
        let Some(apply) = crate::webidl::restore_slot(ctx, &APPLY) else {
            return false;
        };
        let undefined = Value::new_undefined(ctx.clone());
        let args: Value<'js> = match task.args.as_ref() {
            Some(a) => match a.clone().restore(ctx) {
                Ok(v) => v,
                Err(_) => return false,
            },
            None => match crate::webidl::restore_slot(ctx, &EMPTY_ARGS) {
                Some(a) => a.clone().into_value(),
                None => return false,
            },
        };
        let out: rquickjs::Result<Value<'js>> = apply.call((cb, undefined, args));
        return out.is_ok();
    }
    if let Some(s) = cb.as_string()
        && let Ok(src) = s.clone().to_cstring()
    {
        let out: rquickjs::Result<Value<'js>> = ctx.eval(src.as_str());
        return out.is_ok();
    }
    false
}

fn fire_raf_batch(ctx: &Ctx<'_>) -> bool {
    let now = clock::now_us();
    let due = RAFQ.with(|r| {
        let rb = r.borrow();
        !rb.cbs.is_empty() && now >= rb.frame_us
    });
    if !due {
        return false;
    }
    let ts = clock::now_ms_quantized();
    let mut next_frame_us: u64 = 0;
    let batch: Vec<(u32, Persistent<Value<'static>>)> = RAFQ.with(|r| {
        let mut r = r.borrow_mut();
        next_frame_us = raf_frame_interval_us(r.cadence, r.seed, r.display_hz);
        r.cadence = r.cadence.wrapping_add(1);
        std::mem::take(&mut r.cbs)
    });
    let mut keep: Vec<(u32, Persistent<Value<'static>>)> = Vec::new();
    for (i, p) in batch {
        let Ok(cb) = p.clone().restore(ctx) else {
            continue;
        };
        if cb.is_function()
            && let Ok(f) = Function::from_value(cb)
        {
            let _: rquickjs::Result<Value<'_>> = f.call((ts,));
        } else {
            keep.push((i, p));
        }
    }
    RAFQ.with(|r| {
        let mut r = r.borrow_mut();
        if r.cbs.is_empty() {
            r.cbs = keep;
        } else {
            r.cbs.extend(keep);
        }
        r.frame_us = clock::now_us().saturating_add(next_frame_us);
    });
    true
}

pub(crate) fn run_loop(ctx: &Ctx<'_>, deadline: std::time::Instant) {
    let mut fired = 0usize;
    loop {
        if fired >= FIRE_LIMIT || std::time::Instant::now() >= deadline {
            break;
        }
        drain_microtasks(ctx, deadline);
        let now = clock::now_us();
        if fire_raf_batch(ctx) {
            fired += 1;
            continue;
        }
        let mut next_task: Option<(TimerKey, TimerTail)> = None;
        TIMERQ.with(|q| {
            let mut q = q.borrow_mut();
            while let Some(top) = q.heap.peek() {
                if top.due_us > now {
                    break;
                }
                let key = q.heap.pop().expect("peeked");
                match q.tails.remove(&key.id) {
                    Some(tail) => {
                        q.live -= 1;
                        next_task = Some((key, tail));
                        break;
                    }
                    None => continue,
                }
            }
        });
        let Some((key, task)) = next_task else {
            let t = TIMERQ.with(|q| q.borrow().live > 0);
            let r = RAFQ.with(|q| !q.borrow().cbs.is_empty());
            if !t && !r {
                break;
            }
            let next_due = TIMERQ
                .with(|q| q.borrow().heap.peek().map(|x| x.due_us))
                .unwrap_or(u64::MAX);
            let next_frame = RAFQ.with(|q| {
                let rb = q.borrow();
                if rb.cbs.is_empty() {
                    u64::MAX
                } else {
                    rb.frame_us
                }
            });
            let wait = next_due.min(next_frame).saturating_sub(clock::now_us());
            if wait > 0 {
                let now_std = std::time::Instant::now();
                let wake_at = now_std + std::time::Duration::from_micros(wait);
                if wake_at >= deadline {
                    break;
                }
                std::thread::sleep(deadline.min(wake_at) - now_std);
            }
            continue;
        };
        let alive = fire_task(ctx, &task);
        fired += 1;
        if key.interval_us != 0 && alive {
            let due = clock::now_us().saturating_add(key.interval_us);
            TIMERQ.with(|q| {
                let mut q = q.borrow_mut();
                push_timer(&mut q, due, key.id, key.interval_us, task);
            });
        }
    }
    drain_microtasks(ctx, deadline);
}
