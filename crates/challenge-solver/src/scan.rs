use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread;

use crate::{CORE_CURSOR, Align64, Lane, PaddedAtomicU64, PaddedAtomicUsize};
use core_utils::BytesExt as _;
use core_utils::pin_thread;

use core_utils::crypto::{compress8, sha256_block, words_be32};

pub(crate) const CHUNK_CAP: u64 = 1 << 20;

#[inline]
pub(crate) fn need_bits_of(difficulty: u8) -> u32 {
    difficulty as u32 * 4
}

pub(crate) fn cpu_avx512_enabled() -> bool {
    static V: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        !core_utils::env_present("SILO_POW_NO_AVX512")
            && core_utils::cpu_avx512cd()
    });
    *V
}

pub(crate) const MAX_WIDTH: usize = 19;

pub(crate) use core_utils::math::u64_digits_fixed_into as digits_of;

#[inline(always)]
pub(crate) fn incr_dec(buf: &mut [u8; 20], width: usize, delta: u8) {
    let mut i = width;
    let mut carry = delta;
    while i > 0 && carry > 0 {
        i -= 1;
        let cur = (buf[i] - b'0') + carry;
        let over = cur >= 10;
        buf[i] = b'0' + cur - 10 * u8::from(over);
        carry = u8::from(over);
    }
}

pub(crate) fn chunk_and_jobs(
    base: u64,
    limit: u64,
    threads: usize,
    jobs_mult: u64,
    min_chunk: u64,
    cap: u64,
) -> (u64, u64) {
    let range = limit - base;
    let target_jobs = (threads as u64) * jobs_mult;
    let mut chunk = range.div_ceil(target_jobs);
    if chunk < min_chunk {
        chunk = min_chunk;
    }
    chunk = (chunk + 7) & !7;
    if chunk > cap {
        chunk = cap;
    }
    let n_jobs = range.div_ceil(chunk);
    (chunk, n_jobs.max(1))
}

const POW10: [u64; 20] = {
    let mut t = [1u64; 20];
    let mut i = 1;
    while i < 20 {
        t[i] = t[i - 1] * 10;
        i += 1;
    }
    t
};

#[inline]
pub(crate) fn width_range(width: usize) -> (u64, u64) {
    let base = if width == 1 { 0 } else { POW10[width - 1] };
    let limit = POW10[width];
    (base, limit)
}

pub(crate) struct ScanPlan {
    pub(crate) width: usize,
    pub(crate) block1: [u8; 64],
    pub(crate) block2: [u8; 64],
    pub(crate) single: bool,
}

impl ScanPlan {
    #[inline]
    pub(crate) fn build(width: usize, tail: &[u8], total: usize) -> Self {
        let (block1, block2, single) = frame_blocks(tail, width, total);
        Self {
            width,
            block1,
            block2,
            single,
        }
    }
}

pub(crate) fn scalar_scan<D>(
    mut n: u64,
    end: u64,
    need_bits: u32,
    mut digest: D,
) -> Option<(u64, [u8; 32])>
where
    D: FnMut(u64) -> [u8; 32],
{
    let head_only = need_bits <= 32;
    while n < end {
        let h = digest(n);
        let hit = if head_only {
            h.be_u32(0).leading_zeros() >= need_bits
        } else {
            let s = core_utils::be32_words(&h);
            core_utils::lz_words_be(&s) >= need_bits
        };
        if hit {
            return Some((n, h));
        }
        n += 1;
    }
    None
}

#[inline]
pub(crate) fn pad64(block: &mut [u8; 64], at: usize, total: usize) {
    debug_assert!(at < 56);
    core_utils::sha_tail_pad(block, at, (total as u64) * 8);
}

pub(crate) struct TailCtx<'a> {
    pub prefix: [u32; 8],
    pub tail: &'a [u8],
    pub total: usize,
}

