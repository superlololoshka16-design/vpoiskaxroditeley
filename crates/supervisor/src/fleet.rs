use bytes::Bytes;
use compact_str::CompactString;
use core_utils::{FxBuild, fx_map};
use net_client::{EngineSet, push_telemetry};
use parser_pipeline::TelemetryRoute;
use payload_gen::input::{
    BATCH_CAP, InputHub, RAW_EVENT_LEN, SessionStart, SlotId, TabEvents,
    TabInput, TabSession, TelemetryBatcher, batch_interval_for,
};
use session_state::{CookieJar, Profile, Session};
use smallvec::SmallVec;
use std::collections::{BinaryHeap, HashMap};
use std::cmp::Reverse;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PUSH_CAP: Duration = Duration::from_secs(10);
const PUSH_DONE_CAP: usize = 64;

struct SiteEntry {
    hub_site: u32,
    tabs: usize,
}

pub struct PushJob {
    pub handle: u32,
    pub engine_slot: usize,
    pub route_bundle: Arc<RouteBundle>,
    pub cookie: CompactString,
    pub blob: Bytes,
    pub events: u64,
}

pub struct RouteBundle {
    pub endpoint: CompactString,
    pub route: TelemetryRoute,
}

struct FleetSlot {
    handle: u32,
    site_key: u64,
    session: Session,
    engine_slot: usize,
    site: u32,
    hub_tab: u32,
    route_bundle: Arc<RouteBundle>,
    batcher: TelemetryBatcher,
    scratch: bytes::BytesMut,
    cookie_scratch: session_state::CookieScratch,
    events_total: u64,
    batches: u32,
}

pub struct Fleet {
    hubs: Vec<InputHub>,
    slots: Vec<Option<FleetSlot>>,
    handles: HashMap<u32, u32, FxBuild>,
    tab_to_idx: HashMap<u32, u32, FxBuild>,
    free: Vec<u32>,
    hot: Vec<u32>,
    sites: HashMap<u64, SiteEntry, FxBuild>,
    expiry_heap: BinaryHeap<Reverse<(Instant, u32, u32)>>,
    next_handle: u32,
    live: usize,
    max_tabs: usize,
    session_ttl: Duration,
    engines: Arc<EngineSet>,
}
fn cookies_take(jar: &CookieJar) -> CookieJar {
    jar.clone()
}

impl Fleet {
    pub fn new(engines: Arc<EngineSet>) -> Self {
        Self::with_limits(engines, 1024, Duration::from_secs(300))
    }
    pub fn with_limits(engines: Arc<EngineSet>, max_tabs: usize, session_ttl: Duration) -> Self {
        let n = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(1);
        Self {
            hubs: (0..n).map(|_| InputHub::new()).collect(),
            slots: Vec::new(),
            handles: fx_map(),
            tab_to_idx: fx_map(),
            free: Vec::new(),
            hot: Vec::new(),
            sites: fx_map(),
            expiry_heap: BinaryHeap::new(),
            next_handle: 0,
            live: 0,
            max_tabs,
            session_ttl,
            engines,
        }
    }

