pub mod anubis;
pub mod argon;
pub mod cache;
pub mod chain;
pub mod decode;
pub mod hmac;
pub mod pow;
mod scan;
pub mod slider;

use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[repr(align(64))]
pub(crate) struct PaddedAtomicUsize(pub(crate) AtomicUsize);

#[repr(align(64))]
pub(crate) struct PaddedAtomicU64(pub(crate) std::sync::atomic::AtomicU64);

static CORE_CURSOR: PaddedAtomicUsize = PaddedAtomicUsize(AtomicUsize::new(0));


#[repr(C, align(64))]
pub(crate) struct Align64<T>(pub(crate) T);

#[repr(C, align(64))]
pub(crate) struct Lane<const N: usize>(pub(crate) [[u32; 8]; N]);

impl<const N: usize> Lane<N> {
    #[inline(always)]
    pub(crate) fn splat(v: [u32; 8]) -> Self {
        Lane([v; N])
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Algorithm {
    Sha256,
    HmacSha256,
    Argon2,
    HashChain,
}

impl Algorithm {
    pub fn parse(s: &[u8]) -> Option<Self> {
        match s {
            b"fast" | b"sha256" | b"sha-256" => Some(Algorithm::Sha256),
            b"hmac-sha256" | b"hmac" => Some(Algorithm::HmacSha256),
            b"argon2" | b"argon2id" => Some(Algorithm::Argon2),
            b"hash-chain" | b"chain" => Some(Algorithm::HashChain),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Algorithm::Sha256 => "fast",
            Algorithm::HmacSha256 => "hmac-sha256",
            Algorithm::Argon2 => "argon2id",
            Algorithm::HashChain => "hash-chain",
        }
    }
}

pub struct Challenge {
    pub algorithm: Algorithm,
    pub salt: SmallVec<[u8; 192]>,
    pub key: SmallVec<[u8; 64]>,
    pub difficulty: u8,
    pub rounds: u32,
    pub mem_cost: u32,
    pub time_cost: u32,
    pub threads: usize,
}

impl Challenge {
    pub const MAX_MEM_COST: u32 = 64 * 1024;

    pub fn validate(&self) -> Result<(), ChallengeError> {
        if self.mem_cost > Self::MAX_MEM_COST {
            return Err(ChallengeError::MemCostExceeded);
        }
        if self.time_cost == 0 || self.time_cost > 64 {
            return Err(ChallengeError::TimeCost);
        }
        if self.threads == 0 || self.threads > 64 {
            return Err(ChallengeError::Threads);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeError {
    MemCostExceeded,
    TimeCost,
    Threads,
}

pub struct Solution {
    pub nonce: u64,
    pub digest: [u8; 32],
}

pub fn solve(ch: &Challenge) -> Option<Solution> {
    ch.validate().ok()?;
    let r = match ch.algorithm {
        Algorithm::Sha256 => pow::solve(&ch.salt, ch.difficulty, ch.threads),
        Algorithm::HmacSha256 => hmac::solve(&ch.key, &ch.salt, ch.difficulty, ch.threads),
        Algorithm::Argon2 => argon::solve(
            &ch.salt,
            ch.difficulty,
            ch.mem_cost,
            ch.time_cost,
            ch.threads,
        ),
        Algorithm::HashChain => chain::solve(&ch.salt, ch.rounds, ch.difficulty, ch.threads),
    };
    r.map(|(nonce, digest)| Solution { nonce, digest })
}

pub fn solve_until(ch: &Challenge, deadline: std::time::Instant) -> Option<Solution> {
    ch.validate().ok()?;
    let abort = Arc::new(AtomicBool::new(false));
    let guard = DeadlineWatch::arm(deadline, Arc::clone(&abort));
    let r = match ch.algorithm {
        Algorithm::Sha256 => pow::solve_until(&ch.salt, ch.difficulty, ch.threads, &abort),
        Algorithm::HmacSha256 => {
            hmac::solve_until(&ch.key, &ch.salt, ch.difficulty, ch.threads, &abort)
        }
        Algorithm::Argon2 => argon::solve_until(
            &ch.salt,
            ch.difficulty,
            ch.mem_cost,
            ch.time_cost,
            ch.threads,
            &abort,
        ),
        Algorithm::HashChain => chain::solve_until(
            &ch.salt,
            ch.rounds,
            ch.difficulty,
            ch.threads,
            &abort,
        ),
    };
    drop(guard);
    let timed_out = abort.load(Ordering::Acquire) || WATCH.abort.load(Ordering::Acquire);
    if timed_out {
        return None;
    }
    r.map(|(nonce, digest)| Solution { nonce, digest })
}
#[repr(align(64))]
struct WatchArm {
    abort: AtomicBool,
    active: AtomicBool,
    deadline: parking_lot::Mutex<std::time::Instant>,
}

static WATCH: WatchArm = WatchArm {
    abort: AtomicBool::new(false),
    active: AtomicBool::new(false),
    deadline: parking_lot::Mutex::new(std::time::Instant::now()),
};

impl WatchArm {
    fn check(&self) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        if std::time::Instant::now() >= *self.deadline.lock() {
            self.abort.store(true, Ordering::Release);
            self.active.store(false, Ordering::Release);
        }
    }
}

pub(crate) fn watch_fired() -> bool {
    WATCH.check();
    WATCH.abort.load(Ordering::Acquire)
}
struct DeadlineWatch;

impl DeadlineWatch {
    fn arm(deadline: std::time::Instant, _abort: Arc<AtomicBool>) -> Self {
        *WATCH.deadline.lock() = deadline;
        WATCH.abort.store(false, Ordering::Release);
        WATCH.active.store(true, Ordering::Release);
        DeadlineWatch
    }
}

impl Drop for DeadlineWatch {
    fn drop(&mut self) {
        WATCH.active.store(false, Ordering::Release);
    }
}
