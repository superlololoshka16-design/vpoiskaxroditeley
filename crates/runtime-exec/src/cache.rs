use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::normalize::Lit;
use core_utils::xxh3;
use scc::HashMap;
use smallvec::SmallVec;

pub(crate) const BYTES_PER_ENTRY: usize = 4096;

struct RawEntry {
    skel: u64,
    args: Arc<SmallVec<[Lit; 16]>>,
    used: AtomicU64,
    _charge: Charge,
}

struct SourceEntry {
    source: Arc<str>,
    used: AtomicU64,
    _charge: Charge,
}

pub struct NormCache {
    raw: HashMap<(u64, u64), RawEntry>,
    src: HashMap<(u64, u64), SourceEntry>,
    raw_budget: Arc<Budget>,
    src_budget: Arc<Budget>,
    epoch: Instant,
}

impl NormCache {
    pub fn with_budget(cap: usize, accounted_bytes: usize) -> Self {
        let raw_bytes = accounted_bytes / 2;
        Self {
            raw: HashMap::new(),
            src: HashMap::new(),
            raw_budget: Budget::new(cap, raw_bytes),
            src_budget: Budget::new(cap, accounted_bytes - raw_bytes),
            epoch: Instant::now(),
        }
    }

    fn now_secs(&self) -> u64 {
        self.epoch.elapsed().as_secs()
    }

    pub fn raw_hash(script: &[u8]) -> u64 {
        xxh3::hash(script)
    }

    pub fn lookup_raw(&self, domain: u64, raw: u64) -> Option<(u64, Arc<SmallVec<[Lit; 16]>>)> {
        let now = self.now_secs();
        self.raw.read_sync(&(domain, raw), |_, entry| {
            entry.used.store(now, Ordering::Relaxed);
            (entry.skel, Arc::clone(&entry.args))
        })
    }

    pub fn put_raw(&self, domain: u64, raw: u64, skel: u64, args: Arc<SmallVec<[Lit; 16]>>) {
        let mut bytes = std::mem::size_of::<RawEntry>()
            .saturating_add(std::mem::size_of::<SmallVec<[Lit; 16]>>());
        if args.spilled() {
            bytes =
                bytes.saturating_add(args.capacity().saturating_mul(std::mem::size_of::<Lit>()));
        }
        for arg in args.iter() {
            if let Lit::Str(s) = arg {
                bytes = bytes.saturating_add(s.len());
            }
        }
        let Some(charge) = self.raw_budget.reserve(1, bytes as u64) else {
            return;
        };
        let _ = self.raw.insert_sync(
            (domain, raw),
            RawEntry {
                skel,
                args,
                used: AtomicU64::new(self.now_secs()),
                _charge: charge,
            },
        );
    }

    pub fn lookup_src(&self, domain: u64, skel: u64) -> Option<Arc<str>> {
        let now = self.now_secs();
        self.src.read_sync(&(domain, skel), |_, entry| {
            entry.used.store(now, Ordering::Relaxed);
            Arc::clone(&entry.source)
        })
    }

    pub fn put_src(&self, domain: u64, skel: u64, source: Arc<str>) {
        let bytes = std::mem::size_of::<SourceEntry>().saturating_add(source.len());
        let Some(charge) = self.src_budget.reserve(1, bytes as u64) else {
            return;
        };
        let _ = self.src.insert_sync(
            (domain, skel),
            SourceEntry {
                source,
                used: AtomicU64::new(self.now_secs()),
                _charge: charge,
            },
        );
    }
}

fn is_fresh(used: &AtomicU64, now: u64, max_age_secs: u64) -> bool {
    now.saturating_sub(used.load(Ordering::Relaxed)) < max_age_secs
}

impl NormCache {
    pub fn sweep(&self, max_age_secs: u64) {
        let now = self.now_secs();
        self.raw
            .retain_sync(|_, e| is_fresh(&e.used, now, max_age_secs));
        self.src
            .retain_sync(|_, e| is_fresh(&e.used, now, max_age_secs));
    }

    pub fn raw_len(&self) -> usize {
        self.raw.len()
    }
    pub fn src_len(&self) -> usize {
        self.src.len()
    }

    pub fn accounted_bytes(&self) -> u64 {
        self.raw_budget
            .used()
            .saturating_add(self.src_budget.used())
    }
}

#[repr(C, align(64))]
pub(crate) struct Budget {
    entry_limit: u64,
    byte_limit: u64,
    _p0: [u64; 6],
    entries: AtomicU64,
    _p1: [u64; 7],
    bytes: AtomicU64,
    _p2: [u64; 7],
}

pub(crate) struct Charge {
    budget: Arc<Budget>,
    units: u64,
    bytes: u64,
}

impl Budget {
    pub(crate) fn new(entry_limit: usize, byte_limit: usize) -> Arc<Self> {
        Arc::new(Self {
            entry_limit: entry_limit.max(1) as u64,
            byte_limit: byte_limit.max(1) as u64,
            _p0: [0; 6],
            entries: AtomicU64::new(0),
            _p1: [0; 7],
            bytes: AtomicU64::new(0),
            _p2: [0; 7],
        })
    }

    pub(crate) fn reserve(self: &Arc<Self>, units: u64, bytes: u64) -> Option<Charge> {
        let mut entries = self.entries.load(Ordering::Relaxed);
        let mut bytes_now = self.bytes.load(Ordering::Relaxed);
        loop {
            let next_entries = entries.checked_add(units)?;
            if next_entries > self.entry_limit {
                return None;
            }
            let next_bytes = bytes_now.checked_add(bytes)?;
            if next_bytes > self.byte_limit {
                return None;
            }
            if self
                .entries
                .compare_exchange_weak(entries, next_entries, Ordering::AcqRel, Ordering::Relaxed)
                .is_err()
            {
                entries = self.entries.load(Ordering::Relaxed);
                bytes_now = self.bytes.load(Ordering::Relaxed);
                continue;
            }
            if self
                .bytes
                .compare_exchange(bytes_now, next_bytes, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return Some(Charge {
                    budget: Arc::clone(self),
                    units,
                    bytes,
                });
            }
            self.entries.fetch_sub(units, Ordering::AcqRel);
            entries = self.entries.load(Ordering::Relaxed);
            bytes_now = self.bytes.load(Ordering::Relaxed);
        }
    }

    pub(crate) fn used(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.entries.fetch_sub(self.units, Ordering::AcqRel);
        self.budget.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
