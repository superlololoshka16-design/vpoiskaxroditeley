use compact_str::CompactString;
use rquickjs::{Ctx, Persistent};
use std::cell::{Cell, RefCell};
use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

pub struct ApiKey;

impl ApiKey {
    pub const USER_AGENT: u32 = 0;
    pub const LANGUAGE: u32 = 1;
    pub const LANGUAGES: u32 = 2;
    pub const PLATFORM: u32 = 3;
    pub const HARDWARE_CONCURRENCY: u32 = 4;
    pub const DEVICE_MEMORY: u32 = 5;
    pub const VENDOR: u32 = 6;
    pub const APP_VERSION: u32 = 7;
    pub const PRODUCT: u32 = 8;
    pub const PRODUCT_SUB: u32 = 9;
    pub const OSCPU: u32 = 10;
    pub const BUILD_ID: u32 = 11;
    pub const USER_AGENT_DATA: u32 = 12;
    pub const COOKIE_ENABLED: u32 = 13;
    pub const DO_NOT_TRACK: u32 = 14;
    pub const GLOBAL_PRIVACY_CONTROL: u32 = 15;
    pub const PDF_VIEWER: u32 = 16;
    pub const CONNECTION: u32 = 17;
    pub const WIDTH: u32 = 18;
    pub const HEIGHT: u32 = 19;
    pub const AVAIL_WIDTH: u32 = 20;
    pub const AVAIL_HEIGHT: u32 = 21;
    pub const COLOR_DEPTH: u32 = 22;
    pub const PIXEL_DEPTH: u32 = 23;
    pub const DEVICE_PIXEL_RATIO: u32 = 24;
    pub const SCREEN_X: u32 = 25;
    pub const SCREEN_Y: u32 = 26;
    pub const PAGE_X_OFFSET: u32 = 27;
    pub const PAGE_Y_OFFSET: u32 = 28;
    pub const INNER_WIDTH: u32 = 29;
    pub const INNER_HEIGHT: u32 = 30;
    pub const OUTER_WIDTH: u32 = 31;
    pub const OUTER_HEIGHT: u32 = 32;
    pub const TITLE: u32 = 33;
    pub const REFERRER: u32 = 34;
    pub const URL: u32 = 35;
    pub const DOMAIN: u32 = 36;
    pub const ORIGIN: u32 = 37;
    pub const READY_STATE: u32 = 38;
    pub const CHARACTER_SET: u32 = 39;
    pub const CONTENT_TYPE: u32 = 40;
    pub const COOKIE: u32 = 41;
    pub const HEAD: u32 = 42;
    pub const BODY: u32 = 43;
    pub const SCRIPTS: u32 = 44;
    pub const FORMS: u32 = 45;
    pub const IMAGES: u32 = 46;
    pub const LINKS: u32 = 47;
    pub const EMBEDS: u32 = 48;
    pub const QUERY_SELECTOR: u32 = 49;
    pub const QUERY_SELECTOR_ALL: u32 = 50;
    pub const GET_ELEMENT_BY_ID: u32 = 51;
    pub const GET_ELEMENTS_BY_TAG_NAME: u32 = 52;
    pub const CREATE_ELEMENT: u32 = 53;
    pub const CANVAS_CONTEXT: u32 = 54;
    pub const CANVAS_TO_DATA_URL: u32 = 55;
    pub const WEBGL_CONTEXT: u32 = 56;
    pub const GET_SUPPORTED_EXTENSIONS: u32 = 57;
    pub const GET_PARAMETER: u32 = 58;
    pub const GET_CONTEXT_ATTRIBUTES: u32 = 59;
    pub const CIPHERS: u32 = 60;
    pub const NOW: u32 = 61;
    pub const RANDOM: u32 = 62;
    pub const GET_BOUNDING_CLIENT_RECT: u32 = 63;
    pub const WEBDRIVER: u32 = 64;
    pub const MAX_TOUCH_POINTS: u32 = 65;
    pub const LOCATION_HREF: u32 = 66;
    pub const LOCATION_ORIGIN: u32 = 67;
    pub const LOCATION_HOST: u32 = 68;
    pub const LOCATION_PATHNAME: u32 = 69;
    pub const LOCATION_PROTOCOL: u32 = 70;
    pub const VISIBILITY: u32 = 71;
    pub const XHR: u32 = 72;
    pub const WEBRTC: u32 = 73;
    pub const CURRENT_SCRIPT: u32 = 74;
    pub const INTL: u32 = 75;
    pub const MUTATION_OBSERVER: u32 = 76;
    pub const RESIZE_OBSERVER: u32 = 77;
    pub const INTERSECTION_OBSERVER: u32 = 78;
    pub const FONTS: u32 = 79;
    pub const PERMISSIONS: u32 = 80;
    pub const MEDIA_DEVICES: u32 = 81;
    pub const BATTERY: u32 = 82;
    pub const SEND_BEACON: u32 = 83;
    pub const LOCATION_HOSTNAME: u32 = 84;
    pub const LOCATION_SEARCH: u32 = 85;
    pub const LOCATION_HASH: u32 = 86;
    pub const READ_PIXELS: u32 = 87;
    pub const TOTAL: u32 = 88;
}