    #[inline]
    fn lane(&self, hub_tab: u32) -> usize {
        (hub_tab % self.hubs.len() as u32) as usize
    }
    #[inline]
    fn encode_tab(&self, lane: usize, local: SlotId) -> SlotId {
        SlotId(lane as u32 + local.0 * self.hubs.len() as u32)
    }
    pub fn attach(
        &mut self,
        profile: Arc<Profile>,
        origin: &str,
        jar: &CookieJar,
        route: TelemetryRoute,
        engine_slot: usize,
        weight: u32,
    ) -> u32 {
        self.expire(Instant::now());
        if self.live >= self.max_tabs
            || self.next_handle == u32::MAX
            || self.session_ttl.is_zero()
        {
            return u32::MAX;
        }
        let handle = self.next_handle;
        self.next_handle += 1;
        let key = crate::site_key(origin);
        let site = match self.sites.get_mut(&key) {
            Some(entry) => {
                entry.tabs += 1;
                entry.hub_site
            }
            None => {
                let calib = Arc::new(payload_gen::input::Calibration::new(3));
                let s = self.hubs[0].site_free_slot();
                for hub in self.hubs.iter_mut() {
                    hub.register_site_shared(s, Arc::clone(&calib));
                }
                self.sites.insert(
                    key,
                    SiteEntry {
                        hub_site: s,
                        tabs: 1,
                    },
                );
                s
            }
        };
        let (lane_pick, local_tab) = self.hub_open_tab(site, weight);
        let tab = self.encode_tab(lane_pick, local_tab);
        let (from, target) = payload_gen::placement(&profile, u64::from(tab.0));
        let batch_interval = batch_interval_for(profile.canvas_seed ^ u64::from(tab.0));
        let mut http_session = Session::new(Arc::clone(&profile), origin);
        http_session.jar = cookies_take(jar);
        let endpoint_resolved =
            core_utils::join_origin(&http_session.origin, route.endpoint.as_str(), false);
        let trust = http_session.trust();
        let session = TabSession::start(SessionStart {
            from,
            target,
            trust,
            text: None,
            ..SessionStart::for_profile(&profile, u64::from(tab.0), core_utils::unix_us())
        });
        let slot = FleetSlot {
            handle,
            site_key: key,
            session: http_session,
            engine_slot,
            site,
            hub_tab: tab.0,
            route_bundle: Arc::new(RouteBundle {
                endpoint: endpoint_resolved,
                route,
            }),
            batcher: TelemetryBatcher::new(BATCH_CAP, batch_interval),
            scratch: bytes::BytesMut::with_capacity((BATCH_CAP + 32) * RAW_EVENT_LEN),
            cookie_scratch: session_state::CookieScratch::default(),
            events_total: 0,
            batches: 0,
        };
        let idx = if let Some(i) = self.free.pop() {
            self.slots[i as usize] = Some(slot);
            i
        } else {
            self.slots.push(Some(slot));
            (self.slots.len() - 1) as u32
        };
        self.tab_to_idx.insert(tab.0, idx);
        self.handles.insert(handle, idx);
        self.live += 1;
        let exp = Instant::now() + self.session_ttl;
        self.expiry_heap.push(Reverse((exp, idx, handle)));
        self.hubs[lane_pick].set_input(local_tab, TabInput::Session(session));
        handle
    }

    fn hub_open_tab(&mut self, site: u32, weight: u32) -> (usize, SlotId) {
        let mut min = self.hubs[0].live_tabs();
        let mut pick = 0usize;
        for (i, hub) in self.hubs.iter().enumerate().skip(1) {
            let l = hub.live_tabs();
            if l < min {
                min = l;
                pick = i;
            }
        }
        let local = self.hubs[pick].open_tab(site, weight);
        (pick, local)
    }
    pub fn detach(&mut self, handle: u32) -> bool {
        let Some(&index) = self.handles.get(&handle) else {
            return false;
        };
        self.handles.remove(&handle);
        let Some(slot) = self.slots[index as usize].take() else {
            return false;
        };
        self.retire(index, slot);
        true
    }

    fn retire(&mut self, index: u32, slot: FleetSlot) {
        self.tab_to_idx.remove(&slot.hub_tab);
        let lane = self.lane(slot.hub_tab);
        let local = SlotId(slot.hub_tab / self.hubs.len() as u32);
        self.hubs[lane].close_tab(local);
        if let Some(pos) = self.hot.iter().position(|&i| i == index) {
            self.hot.swap_remove(pos);
        }
        self.free.push(index);
        self.live -= 1;
        let site_key = slot.site_key;
        let site = slot.site;
        let remove_site = if let Some(site_rec) = self.sites.get_mut(&site_key) {
            site_rec.tabs -= 1;
            site_rec.tabs == 0
        } else {
            false
        };
        if remove_site {
            self.sites.remove(&site_key);
            for hub in self.hubs.iter_mut() {
                hub.unregister_site(site);
            }
        }
    }


    pub fn expire(&mut self, now: Instant) -> usize {
        let mut removed = 0;
        while let Some(&Reverse((exp, idx, handle))) = self.expiry_heap.peek() {
            if exp > now {
                break;
            }
            self.expiry_heap.pop();
            let live = self
                .slots
                .get(idx as usize)
                .and_then(|s| s.as_ref())
                .is_some_and(|s| s.handle == handle);
            if !live {
                continue;
            }
            let slot = self.slots[idx as usize].take().unwrap();
            self.handles.remove(&handle);
            self.retire(idx, slot);
            removed += 1;
        }
        removed
    }

