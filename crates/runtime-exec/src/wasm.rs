use std::ffi::{CStr, c_char, c_void};
use compact_str::CompactString;
use core_utils::BytesExt as _;
use std::time::{Duration, Instant};

use crate::task::deadline_budget;

type M3Result = *const c_char;

#[repr(C)]
struct M3Environment(c_void);
#[repr(C)]
struct M3Runtime(c_void);
#[repr(C)]
struct M3Module(c_void);
#[repr(C)]
struct M3Function(c_void);

unsafe extern "C" {
    fn m3_NewEnvironment() -> *mut M3Environment;
    fn m3_FreeEnvironment(env: *mut M3Environment);
    fn m3_NewRuntime(env: *mut M3Environment, stack: u32, userdata: *mut c_void) -> *mut M3Runtime;
    fn m3_FreeRuntime(rt: *mut M3Runtime);
    fn m3_SetFuel(rt: *mut M3Runtime, fuel: i64);
    fn m3_ParseModule(
        env: *mut M3Environment,
        module: *mut *mut M3Module,
        bytes: *const u8,
        len: u32,
    ) -> M3Result;
    fn m3_LoadModule(rt: *mut M3Runtime, module: *mut M3Module) -> M3Result;
    fn m3_RunStart(module: *mut M3Module) -> M3Result;
    fn m3_FreeModule(module: *mut M3Module);
    fn m3_FindFunction(
        out: *mut *mut M3Function,
        rt: *mut M3Runtime,
        name: *const c_char,
    ) -> M3Result;
    fn m3_Call(f: *mut M3Function, argc: u32, args: *const *const c_void) -> M3Result;
    fn m3_GetResults(f: *mut M3Function, retc: u32, rets: *const *const c_void) -> M3Result;
    fn m3_GetArgCount(f: *mut M3Function) -> u32;
    fn m3_GetRetCount(f: *mut M3Function) -> u32;
    fn m3_GetRetType(f: *mut M3Function, index: u32) -> u32;
}
#[derive(Debug)]
pub enum WasmError {
    Malformed,
    Imports,
    NoEntry,
    BadEntry,
    Fuel,
    Timeout,
    Trap(CompactString),
}

impl std::fmt::Display for WasmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WasmError::Malformed => f.write_str("wasm: malformed module"),
            WasmError::Imports => f.write_str("wasm: imports unsupported"),
            WasmError::NoEntry => f.write_str("wasm: no entry export"),
            WasmError::BadEntry => f.write_str("wasm: unsupported entry signature"),
            WasmError::Fuel => f.write_str("wasm: fuel exhausted"),
            WasmError::Timeout => f.write_str("wasm: watchdog deadline"),
            WasmError::Trap(m) => write!(f, "wasm trap: {m}"),
        }
    }
}

impl std::error::Error for WasmError {}
const STACK_BYTES: u32 = 512 * 1024;
const ENTRY_NAMES: [&str; 4] = ["answer", "main", "run", "start"];
const WASM_BUDGET_CAP: Duration = Duration::from_secs(8);
const FUEL_CALLS: i64 = 20_000_000;

#[inline]
fn res_str(res: M3Result) -> Option<&'static str> {
    if res.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(res).to_str().ok() }
}

fn trap(res: M3Result) -> WasmError {
    let msg = res_str(res).unwrap_or("unknown");
    WasmError::Trap(CompactString::const_new(msg))
}

fn m3_ok(res: M3Result) -> Result<(), WasmError> {
    if res.is_null() {
        Ok(())
    } else {
        Err(trap(res))
    }
}

pub(crate) fn is_wasm_magic(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == [0x00, 0x61, 0x73, 0x6d]
}

fn has_import_section(bytes: &[u8]) -> bool {
    let mut pos = 8usize;
    while pos < bytes.len() {
        let id = bytes[pos];
        pos += 1;
        let Some(len) = bytes.leb128(&mut pos) else {
            return false;
        };
        let len = len as usize;
        let Some(body) = bytes.get(pos..pos + len) else {
            return false;
        };
        if id == 2 {
            let mut p = 0usize;
            return body.leb128(&mut p).is_some_and(|count| count > 0);
        }
        pos += len;
    }
    false
}

struct M3Env {
    env: *mut M3Environment,
    rt: *mut M3Runtime,
}

impl M3Env {
    fn new() -> Option<Self> {
        unsafe {
            let env = m3_NewEnvironment();
            if env.is_null() {
                return None;
            }
            let rt = m3_NewRuntime(env, STACK_BYTES, std::ptr::null_mut());
            if rt.is_null() {
                m3_FreeEnvironment(env);
                return None;
            }
            Some(Self { env, rt })
        }
    }
}

impl Drop for M3Env {
    fn drop(&mut self) {
        unsafe {
            m3_FreeRuntime(self.rt);
            m3_FreeEnvironment(self.env);
        }
    }
}

struct NameZ {
    buf: [u8; 16],
}

impl NameZ {
    fn new(name: &str) -> Self {
        debug_assert!(name.len() < 16);
        let mut buf = [0u8; 16];
        let len = name.len().min(15);
        buf[..len].copy_from_slice(&name.as_bytes()[..len]);
        Self { buf }
    }
    fn as_ptr(&self) -> *const c_char {
        self.buf.as_ptr() as *const c_char
    }
}