pub(crate) const STACK_GRAB_JS: &str =
    "(function () { try { return new Error().stack; } catch (e) { return undefined; } })";

thread_local! {
    static STACK_GRAB: RefCell<Option<Persistent<rquickjs::Function<'static>>>> =
        const { RefCell::new(None) };
}

pub(crate) fn stack_grab_fn<'js>(ctx: &Ctx<'js>) -> Option<rquickjs::Function<'js>> {
    let cached = STACK_GRAB.with(|s| s.borrow().clone());
    if let Some(f) = cached.and_then(|p| p.restore(ctx).ok()) {
        return Some(f);
    }
    let f = ctx.eval::<rquickjs::Function, _>(STACK_GRAB_JS).ok()?;
    STACK_GRAB.with(|s| *s.borrow_mut() = Some(Persistent::save(ctx, f.clone())));
    Some(f)
}

pub(crate) fn clear_thunks() {
    STACK_GRAB.with(|s| *s.borrow_mut() = None);
}

const KEY_NAMES: [&'static str; ApiKey::TOTAL as usize] = [
        "navigator.userAgent",
        "navigator.language",
        "navigator.languages",
        "navigator.platform",
        "navigator.hardwareConcurrency",
        "navigator.deviceMemory",
        "navigator.vendor",
        "navigator.appVersion",
        "navigator.product",
        "navigator.productSub",
        "navigator.oscpu",
        "navigator.buildID",
        "navigator.userAgentData",
        "navigator.cookieEnabled",
        "navigator.doNotTrack",
        "navigator.globalPrivacyControl",
        "navigator.pdfViewerEnabled",
        "navigator.connection",
        "screen.width",
        "screen.height",
        "screen.availWidth",
        "screen.availHeight",
        "screen.colorDepth",
        "screen.pixelDepth",
        "window.devicePixelRatio",
        "window.screenX",
        "window.screenY",
        "window.pageXOffset",
        "window.pageYOffset",
        "window.innerWidth",
        "window.innerHeight",
        "window.outerWidth",
        "window.outerHeight",
        "document.title",
        "document.referrer",
        "document.URL",
        "document.domain",
        "document.origin",
        "document.readyState",
        "document.characterSet",
        "document.contentType",
        "document.cookie",
        "document.head",
        "document.body",
        "document.scripts",
        "document.forms",
        "document.images",
        "document.links",
        "document.embeds",
        "document.querySelector",
        "document.querySelectorAll",
        "document.getElementById",
        "document.getElementsByTagName",
        "document.createElement",
        "HTMLCanvasElement.getContext",
        "HTMLCanvasElement.toDataURL",
        "HTMLCanvasElement.getContext(webgl)",
        "WebGLRenderingContext.getSupportedExtensions",
        "WebGLRenderingContext.getParameter",
        "WebGLRenderingContext.getContextAttributes",
        "crypto.getRandomValues",
        "performance.now",
        "Math.random",
        "Element.getBoundingClientRect",
        "navigator.webdriver",
        "navigator.maxTouchPoints",
        "location.href",
        "location.origin",
        "location.host",
        "location.pathname",
        "location.protocol",
        "document.visibilityState",
        "XMLHttpRequest",
        "RTCPeerConnection",
        "document.currentScript",
        "Intl",
        "MutationObserver",
        "ResizeObserver",
        "IntersectionObserver",
        "document.fonts",
        "navigator.permissions",
        "navigator.mediaDevices",
        "navigator.getBattery",
        "navigator.sendBeacon",
        "location.hostname",
        "location.search",
        "location.hash",
        "WebGLRenderingContext.readPixels",
];

#[inline]
pub fn key_name(idx: u32) -> &'static str {
    KEY_NAMES[idx as usize]
}


#[repr(align(64))]
pub struct TouchLog {
    words: [Cell<u64>; TouchLog::words()],
    stack_keys: [Cell<u64>; TouchLog::words()],
    stacks: RefCell<Vec<(u32, CompactString)>>,
}

impl TouchLog {
    pub const fn words() -> usize {
        (ApiKey::TOTAL as usize).div_ceil(64)
    }

    #[inline(always)]
    pub fn record(&self, key: u32) {
        let idx = key as usize;
        debug_assert!(idx < ApiKey::TOTAL as usize);
        let w = &self.words[idx >> 6];
        w.set(w.get() | (1u64 << (idx & 63)));
    }

    pub fn clear(&self) {
        for w in &self.words {
            w.set(0);
        }
        for w in &self.stack_keys {
            w.set(0);
        }
        self.stacks.borrow_mut().clear();
    }