    fn slot_of(&self, tab: u32) -> Option<usize> {
        self.tab_to_idx.get(&tab).map(|&i| i as usize)
    }

    pub fn calibrate(&self, handle: u32, ok: bool) {
        if let Some(site) = self.handle_site(handle)
            && let Some(c) = self.hubs.iter().find_map(|h| h.calibration(site))
        {
            c.record(ok);
        }
    }

    fn handle_site(&self, handle: u32) -> Option<u32> {
        let idx = self.handles.get(&handle).copied()? as usize;
        self.slots
            .get(idx)
            .and_then(|s| s.as_ref().map(|s| s.site))
    }

    pub fn next_due_us(&self) -> u64 {
        self.hubs.iter().map(InputHub::next_due_us).min().unwrap_or(u64::MAX)
    }

    pub fn live_tabs(&self) -> usize {
        self.hubs.iter().map(InputHub::live_tabs).sum()
    }

    pub fn pump(&mut self, now_us: u64, jobs: &mut SmallVec<[PushJob; 8]>) {
        self.expire(Instant::now());
        jobs.clear();
        let lanes_n = self.hubs.len() as u32;
        let mut lanes: TabEvents = SmallVec::new();
        for (lane, hub) in self.hubs.iter_mut().enumerate() {
            let mut events: TabEvents = SmallVec::new();
            hub.tick(now_us, &mut events);
            for (tab, _) in events.iter_mut() {
                tab.0 = lane as u32 + tab.0 * lanes_n;
            }
            lanes.extend(events);
        }
        let mut i = 0usize;
        while i < lanes.len() {
            let tab = lanes[i].0;
            let mut j = i;
            while j < lanes.len() && lanes[j].0 == tab {
                j += 1;
            }
            self.absorb_run(tab, &lanes[i..j]);
            i = j;
        }
        let hot = core::mem::take(&mut self.hot);
        let mut keep: Vec<u32> = Vec::with_capacity(hot.len());
        for idx in hot {
            let Some(slot) = self.slots.get_mut(idx as usize).and_then(|s| s.as_mut()) else {
                continue;
            };
            let mut keep_hot = true;
            if slot.batcher.ready(now_us) && !slot.scratch.is_empty() {
                slot.batches += 1;
                let n = (slot.scratch.len() / RAW_EVENT_LEN) as u64;
                let blob = slot.scratch.split().freeze();
                slot.batcher.begin(now_us);
                let cookie = slot
                    .session
                    .jar
                    .header_for_url_into(slot.route_bundle.endpoint.as_str(), &mut slot.cookie_scratch)
                    .map(CompactString::new)
                    .unwrap_or_default();
                jobs.push(PushJob {
                    handle: slot.handle,
                    engine_slot: slot.engine_slot,
                    route_bundle: Arc::clone(&slot.route_bundle),
                    cookie,
                    blob,
                    events: n,
                });
                keep_hot = !slot.scratch.is_empty();
            }
            if keep_hot {
                keep.push(idx);
            }
        }
        self.hot = keep;
    }

    fn absorb_run(&mut self, tab: SlotId, run: &[(SlotId, payload_gen::input::RawEvent)]) {
        let Some(i) = self.slot_of(tab.0) else {
            return;
        };
        let Some(slot) = self.slots.get_mut(i).and_then(|s| s.as_mut()) else {
            return;
        };
        let n = run.len();
        slot.scratch.reserve(n * RAW_EVENT_LEN);
        for (_, ev) in run {
            slot.scratch
                .extend_from_slice(payload_gen::input::events_bytes(core::slice::from_ref(ev)));
        }
        slot.events_total += n as u64;
        slot.batcher.feed(n);
        self.hot.push(i as u32);
    }