fn run_inplace(bin: &[u8]) -> Result<CompactString, WasmError> {
    if bin.len() < 8 || !is_wasm_magic(bin) {
        return Err(WasmError::Malformed);
    }
    if has_import_section(bin) {
        return Err(WasmError::Imports);
    }
    let Some(m3) = M3Env::new() else {
        return Err(WasmError::Trap(CompactString::const_new("env alloc")));
    };
    unsafe {
        m3_SetFuel(m3.rt, FUEL_CALLS);
        let mut module: *mut M3Module = std::ptr::null_mut();
        let res = m3_ParseModule(m3.env, &mut module, bin.as_ptr(), bin.len() as u32);
        if let Err(e) = m3_ok(res) {
            m3_FreeModule(module);
            return Err(e);
        }
        m3_ok(m3_LoadModule(m3.rt, module))?;
        m3_ok(m3_RunStart(module))?;
        let mut f: *mut M3Function = std::ptr::null_mut();
        for name in ENTRY_NAMES {
            f = std::ptr::null_mut();
            let nz = NameZ::new(name);
            if m3_FindFunction(&mut f, m3.rt, nz.as_ptr()).is_null() && !f.is_null() {
                break;
            }
        }
        if f.is_null() {
            return Err(WasmError::NoEntry);
        }
        if m3_GetArgCount(f) != 0 || m3_GetRetCount(f) != 1 {
            return Err(WasmError::BadEntry);
        }
        m3_ok(m3_Call(f, 0, std::ptr::null()))?;
        let ty = m3_GetRetType(f, 0);
        if !(1..=4).contains(&ty) {
            return Err(WasmError::BadEntry);
        }
        macro_rules! fetch {
            ($ty:ty) => {{
                let mut v: $ty = 0 as $ty;
                let ptr: *const c_void = &mut v as *mut $ty as *const c_void;
                m3_ok(m3_GetResults(f, 1, [ptr].as_ptr()))?;
                v
            }};
        }
        match ty {
            1 => Ok(core_utils::int_to_compact(fetch!(i32) as i64)),
            2 => Ok(core_utils::int_to_compact(fetch!(i64))),
            3 => Ok(core_utils::float_to_compact(fetch!(f32) as f64)),
            _ => Ok(core_utils::float_to_compact(fetch!(f64))),
        }
    }
}

type WasmReply = Result<CompactString, WasmError>;
type WasmJob = (bytes::Bytes, crossbeam_channel::Sender<WasmReply>);

const SANDBOX_COUNT: usize = 2;

struct Sandbox {
    tx: std::sync::atomic::AtomicPtr<crossbeam_channel::Sender<WasmJob>>,
}

static SANDBOXES: [Sandbox; SANDBOX_COUNT] = [
    Sandbox {
        tx: std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()),
    },
    Sandbox {
        tx: std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()),
    },
];
static RR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn spawn_sandbox() -> crossbeam_channel::Sender<WasmJob> {
    let (tx, rx) = crossbeam_channel::bounded::<WasmJob>(1);
    std::thread::Builder::new()
        .name("silo-wasm3".into())
        .spawn(move || {
            for (bin, reply) in rx {
                let _ = reply.send(run_inplace(bin.as_ref()));
            }
        })
        .expect("wasm3 sandbox spawn");
    tx
}

#[inline]
fn sandbox_tx(idx: usize) -> Option<crossbeam_channel::Sender<WasmJob>> {
    let slot = &SANDBOXES[idx % SANDBOX_COUNT];
    let cur = slot.tx.load(std::sync::atomic::Ordering::Acquire);
    if !cur.is_null() {
        return Some(unsafe { (*cur).clone() });
    }
    let fresh = Box::into_raw(Box::new(spawn_sandbox()));
    match slot.tx.compare_exchange(
        std::ptr::null_mut(),
        fresh,
        std::sync::atomic::Ordering::AcqRel,
        std::sync::atomic::Ordering::Acquire,
    ) {
        Ok(_) => Some(unsafe { (*fresh).clone() }),
        Err(prev) => {
            drop(unsafe { Box::from_raw(fresh) });
            if prev.is_null() {
                return None;
            }
            Some(unsafe { (*prev).clone() })
        }
    }
}

fn sandbox_kill(idx: usize) {
    let slot = &SANDBOXES[idx % SANDBOX_COUNT];
    slot.tx.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
}


pub fn run_wasm(bin: &bytes::Bytes, deadline: Instant) -> Result<CompactString, WasmError> {
    let budget = deadline_budget(deadline, WASM_BUDGET_CAP);
    let idx = RR.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as usize % SANDBOX_COUNT;
    let Some(sender) = sandbox_tx(idx) else {
        return Err(WasmError::Timeout);
    };
    let (tx, rx) = crossbeam_channel::bounded::<WasmReply>(1);
    let job = (bytes::Bytes::clone(bin), tx);
    if sender.send(job).is_err() {
        sandbox_kill(idx);
        return Err(WasmError::Timeout);
    }
    rx.recv_timeout(budget).map_err(|_| WasmError::Timeout)?
}
