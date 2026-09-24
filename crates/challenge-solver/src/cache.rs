use crate::PaddedAtomicUsize;
use core_utils::xxh3;
use std::sync::atomic::{AtomicUsize, Ordering};

static EVICT_ROTATE: PaddedAtomicUsize = PaddedAtomicUsize(AtomicUsize::new(0));


struct CacheEntry {
    x: u32,
    y: u32,
    hits: u32,
}

pub struct AnswerCache {
    map: scc::HashMap<u64, CacheEntry>,
    cap: usize,
}

impl Default for AnswerCache {
    fn default() -> Self {
        AnswerCache::new(4096)
    }
}

impl AnswerCache {
    pub fn new(cap: usize) -> Self {
        AnswerCache {
            map: scc::HashMap::new(),
            cap: cap.max(64),
        }
    }

    pub fn lookup(&self, key: u64) -> Option<(u32, u32)> {
        self.map.read_sync(&key, |_, e| (e.x, e.y))
    }

    pub fn record(&self, key: u64, x: u32, y: u32) {
        if self.map.update_sync(&key, |_, e| e.hits += 1).is_some() {
            return;
        }
        if self.map.len() >= self.cap {
            self.evict();
        }
        let _ = self.map.insert_sync(key, CacheEntry { x, y, hits: 1 });
    }

    fn evict(&self) {
        let skip = EVICT_ROTATE.0.fetch_add(1, Ordering::Relaxed) % self.cap.max(1);
        let mut skipped = 0usize;
        let mut victims: smallvec::SmallVec<[(u64, u32); 64]> = smallvec::SmallVec::new();
        self.map.iter_sync(|k, v| {
            if skipped < skip {
                skipped += 1;
                return true;
            }
            if victims.len() < 64 {
                victims.push((*k, v.hits));
            }
            true
        });
        if victims.is_empty() {
            self.map.iter_sync(|k, v| {
                if victims.len() < 64 {
                    victims.push((*k, v.hits));
                }
                victims.len() < 64
            });
        }
        victims.sort_unstable_by_key(|&(_, h)| h);
        for (k, _) in victims.iter().take(victims.len().div_ceil(2)) {
            let _ = self.map.remove_sync(k);
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
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
