use crate::cache::Charge;
use bytes::Bytes;
use crossbeam_channel::Sender;
use compact_str::CompactString;
use payload_gen::input::RawEvent;
use session_state::Profile;
use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(crate) const DEFAULT_RTT_MS: u32 = 50;
const RTT_MIN_MS: u32 = 10;
const RTT_MAX_MS: u32 = 500;
const MIN_BUDGET: Duration = Duration::from_millis(1);
const FETCH_BUDGET_CAP: Duration = Duration::from_secs(30);

pub struct ProfileSnap {
    pub prof: Arc<Profile>,
    pub href: CompactString,
    pub cookie: CompactString,
    pub rtt_ms: u32,
}

impl ProfileSnap {
    pub fn from_parts(profile: &Arc<Profile>, href: &str, cookie: &str) -> Self {
        Self {
            prof: Arc::clone(profile),
            href: CompactString::new(href),
            cookie: CompactString::new(cookie),
            rtt_ms: DEFAULT_RTT_MS,
        }
    }

    #[inline(always)]
    pub fn seed(&self) -> u64 {
        self.prof.canvas_seed
    }

    pub fn with_rtt(mut self, rtt_ms: u32) -> Self {
        self.rtt_ms = rtt_ms.clamp(RTT_MIN_MS, RTT_MAX_MS);
        self
    }
}

pub const NAV_RELOAD: &str = "\u{0}reload";
pub const NAV_HOP_MAX: u8 = 2;