pub(crate) fn tail_digest(ctx: TailCtx<'_>, nonce: u64, width: usize) -> [u8; 32] {
    let TailCtx { prefix, tail, total } = ctx;
    let (mut b1, mut b2, single) = frame_blocks(tail, width, total);
    let mut digits = [0u8; 20];
    digits_of(nonce, &mut digits, width);
    patch_digit_at(&mut b1, &mut b2, tail.len(), width, &digits);
    let mut st = prefix;
    sha256_block(&mut st, &b1);
    if !single {
        sha256_block(&mut st, &b2);
    }
    words_be32(&st)
}

#[inline(always)]
pub(crate) fn check8(st: &[[u32; 8]; 8], need_bits: u32, avx512: bool) -> Option<usize> {
    if need_bits <= 32 {
        let mut hits = 0u32;
        for (k, s) in st.iter().enumerate() {
            hits |= u32::from(s[0].leading_zeros() >= need_bits) << k;
        }
        if hits == 0 {
            return None;
        }
        return Some(hits.trailing_zeros() as usize);
    }
    if avx512 {
        for pair in 0..4 {
            let k0 = pair * 2;
            let k1 = pair * 2 + 1;
            let r = unsafe { core_utils::lz_batch16_ref(&st[k0], &st[k1]) };
            if r[0] >= need_bits {
                return Some(k0);
            }
            if r[1] >= need_bits {
                return Some(k1);
            }
        }
        return None;
    }
    for (k, s) in st.iter().enumerate() {
        if core_utils::lz_words_be(s) >= need_bits {
            return Some(k);
        }
    }
    None
}

fn scalar_scan_plan<C: BatchCtx>(
    ctx: &C,
    plan: &ScanPlan,
    base: u64,
    end: u64,
) -> Option<(u64, [u8; 32])> {
    let off = ctx.batch_off();
    let mut digits = [0u8; 20];
    let mut b1 = plan.block1;
    let mut b2 = plan.block2;
    scalar_scan(base, end, ctx.need_bits(), |n| {
        digits_of(n, &mut digits, plan.width);
        patch_digit_at(&mut b1, &mut b2, off, plan.width, &digits);
        let mut st = ctx.batch_prefix();
        sha256_block(&mut st, &b1);
        if !plan.single {
            sha256_block(&mut st, &b2);
        }
        ctx.mid_one(&st)
    })
}

pub(crate) trait BatchCtx: Sync {
    fn batch_off(&self) -> usize;
    fn batch_prefix(&self) -> [u32; 8];
    fn need_bits(&self) -> u32;
    fn avx512(&self) -> bool;
    fn plan_of(&self, width: usize) -> ScanPlan;

    fn mid_one(&self, st: &[u32; 8]) -> [u8; 32] {
        words_be32(st)
    }
    unsafe fn init_blocks(&self, _blocks: &mut [[u8; 64]; 8]) {}
    unsafe fn mid_step(&self, st: &mut [[u32; 8]; 8], blocks: &mut [[u8; 64]; 8]);
}

pub(crate) unsafe fn batch8<C: BatchCtx>(
    ctx: &C,
    plan: &ScanPlan,
    base: u64,
    end: u64,
) -> Option<(u64, [u8; 32])> {
    unsafe {
        let off = ctx.batch_off();
        if off + plan.width > 64 {
            return scalar_scan_plan(ctx, plan, base, end);
        }
        let mut nd = NonceDigits::new(base, plan.width);
        let mut b1 = Align64([plan.block1; 8]);
        let b2 = Align64([plan.block2; 8]);
        let mut blocks = Align64([[0u8; 64]; 8]);
        ctx.init_blocks(&mut blocks.0);
        let need = ctx.need_bits();
        let avx = ctx.avx512();
        let single = plan.single;
        let prefix = ctx.batch_prefix();
        let mut st = Lane::<8>::splat(prefix);
        let mut cur = base;
        while cur + 8 <= end {
            nd.patch(&mut b1.0, off, plan.width);
            st.0 = [prefix; 8];
            compress8(&mut st.0, &b1.0);
            if !single {
                compress8(&mut st.0, &b2.0);
            }
            ctx.mid_step(&mut st.0, &mut blocks.0);
            if let Some(k) = check8(&st.0, need, avx) {
                return Some((cur + k as u64, words_be32(&st.0[k])));
            }
            nd.bump(plan.width);
            cur += 8;
        }
        scalar_scan_plan(ctx, plan, cur, end)
    }
}