    pub fn count(&self) -> u32 {
        let mut n = 0u32;
        for w in &self.words {
            n += w.get().count_ones();
        }
        n
    }

    #[cold]
    #[inline(never)]
    pub fn capture_stack(&self, key: u32, ctx: &Ctx<'_>) -> bool {
        let Some(f) = stack_grab_fn(ctx) else { return false };
        let raw: Option<String> = f.call(()).ok();
        let Some(raw) = raw else { return false };
        let first_line = raw.lines().nth(3).unwrap_or("").trim();
        self.stacks
            .borrow_mut()
            .push((key, CompactString::new(first_line)));
        true
    }

    pub fn record_with_stack(&self, key: u32, ctx: &Ctx<'_>) {
        self.record(key);
        let idx = key as usize;
        if idx >= ApiKey::TOTAL as usize {
            return;
        }
        let w = &self.stack_keys[idx >> 6];
        let bit = 1u64 << (idx & 63);
        if w.get() & bit == 0 && self.capture_stack(key, ctx) {
            w.set(w.get() | bit);
        }
    }

    pub fn stacks(&self) -> Vec<(u32, CompactString)> {
        self.stacks.borrow().clone()
    }

    pub fn touched_keys(&self) -> impl Iterator<Item = u32> + '_ {
        self.words
            .iter()
            .enumerate()
            .flat_map(|(wi, w)| {
                let mut x = w.get();
                (0..64).filter_map(move |_| {
                    if x == 0 {
                        return None;
                    }
                    let bit = x.trailing_zeros();
                    x &= x - 1;
                    Some((wi as u32) * 64 + bit)
                })
            })
            .take(ApiKey::TOTAL as usize)
    }
}

static TASK_SEQ: AtomicU32 = AtomicU32::new(0);

pub fn next_task_seq() -> u32 {
    TASK_SEQ.fetch_add(1, Ordering::Relaxed)
}

pub fn task_seq() -> u32 {
    TASK_SEQ.load(Ordering::Relaxed)
}

thread_local! {
    static LOG: TouchLog = const {
        TouchLog {
            words: [const { Cell::new(0) }; TouchLog::words()],
            stack_keys: [const { Cell::new(0) }; TouchLog::words()],
            stacks: const { RefCell::new(Vec::new()) },
        }
    };
}

#[inline(always)]
pub fn touch_log_record(key: u32) {
    LOG.with(|l| l.record(key));
}

#[inline(always)]
pub fn touch_log_record_stack(key: u32, ctx: &Ctx<'_>) {
    LOG.with(|l| l.record_with_stack(key, ctx));
}

pub fn touch_log_reset() -> u32 {
    let seq = next_task_seq();
    LOG.with(|l| l.clear());
    seq
}

pub fn touch_log_count() -> u64 {
    LOG.with(|l| l.count() as u64)
}

pub fn touch_log_dump() -> TouchDump {
    let seq = task_seq();
    let keys: Vec<u32> = LOG.with(|l| l.touched_keys().collect());
    let stacks: Vec<(u32, CompactString)> = LOG.with(|l| l.stacks());
    TouchDump { seq, keys, stacks }
}

#[derive(Clone)]
pub struct TouchDump {
    pub seq: u32,
    pub keys: Vec<u32>,
    pub stacks: Vec<(u32, CompactString)>,
}

impl fmt::Display for TouchDump {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.keys.is_empty() {
            return write!(f, "task#{}: none", self.seq);
        }
        write!(f, "task#{}: {} api touched: ", self.seq, self.keys.len())?;
        let mut first = true;
        for k in &self.keys {
            if *k < ApiKey::TOTAL {
                if !first {
                    f.write_str(", ")?;
                }
                f.write_str(KEY_NAMES[*k as usize])?;
                first = false;
            }
        }
        if !self.stacks.is_empty() {
            f.write_str("\n")?;
        }
        for (k, s) in &self.stacks {
            if *k < ApiKey::TOTAL {
                writeln!(f, "  {} <- {}", KEY_NAMES[*k as usize], s)?;
            }
        }
        Ok(())
    }
}

const PARTIAL_KEYS: &[u32] = &[
    ApiKey::GET_ELEMENTS_BY_TAG_NAME,
    ApiKey::SEND_BEACON,
    ApiKey::WEBRTC,
    ApiKey::FONTS,
    ApiKey::MEDIA_DEVICES,
    ApiKey::PERMISSIONS,
    ApiKey::BATTERY,
    ApiKey::CONNECTION,
    ApiKey::NOW,
];

const NOT_WIRED_KEYS: &[u32] = &[ApiKey::USER_AGENT_DATA];

pub fn vectors_summary() -> (usize, usize, usize) {
    let total = ApiKey::TOTAL as usize;
    let partial = PARTIAL_KEYS.len();
    let not = NOT_WIRED_KEYS.len();
    (total - partial - not, partial, not)
}