    pub async fn push_job(engines: &EngineSet, job: &PushJob) -> (u16, SmallVec<[CompactString; 4]>) {
        tokio::time::timeout(
            PUSH_CAP,
            push_telemetry(
                engines,
                job.engine_slot,
                &job.route_bundle.route,
                job.route_bundle.endpoint.as_str(),
                job.cookie.as_str(),
                bytes::Bytes::clone(&job.blob),
            ),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or((0, SmallVec::new()))
    }

    pub fn merge_push_cookies(&mut self, handle: u32, lines: &[CompactString]) {
        if lines.is_empty() {
            return;
        }
        let Some(&idx) = self.handles.get(&handle) else {
            return;
        };
        let Some(slot) = self.slots.get_mut(idx as usize).and_then(|s| s.as_mut()) else {
            return;
        };
        let FleetSlot {
            session,
            route_bundle,
            ..
        } = slot;
        let endpoint = &route_bundle.endpoint;
        for line in lines {
            session.jar.ingest_for_url(line.as_str(), endpoint.as_str());
        }
        session.touch();
    }

    pub fn stats(&self) -> FleetStats {
        let (events, batches) = self.slots.iter().flatten().fold((0u64, 0u32), |(e, b), s| {
            (e + s.events_total, b + s.batches)
        });
        FleetStats {
            live_tabs: self.live_tabs(),
            events,
            batches,
        }
    }
}

#[derive(Debug)]
pub struct FleetStats {
    pub live_tabs: usize,
    pub events: u64,
    pub batches: u32,
}

pub enum FleetMsg {
    Attach {
        profile: Arc<Profile>,
        origin: CompactString,
        cookies: CookieJar,
        route: TelemetryRoute,
        engine_slot: usize,
        weight: u32,
        reply: tokio::sync::oneshot::Sender<u32>,
    },
    Stats {
        reply: tokio::sync::oneshot::Sender<FleetStats>,
    },
}

struct PushDone {
    handle: u32,
    events: u64,
    status: u16,
    set_cookies: SmallVec<[CompactString; 4]>,
}

pub async fn fleet_daemon(
    mut fleet: Fleet,
    mut rx: tokio::sync::mpsc::Receiver<FleetMsg>,
    stats: crate::stats::StatsRef,
) {
    let mut jobs: SmallVec<[PushJob; 8]> = SmallVec::new();
    let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<PushDone>(PUSH_DONE_CAP);
    let mut clock_us = core_utils::unix_us();
    loop {
        let next = fleet.next_due_us();
        let wait_us = next.saturating_sub(clock_us).clamp(1_000, 250_000);
        let wait = std::time::Duration::from_micros(wait_us);
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(FleetMsg::Attach {
                    profile,
                    origin,
                    cookies,
                    route,
                    engine_slot,
                    weight,
                    reply,
                }) => {
                    let tab = fleet.attach(
                        profile,
                        origin.as_str(),
                        &cookies,
                        route,
                        engine_slot,
                        weight,
                    );
                    if reply.send(tab).is_err() && tab != u32::MAX {
                        fleet.detach(tab);
                    }
                }
                Some(FleetMsg::Stats { reply }) => {
                    let _ = reply.send(fleet.stats());
                }
                None => break,
            },
            done = done_rx.recv() => {
                if let Some(d) = done {
                    stats.add_touches(d.events);
                    fleet.merge_push_cookies(d.handle, &d.set_cookies);
                    fleet.calibrate(d.handle, d.status / 100 == 2);
                }
            }
            _ = tokio::time::sleep(wait) => {}
        }
        clock_us = core_utils::unix_us();
        if clock_us < next {
            continue;
        }
        let now = clock_us;
        let pumped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fleet.pump(now, &mut jobs);
        }));
        if let Err(_) = pumped {
            let lost: u64 = jobs.iter().map(|j| j.events).sum();
            jobs.clear();
            stats.add_touches(lost);
        }
        for job in jobs.drain(..) {
            let engines = Arc::clone(&fleet.engines);
            let done_tx = done_tx.clone();
            tokio::spawn(async move {
                let handle = job.handle;
                let events = job.events;
                let (status, set_cookies) = Fleet::push_job(&engines, &job).await;
                let _ = done_tx
                    .send(PushDone {
                        handle,
                        events,
                        status,
                        set_cookies,
                    })
                    .await;
            });
        }
    }
}