pub(crate) fn batched_solve<C: BatchCtx + Send + Sync + 'static>(
    ctx: C,
    threads: usize,
) -> Option<(u64, [u8; 32])> {
    batched_solve_until(ctx, threads, &AtomicBool::new(false))
}

pub(crate) fn batched_solve_until<C: BatchCtx + Send + Sync + 'static>(
    ctx: C,
    threads: usize,
    abort: &AtomicBool,
) -> Option<(u64, [u8; 32])> {
    let ctx = std::sync::Arc::new(ctx);
    width_solve_until(
        threads,
        MAX_WIDTH,
        16,
        8,
        CHUNK_CAP,
        move |width, base, end| {
            let plan = ctx.plan_of(width);
            unsafe { batch8(ctx.as_ref(), &plan, base, end) }
        },
        abort,
    )
}



static SCAN_POOL: std::sync::LazyLock<ScanPool> = std::sync::LazyLock::new(|| {
    let ncores = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .max(1);
    let n = (*NCPUS).min(ncores);
    let (tx, rx) = crossbeam_channel::bounded::<Box<dyn FnOnce() + Send>>(n * 4);
    let core0 = CORE_CURSOR.0.fetch_add(n, Ordering::Relaxed) % ncores;
    for t in 0..n {
        let rx = rx.clone();
        thread::Builder::new()
            .name(format!("silo-pow-{t}"))
            .spawn(move || {
                pin_thread((core0 + t) % ncores);
                for f in rx.iter() {
                    f();
                }
            })
            .expect("scan pool spawn");
    }
    ScanPool { tx }
});

static NCPUS: std::sync::LazyLock<usize> = std::sync::LazyLock::new(|| {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .max(1)
});

struct ScanPool {
    tx: crossbeam_channel::Sender<Box<dyn FnOnce() + Send>>,
}

pub(crate) fn width_solve<Job>(
    threads: usize,
    max_width: usize,
    jobs_mult: u64,
    min_chunk: u64,
    cap: u64,
    job: Job,
) -> Option<(u64, [u8; 32])>
where
    Job: Fn(usize, u64, u64) -> Option<(u64, [u8; 32])> + Send + Sync + 'static,
{
    width_solve_until(
        threads,
        max_width,
        jobs_mult,
        min_chunk,
        cap,
        job,
        &AtomicBool::new(false),
    )
}

