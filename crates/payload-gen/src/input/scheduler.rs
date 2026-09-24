use crate::input::event::ClickCursor;
use crate::input::event::RawEvent;
use crate::input::event::ScrollCursor;
use crate::input::motion::MotionCursor;
use crate::input::session::TabSession;
use crate::input::typing::TypingCursor;
use smallvec::SmallVec;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

pub type TabEvents = SmallVec<[(SlotId, RawEvent); 32]>;

#[repr(C, align(64))]
pub struct Calibration {
    strictness: u8,
    _pad0: [u8; 7],
    trust: AtomicI32,
    passes: AtomicU32,
    fails: AtomicU32,
    _pad1: [u64; 5],
}

impl Calibration {
    pub fn new(strictness: u8) -> Self {
        Self {
            strictness,
            _pad0: [0; 7],
            trust: AtomicI32::new(0),
            passes: AtomicU32::new(0),
            fails: AtomicU32::new(0),
            _pad1: [0; 5],
        }
    }

    pub fn record(&self, passed: bool) {
        let w = 1 + self.strictness as i32;
        if passed {
            self.passes.fetch_add(1, Ordering::Relaxed);
            self.trust.fetch_add(w, Ordering::AcqRel);
        } else {
            self.fails.fetch_add(1, Ordering::Relaxed);
            self.trust.fetch_sub(w, Ordering::AcqRel);
        }
    }

    #[inline]
    pub fn trust(&self) -> i32 {
        self.trust.load(Ordering::Acquire)
    }

    #[inline]
    pub fn stats(&self) -> (u32, u32) {
        (
            self.passes.load(Ordering::Relaxed),
            self.fails.load(Ordering::Relaxed),
        )
    }
}

pub enum TabInput {
    Move(MotionCursor),
    Scroll(ScrollCursor),
    Type(TypingCursor),
    Click(ClickCursor),
    Session(TabSession),
}

impl TabInput {
    #[inline]
    pub fn next_due_us(&self) -> u64 {
        match self {
            TabInput::Move(c) => c.next_due_us(),
            TabInput::Scroll(c) => c.next_due_us(),
            TabInput::Type(c) => c.next_due_us(),
            TabInput::Click(c) => c.next_due_us(),
            TabInput::Session(s) => s.next_due_us(),
        }
    }

    #[inline]
    pub fn done(&self) -> bool {
        match self {
            TabInput::Move(c) => c.done(),
            TabInput::Scroll(c) => c.done(),
            TabInput::Type(c) => c.done(),
            TabInput::Click(c) => c.done(),
            TabInput::Session(s) => s.finished(),
        }
    }

