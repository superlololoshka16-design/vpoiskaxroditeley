use bumpalo::Bump;
use std::cell::RefCell;

const POOL_CAP: usize = 512 * 1024;
const EV_CAP: usize = 8192;
const ATTR_CAP: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub off: u32,
    pub len: u32,
}

pub(crate) const NIL_SPAN: Span = Span { off: 0, len: 0 };

impl Span {
    #[inline(always)]
    pub fn get(self, pool: &[u8]) -> &str {
        let s = self.off as usize;
        let e = s + self.len as usize;
        debug_assert!(e <= pool.len(), "span out of scratch pool");
        unsafe { core::str::from_utf8_unchecked(pool.get_unchecked(s..e)) }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AttrEv {
    pub name: u16,
    pub name_dyn: Option<Span>,
    pub value: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptEvKind {
    Inline,
    NextData,
    Anubis,
    AnubisVersion,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Ev {
    TitleOpen,
    Script {
        src: Option<Span>,
        kind: ScriptEvKind,
    },
    ScriptText {
        span: Span,
        last: bool,
    },
    TitleText {
        span: Span,
        last: bool,
    },
    MetaDescription {
        span: Span,
    },
    Form {
        action: Option<Span>,
        method: Span,
    },
    Field {
        name: Span,
        value: Option<Span>,
        kind: Span,
        hidden: bool,
    },
    Extract {
        key: u32,
        span: Span,
    },
    DomOpen {
        tag: u16,
        tag_dyn: Option<Span>,
        attr_start: u32,
        attr_count: u8,
    },
    DomText {
        span: Span,
    },
    DomClose {
        tag: u16,
        tag_dyn: Option<Span>,
    },
    ChallengeDetected {
        marker: Span,
    },
}

struct Arena<T> {
    bump: Bump,
    arr: *mut T,
    cap: usize,
    len: usize,
}

impl<T: Copy> Arena<T> {
    fn new(cap: usize) -> Self {
        Self {
            bump: Bump::with_capacity(cap * core::mem::size_of::<T>() + 64),
            arr: core::ptr::null_mut(),
            cap,
            len: 0,
        }
    }

    #[inline(always)]
    fn carve(&mut self) {
        if self.arr.is_null() {
            let layout = core::alloc::Layout::array::<T>(self.cap).unwrap();
            self.arr = self.bump.alloc_layout(layout).as_ptr() as *mut T;
        }
    }

    #[inline(always)]
    fn push(&mut self, value: T) {
        if self.len >= self.cap {
            return;
        }
        self.carve();
        unsafe {
            *self.arr.add(self.len) = value;
        }
        self.len += 1;
    }

    #[inline(always)]
    fn slice(&mut self) -> &[T] {
        self.carve();
        unsafe { core::slice::from_raw_parts(self.arr, self.len) }
    }

    fn reset(&mut self) {
        self.bump.reset();
        self.arr = core::ptr::null_mut();
        self.len = 0;
    }
}

impl Arena<u8> {
    #[inline(always)]
    fn push_bytes(&mut self, bytes: &[u8]) {
        self.carve();
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.arr.add(self.len), bytes.len());
        }
        self.len += bytes.len();
    }
}

pub(crate) struct EvScratch {
    pool: Arena<u8>,
    evs: Arena<Ev>,
    attrs: Arena<AttrEv>,
    overflow: bool,
}

impl EvScratch {
    fn new() -> Self {
        Self {
            pool: Arena::new(POOL_CAP),
            evs: Arena::new(EV_CAP),
            attrs: Arena::new(ATTR_CAP),
            overflow: false,
        }
    }

    #[inline(always)]
    fn push_str(&mut self, s: &str) -> Span {
        let bytes = s.as_bytes();
        if self.pool.len + bytes.len() > POOL_CAP {
            self.overflow = true;
            return NIL_SPAN;
        }
        let span = Span {
            off: self.pool.len as u32,
            len: bytes.len() as u32,
        };
        self.pool.push_bytes(bytes);
        span
    }

    #[inline(always)]
    fn push_attr(&mut self, a: AttrEv) {
        if self.attrs.len >= self.attrs.cap {
            self.overflow = true;
            return;
        }
        self.attrs.push(a);
    }

    #[inline(always)]
    fn emit(&mut self, ev: Ev) {
        if self.evs.len >= self.evs.cap {
            self.overflow = true;
            return;
        }
        self.evs.push(ev);
    }

    fn reset(&mut self) {
        self.pool.reset();
        self.evs.reset();
        self.attrs.reset();
        self.overflow = false;
    }
}

thread_local! {
    static EV: RefCell<EvScratch> = RefCell::new(EvScratch::new());
}

#[inline(always)]
pub(crate) fn push_str(s: &str) -> Span {
    EV.with(|c| c.borrow_mut().push_str(s))
}

#[inline(always)]
pub(crate) fn attr_mark() -> u32 {
    EV.with(|c| c.borrow().attrs.len as u32)
}

#[inline(always)]
pub(crate) fn push_attr(a: AttrEv) {
    EV.with(|c| c.borrow_mut().push_attr(a))
}

#[inline(always)]
pub(crate) fn emit(ev: Ev) {
    EV.with(|c| c.borrow_mut().emit(ev))
}

pub(crate) fn drain_into(collector: &mut crate::collector::Collector) {
    EV.with(|c| {
        let mut guard = c.borrow_mut();
        let sc = &mut *guard;
        if sc.evs.len == 0 && sc.pool.len == 0 && sc.attrs.len == 0 {
            if sc.overflow {
                collector.note_scratch_overflow();
                sc.overflow = false;
            }
            return;
        }
        let pool = sc.pool.slice();
        let evs = sc.evs.slice();
        let attrs = sc.attrs.slice();
        for ev in evs {
            collector.apply(pool, attrs, *ev);
        }
        let over = sc.overflow;
        sc.reset();
        if over {
            collector.note_scratch_overflow();
        }
    });
}
