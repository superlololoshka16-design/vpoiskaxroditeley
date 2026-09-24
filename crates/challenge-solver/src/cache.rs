use core_utils::xxh3;
use std::sync::atomic::{AtomicU64, Ordering};

#[repr(C, align(64))]
struct RingSlot {
    key: AtomicU64,
    x: u32,
    y: u32,
    _pad: [u32; 10],
}

pub struct AnswerCache {
    slots: Vec<RingSlot>,
    mask: usize,
    len: AtomicU64,
}

impl Default for AnswerCache {
    fn default() -> Self {
        AnswerCache::new(4096)
    }
}

impl AnswerCache {
    pub fn new(cap: usize) -> Self {
        let pow = cap.max(64).next_power_of_two();
        let mut slots = Vec::with_capacity(pow);
        for _ in 0..pow {
            slots.push(RingSlot {
                key: AtomicU64::new(0),
                x: 0,
                y: 0,
                _pad: [0; 10],
            });
        }
        AnswerCache {
            slots,
            mask: pow - 1,
            len: AtomicU64::new(0),
        }
    }

    pub fn lookup(&self, key: u64) -> Option<(u32, u32)> {
        let slot = &self.slots[key as usize & self.mask];
        if slot.key.load(Ordering::Acquire) != key {
            return None;
        }
        Some((slot.x, slot.y))
    }

    pub fn record(&self, key: u64, x: u32, y: u32) {
        let slot = &self.slots[key as usize & self.mask];
        let old_key = slot.key.load(Ordering::Acquire);
        if old_key == key {
            return;
        }
        if old_key == 0 {
            self.len.fetch_add(1, Ordering::Relaxed);
        }
        slot.x = x;
        slot.y = y;
        slot.key.store(key, Ordering::Release);
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Acquire) as usize
    }
}

pub fn key_of(payload: &[u8]) -> u64 {
    xxh3::hash(payload)
}

pub struct SliderOutcome {
    pub x: u32,
    pub y: u32,
    pub kind: OutcomeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    Computed { ssd: u64 },
    CacheHit,
}

pub fn solve_with_cache(
    cache: &AnswerCache,
    ex: &crate::slider::SliderExchange<'_>,
    payload: &[u8],
) -> Result<SliderOutcome, crate::slider::SliderError> {
    let key = key_of(payload);
    if let Some((x, y)) = cache.lookup(key) {
        return Ok(SliderOutcome {
            x,
            y,
            kind: OutcomeKind::CacheHit,
        });
    }
    let hit = crate::slider::solve(ex)?;
    cache.record(key, hit.x, hit.y);
    Ok(SliderOutcome {
        x: hit.x,
        y: hit.y,
        kind: OutcomeKind::Computed { ssd: hit.ssd },
    })
}