    fn step_into(&mut self, now_us: u64, tab: SlotId, out: &mut TabEvents) {
        #[inline]
        fn emit(c: &mut dyn crate::input::InputCursor, now_us: u64, tab: SlotId, out: &mut TabEvents) {
            if let Some(ev) = c.step(now_us) {
                out.push((tab, ev));
            }
        }
        match self {
            TabInput::Session(s) => {
                let tick = s.advance(now_us);
                out.extend(tick.events.into_iter().map(|ev| (tab, ev)));
            }
            TabInput::Move(c) => emit(c, now_us, tab, out),
            TabInput::Scroll(c) => emit(c, now_us, tab, out),
            TabInput::Type(c) => emit(c, now_us, tab, out),
            TabInput::Click(c) => emit(c, now_us, tab, out),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotId(pub u32);

struct TabSlot {
    site: u32,
    weight: u32,
    input_gen: u32,
    cached_rank: u32,
    input: Option<TabInput>,
}

pub struct InputHub {
    tabs: Vec<TabSlot>,
    free: Vec<u32>,
    heap: BinaryHeap<Reverse<(u64, u32, u32, u32)>>,
    sites: Vec<Option<Arc<Calibration>>>,
    free_sites: Vec<u32>,
}

impl InputHub {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            free: Vec::new(),
            heap: BinaryHeap::new(),
            sites: Vec::new(),
            free_sites: Vec::new(),
        }
    }

    pub fn register_site(&mut self, strictness: u8) -> u32 {
        if let Some(site) = self.free_sites.pop() {
            self.sites[site as usize] = Some(Arc::new(Calibration::new(strictness)));
            return site;
        }
        self.sites
            .push(Some(Arc::new(Calibration::new(strictness))));
        self.sites.len() as u32 - 1
    }

    pub fn register_site_shared(&mut self, site: u32, calib: Arc<Calibration>) {
        let i = site as usize;
        while self.sites.len() <= i {
            self.sites.push(None);
        }
        self.sites[i] = Some(calib);
    }

    pub fn site_free_slot(&mut self) -> u32 {
        self.free_sites.pop().unwrap_or(self.sites.len() as u32)
    }

    pub fn unregister_site(&mut self, site: u32) -> bool {
        if self.tabs.iter().any(|tab| tab.site == site) {
            return false;
        }
        let Some(slot) = self.sites.get_mut(site as usize) else {
            return false;
        };
        if slot.take().is_none() {
            return false;
        }
        self.free_sites.push(site);
        true
    }

    pub fn calibration(&self, site: u32) -> Option<&Arc<Calibration>> {
        self.sites.get(site as usize).and_then(Option::as_ref)
    }

    pub fn open_tab(&mut self, site: u32, weight: u32) -> SlotId {
        let slot = TabSlot {
            site,
            weight,
            input_gen: 0,
            cached_rank: u32::MAX - weight.min(u32::MAX - 1),
            input: None,
        };
        let id = if let Some(idx) = self.free.pop() {
            self.tabs[idx as usize] = slot;
            idx
        } else {
            self.tabs.push(slot);
            (self.tabs.len() - 1) as u32
        };
        SlotId(id)
    }

    pub fn close_tab(&mut self, tab: SlotId) {
        let i = tab.0 as usize;
        if i < self.tabs.len() && self.tabs[i].site != u32::MAX {
            self.tabs[i].site = u32::MAX;
            self.tabs[i].input_gen = self.tabs[i].input_gen.wrapping_add(1);
            self.tabs[i].input = None;
            self.heap
                .retain(|Reverse((_, _, index, _))| *index != tab.0);
            self.free.push(tab.0);
        }
    }

    pub fn set_input(&mut self, tab: SlotId, input: TabInput) {
        let i = tab.0 as usize;
        if i >= self.tabs.len() || self.tabs[i].site == u32::MAX {
            return;
        }
        let due = input.next_due_us();
        let rank = self.refresh_rank(i);
        let input_gen = self.tabs[i].input_gen.wrapping_add(1);
        self.tabs[i].input_gen = input_gen;
        self.tabs[i].input = Some(input);
        self.heap.push(Reverse((due, rank, tab.0, input_gen)));
    }

    #[inline]
    fn tab_rank(&self, i: usize) -> u32 {
        self.tabs[i].cached_rank
    }

    #[inline]
    fn refresh_rank(&mut self, i: usize) -> u32 {
        let slot = &self.tabs[i];
        let trust = self
            .sites
            .get(slot.site as usize)
            .and_then(Option::as_ref)
            .map(|c| c.trust())
            .unwrap_or(0);
        let trust_bonus = (trust.max(0) as u32).min(64);
        let rank = u32::MAX - (slot.weight.saturating_add(trust_bonus)).min(u32::MAX - 1);
        self.tabs[i].cached_rank = rank;
        rank
    }

    fn compact(&mut self) {
        self.heap.clear();
        let mut entries: SmallVec<[(u64, u32, u32, u32); 64]> = SmallVec::new();
        for i in 0..self.tabs.len() {
            let Some(slot) = self.tabs.get(i) else {
                continue;
            };
            if slot.site == u32::MAX {
                continue;
            }
            let Some(input) = slot.input.as_ref() else {
                continue;
            };
            let due = input.next_due_us();
            let igen = slot.input_gen;
            let rank = self.refresh_rank(i);
            entries.push((due, rank, i as u32, igen));
        }
        for e in entries {
            self.heap.push(Reverse(e));
        }
    }

    pub fn tick(&mut self, now_us: u64, out: &mut TabEvents) {
        out.clear();
        let mut steps = 0usize;
        while let Some(&Reverse((due, _, tab_idx, input_gen))) = self.heap.peek() {
            if due > now_us || steps == 256 {
                break;
            }
            self.heap.pop();
            let i = tab_idx as usize;
            if i >= self.tabs.len()
                || self.tabs[i].site == u32::MAX
                || self.tabs[i].input_gen != input_gen
            {
                continue;
            }
            let Some(input) = self.tabs[i].input.as_mut() else {
                continue;
            };
            steps += 1;
            input.step_into(now_us, SlotId(tab_idx), out);
            if input.done() {
                self.tabs[i].input = None;
            } else {
                let nd = input.next_due_us();
                let rank = self.tab_rank(i);
                let g = self.tabs[i].input_gen;
                self.heap.push(Reverse((nd, rank, tab_idx, g)));
            }
        }
        let live = self.tabs.iter().filter(|t| t.site != u32::MAX).count();
        if self.heap.len() > live.saturating_mul(4).saturating_add(64) {
            self.compact();
        }
    }

    pub fn live_tabs(&self) -> usize {
        self.tabs.iter().filter(|t| t.site != u32::MAX).count()
    }

    pub fn next_due_us(&self) -> u64 {
        self.heap
            .peek()
            .map(|Reverse((due, _, _, _))| *due)
            .unwrap_or(u64::MAX)
    }
}

impl Default for InputHub {
    fn default() -> Self {
        Self::new()
    }
}