pub(crate) fn width_solve_until<Job>(
    threads: usize,
    max_width: usize,
    jobs_mult: u64,
    min_chunk: u64,
    cap: u64,
    job: Job,
    abort: &AtomicBool,
) -> Option<(u64, [u8; 32])>
where
    Job: Fn(usize, u64, u64) -> Option<(u64, [u8; 32])> + Send + Sync + 'static,
{
    let threads = threads.max(1);
    let pool = &*SCAN_POOL;
    let workers = (*NCPUS).min(threads);
    let found = Arc::new(PaddedAtomicU64(AtomicU64::new(u64::MAX)));
    let (jtx, jrx) = crossbeam_channel::bounded::<(usize, u64, u64)>(workers.max(1) * 4);
    let (rtx, rrx) = crossbeam_channel::unbounded::<(u64, [u8; 32])>();
    let done = Arc::new(PaddedAtomicU64(AtomicU64::new(0)));
    let job = Arc::new(job);
    let stop = Arc::new(AtomicBool::new(abort.load(Ordering::Acquire)));
    let mut spawned: u64 = 0;
    for _ in 0..workers {
        let jrx = jrx.clone();
        let rtx = rtx.clone();
        let found = Arc::clone(&found);
        let done = Arc::clone(&done);
        let job = Arc::clone(&job);
        let stop = Arc::clone(&stop);
        let task = move || {
            for (width, base, end) in jrx.iter() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let cur = found.0.load(Ordering::Acquire);
                if cur != u64::MAX && base > cur {
                    continue;
                }
                if let Some(r) = job(width, base, end)
                    && r.0 < found.0.load(Ordering::Acquire)
                {
                    found.0.fetch_min(r.0, Ordering::AcqRel);
                    let _ = rtx.send(r);
                }
            }
            done.0.fetch_add(1, Ordering::AcqRel);
        };
        if pool.tx.send(Box::new(task)).is_err() {
            break;
        }
        spawned += 1;
    }

    'widths: for width in 1..=max_width {
        if abort.load(Ordering::Relaxed) {
            stop.store(true, Ordering::Release);
            break 'widths;
        }
        let (base, limit) = width_range(width);
        let (chunk, n_jobs) = chunk_and_jobs(base, limit, threads, jobs_mult, min_chunk, cap);
        for j in 0..n_jobs {
            if found.0.load(Ordering::Acquire) != u64::MAX
                || abort.load(Ordering::Relaxed)
            {
                stop.store(true, Ordering::Release);
                break 'widths;
            }
            let b = base + j * chunk;
            let end = (b + chunk).min(limit);
            if jtx.send((width, b, end)).is_err() {
                break 'widths;
            }
        }
    }
    drop(jtx);

    let mut spin = 0u32;
    while done.0.load(Ordering::Acquire) < spawned {
        if abort.load(Ordering::Relaxed) {
            stop.store(true, Ordering::Release);
            break;
        }
        if spin & 0x3FF == 0 && crate::watch_fired() {
            stop.store(true, Ordering::Release);
            abort.store(true, Ordering::Release);
            break;
        }
        spin = spin.wrapping_add(1);
        std::hint::spin_loop();
    }
    let mut best: Option<(u64, [u8; 32])> = None;
    for r in rrx.try_iter() {
        if best.as_ref().is_none_or(|b| r.0 < b.0) {
            best = Some(r);
        }
    }
    best
}

pub(crate) struct NonceDigits {
    d: [[u8; 20]; 8],
}

impl NonceDigits {
    #[inline(always)]
    pub fn new(base: u64, width: usize) -> Self {
        let mut d = [[0u8; 20]; 8];
        for (k, dk) in d.iter_mut().enumerate() {
            digits_of(base + k as u64, dk, width);
        }
        Self { d }
    }

    #[inline(always)]
    pub fn patch(&self, blocks: &mut [[u8; 64]; 8], off: usize, width: usize) {
        for (k, bk) in blocks.iter_mut().enumerate() {
            bk[off..off + width].copy_from_slice(&self.d[k][..width]);
        }
    }

    #[inline(always)]
    pub fn bump(&mut self, width: usize) {
        for dk in self.d.iter_mut() {
            incr_dec(dk, width, 8);
        }
    }
}

pub(crate) fn frame_blocks(tail: &[u8], width: usize, total: usize) -> ([u8; 64], [u8; 64], bool) {
    let mut buf = [0u8; 128];
    buf[..tail.len()].copy_from_slice(tail);
    let pos = tail.len() + width;
    buf[tail.len()..pos].fill(b'0');
    let single = pos <= 55;
    core_utils::sha_tail_pad(&mut buf, pos, (total as u64) * 8);
    let [b1, b2]: [[u8; 64]; 2] = unsafe { std::mem::transmute(buf) };
    (b1, b2, single)
}

#[inline(always)]
pub(crate) fn patch_digit_at(
    b1: &mut [u8; 64],
    b2: &mut [u8; 64],
    off: usize,
    width: usize,
    digits: &[u8; 20],
) {
    if off + width <= 64 {
        b1[off..off + width].copy_from_slice(&digits[..width]);
    } else {
        for (w, &dw) in digits[..width].iter().enumerate() {
            let p = off + w;
            if p < 64 {
                b1[p] = dw;
            } else {
                b2[p - 64] = dw;
            }
        }
    }
}