pub fn nav_target<'a>(nav: &'a str, current: &'a str) -> &'a str {
    if nav == NAV_RELOAD {
        current
    } else {
        nav
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecKind {
    #[default]
    Js,
    Wasm,
    Anubis,
}

pub struct ExecReq {
    pub domain: u64,
    pub script: Bytes,
    pub snap: ProfileSnap,
    pub timeout: Duration,
    pub doc: Option<Arc<parser_pipeline::PageData>>,
    pub input: Option<Arc<[RawEvent]>>,
    pub net_slot: usize,
    pub script_node: Option<u32>,
    pub kind: ExecKind,
}

impl ExecReq {
    pub fn bare(
        domain: u64,
        script: Bytes,
        snap: ProfileSnap,
        timeout: Duration,
        kind: ExecKind,
    ) -> Self {
        Self {
            domain,
            script,
            snap,
            timeout,
            doc: None,
            input: None,
            net_slot: 0,
            script_node: None,
            kind,
        }
    }
}

pub(crate) struct ExecTask {
    pub priority: u8,
    pub req: ExecReq,
    pub reply: tokio::sync::oneshot::Sender<ExecOutcome>,
    pub control: Arc<ExecControl>,
    pub _script_charge: Charge,
}

impl ExecTask {
    #[inline]
    pub(crate) fn priority_of(kind: ExecKind) -> u8 {
        match kind {
            ExecKind::Anubis => 0,
            ExecKind::Js => 1,
            ExecKind::Wasm => 2,
        }
    }
}

pub(crate) enum TrySendError {
    Full,
    Disconnected,
}

pub(crate) struct ExecQueueTx {
    critical: crossbeam_channel::Sender<ExecTask>,
    bulk: crossbeam_channel::Sender<ExecTask>,
}

pub(crate) struct ExecQueueRx {
    critical: crossbeam_channel::Receiver<ExecTask>,
    bulk: crossbeam_channel::Receiver<ExecTask>,
}

impl Clone for ExecQueueRx {
    fn clone(&self) -> Self {
        Self {
            critical: self.critical.clone(),
            bulk: self.bulk.clone(),
        }
    }
}

impl ExecQueueRx {
    pub(crate) fn recv(&self) -> Option<ExecTask> {
        crossbeam_channel::select! {
            recv(self.critical) -> task => task.ok(),
            recv(self.bulk) -> task => task.ok(),
        }
    }
}

impl ExecQueueTx {
    pub(crate) fn try_send(&self, task: ExecTask) -> Result<(), TrySendError> {
        let critical = task.priority <= 1;
        let lane = if critical {
            &self.critical
        } else {
            &self.bulk
        };
        match lane.try_send(task) {
            Ok(()) => Ok(()),
            Err(crossbeam_channel::TrySendError::Full(task)) => {
                if critical {
                    return Err(TrySendError::Full);
                }
                match self.critical.try_send(task) {
                    Ok(()) => Ok(()),
                    Err(crossbeam_channel::TrySendError::Full(_)) => Err(TrySendError::Full),
                    Err(_) => Err(TrySendError::Disconnected),
                }
            }
            Err(_) => Err(TrySendError::Disconnected),
        }
    }
}

pub(crate) fn exec_channel(cap: usize) -> (ExecQueueTx, ExecQueueRx) {
    let (critical_tx, critical_rx) = crossbeam_channel::bounded::<ExecTask>(cap / 4 + 1);
    let (bulk_tx, bulk_rx) = crossbeam_channel::bounded::<ExecTask>(cap);
    (
        ExecQueueTx { critical: critical_tx, bulk: bulk_tx },
        ExecQueueRx { critical: critical_rx, bulk: bulk_rx },
    )
}

pub(crate) struct ExecControl {
    pub deadline: Instant,
    cancelled: AtomicBool,
}

impl ExecControl {
    pub fn new(timeout: Duration) -> Arc<Self> {
        let now = Instant::now();
        Arc::new(Self {
            deadline: now.checked_add(timeout).unwrap_or(now),
            cancelled: AtomicBool::new(false),
        })
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn stopped(&self) -> bool {
        self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    #[inline(always)]
    pub(crate) fn cancelled_ptr(&self) -> *const AtomicBool {
        std::ptr::addr_of!(self.cancelled)
    }
}

pub(crate) struct CancelOnDrop(pub Arc<ExecControl>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecPath {
    RawHit,
    NormHit,
    Compile,
    Wasm,
}

#[derive(Debug)]
pub enum ExecError {
    Parse,
    NoResult,
    Timeout,
    Oom,
    Backpressure,
    Shutdown,
    WasmCompile,
    WasmImports,
    WasmFuel,
    Panic,
    Js(CompactString),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            ExecError::Parse => "script parse failed",
            ExecError::NoResult => "no result value",
            ExecError::Timeout => "deadline exceeded",
            ExecError::Oom => "engine oom",
            ExecError::Backpressure => "worker queue full",
            ExecError::Shutdown => "pool down",
            ExecError::WasmCompile => "wasm compile failed",
            ExecError::WasmImports => "wasm imports unsupported",
            ExecError::WasmFuel => "wasm fuel exhausted",
            ExecError::Panic => "worker panic",
            ExecError::Js(m) => return write!(f, "js exception: {m}"),
        };
        f.write_str(label)
    }
}

impl std::error::Error for ExecError {}

pub struct ExecOutcome {
    pub token: Option<CompactString>,
    pub path: ExecPath,
    pub cache_hit: bool,
    pub err: Option<ExecError>,
    pub touches: u64,
    pub cookie_out: Option<CompactString>,
    pub nav: Option<CompactString>,
    pub anubis: Option<challenge_solver::anubis::SolvedAnubis>,
}

impl ExecOutcome {
    pub(crate) fn failed(err: ExecError) -> Self {
        Self {
            token: None,
            path: ExecPath::Compile,
            cache_hit: false,
            err: Some(err),
            touches: 0,
            cookie_out: None,
            nav: None,
            anubis: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Event {
    CacheHit,
    CacheMiss,
    Compile,
    Timeout,
    Oom,
    WasmRun,
    WasmFail,
    Backpressure,
    ParseFail,
    NoResult,
    ExecFail,
    ExecDone(u64),
}

pub type EventTx = Sender<Event>;

pub type HeaderList = SmallVec<[(CompactString, CompactString); 8]>;

pub struct FetchReply {
    pub status: u16,
    pub headers: HeaderList,
    pub set_cookie: SmallVec<[CompactString; 4]>,
    pub body: Bytes,
}

impl FetchReply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn ingest_cookies(&self) {
        for line in &self.set_cookie {
            crate::worker::cookie_set(line.as_str());
        }
    }
}

pub struct FetchJob {
    pub url: CompactString,
    pub method: CompactString,
    pub headers: HeaderList,
    pub body: Option<Bytes>,
    pub cookie: CompactString,
    pub net_slot: usize,
    pub reply: crossbeam_channel::Sender<FetchReply>,
}

static BRIDGE: OnceLock<crossbeam_channel::Sender<FetchJob>> = OnceLock::new();

pub fn install(tx: crossbeam_channel::Sender<FetchJob>) -> bool {
    BRIDGE.set(tx).is_ok()
}

pub fn installed() -> bool {
    BRIDGE.get().is_some()
}

pub fn dispatch(
    url: &str,
    method: &str,
    headers: HeaderList,
    body: Option<Bytes>,
    cookie: CompactString,
    net_slot: usize,
    timeout: Duration,
) -> Option<FetchReply> {
    let tx = BRIDGE.get()?;
    let (r_tx, r_rx) = crossbeam_channel::bounded(1);
    let job = FetchJob {
        url: CompactString::new(url),
        method: CompactString::new(method),
        headers,
        body,
        cookie,
        net_slot,
        reply: r_tx,
    };
    tx.send(job).ok()?;
    r_rx.recv_timeout(timeout.max(MIN_BUDGET)).ok()
}

pub fn deadline_budget(deadline: Instant, cap: Duration) -> Duration {
    deadline
        .checked_duration_since(Instant::now())
        .unwrap_or(MIN_BUDGET)
        .min(cap)
}

pub fn timeout_budget() -> Duration {
    deadline_budget(crate::worker::control_deadline(), FETCH_BUDGET_CAP)
}
