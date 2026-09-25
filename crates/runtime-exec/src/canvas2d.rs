use crate::timer::clock;
use crate::touch::{self, ApiKey};
use crate::worker::{prof_cpu_scale, prof_raster_seed, with_prof};
use compact_str::CompactString;
use core_utils::rng::mix_ctx;
use core_utils::bump_u32_id;
use payload_gen::{
    CANVAS_MAX_DIM, CANVAS_OP_FILL_RECT, CANVAS_OP_FILL_TEXT, CANVAS_OP_GET_IMAGE_DATA, CANVAS_OP_MEASURE,
    CANVAS_OP_TO_URL, CanvasRaster, READBACK_MAX,
    png_data_url_pixels, webgl_int_param, webgl_param,
};
use rquickjs::IntoJs as _;
use rquickjs::JsLifetime;
use rquickjs::class::Trace;
use rquickjs::function::{Rest, This};
use rquickjs::{Class, Ctx, Function, Object, Persistent, Value};
use std::cell::RefCell;
use std::hash::Hasher as _;
use core_utils::xxh3::XxHash3_64;


#[derive(Clone)]
pub(crate) struct Canvas2DState {
    hasher: XxHash3_64,
    ops: u32,
    seed: u64,
    epoch: u32,
    raster: CanvasRaster,
    cache: RasterCache,
}

#[derive(Clone)]
struct RasterCache {
    key: Option<(u32, u64, u32, u32)>,
    mix: u64,
    buf: Vec<u8>,
}

impl Default for RasterCache {
    fn default() -> Self {
        Self {
            key: None,
            mix: 0,
            buf: Vec::new(),
        }
    }
}

impl Canvas2DState {
    fn new(seed: u64, epoch: u32) -> Self {
        Self {
            hasher: XxHash3_64::new(),
            ops: 0,
            seed,
            epoch,
            raster: CanvasRaster::new(seed),
            cache: RasterCache::default(),
        }
    }

    fn reset_raster(&mut self) {
        self.raster = CanvasRaster::new(self.seed);
        self.cache.key = None;
    }

    fn frame(&mut self, w: u32, h: u32) -> &[u8] {
        let (w, h) = payload_gen::canvas::render_dims(w, h);
        if self.cache.key != Some((self.epoch, self.cache.mix, w, h)) {
            let mix = self.draw_hash(self.epoch);
            self.cache.mix = mix;
            let len = w as usize * h as usize * 4;
            self.cache.buf.clear();
            self.cache.buf.resize(len, 0);
            self.raster.render_into(&mut self.cache.buf, w, h, 0.0, 0.0, mix);
            self.cache.key = Some((self.epoch, mix, w, h));
        }
        &self.cache.buf
    }

    #[inline(always)]
    fn op1(&mut self, tag: u8, a: f64) {
        self.hasher.write_u8(tag);
        self.hasher.write(&a.to_bits().to_le_bytes());
        self.ops += 1;
        self.cache.key = None;
    }

    #[inline(always)]
    fn op4(&mut self, tag: u8, a: f64, b: f64, c: f64, d: f64) {
        self.hasher.write_u8(tag);
        for x in [a, b, c, d] {
            self.hasher.write(&x.to_bits().to_le_bytes());
        }
        self.ops += 1;
        self.cache.key = None;
    }

    #[inline(always)]
    fn text(&mut self, tag: u8, s: &str, x: f64, y: f64) {
        self.hasher.write_u8(tag);
        hash_str(&mut self.hasher, s);
        self.hasher.write(&x.to_bits().to_le_bytes());
        self.hasher.write(&y.to_bits().to_le_bytes());
        self.ops += 1;
        self.cache.key = None;
    }

    #[inline(always)]
    fn str_prop(&mut self, tag: u8, s: &str) {
        self.hasher.write_u8(tag);
        hash_str(&mut self.hasher, s);
    }

    fn live(&self, epoch: u32) -> bool {
        self.epoch == epoch
    }

    fn draw_hash(&self, epoch: u32) -> u64 {
        core_utils::profile::draw_hash(self.hasher.finish(), self.ops, !self.live(epoch))
    }
}

#[inline(always)]
fn hash_str(h: &mut XxHash3_64, s: &str) {
    h.write(s.as_bytes());
    h.write(&[0xFF]);
}

#[inline(always)]
fn gauss() -> f64 {
    crate::worker::FAST_RNG.with_borrow_mut(|rng| rng.gauss())
}

fn pay_cost(op: u8) {
    let cost = core_utils::profile::canvas_time_cost_us(op, prof_cpu_scale(), gauss());
    clock::add_offset_us(cost);
}

fn seed_for(canvas_id: u32) -> u64 {
    core_utils::profile::canvas_seed_of(canvas_id, prof_raster_seed())
}

#[derive(Trace, JsLifetime, Clone)]
#[rquickjs::class(rename_all = "camelCase")]
pub(crate) struct CanvasRenderingContext2D {
    #[qjs(skip_trace)]
    id: u32,
    #[qjs(skip_trace)]
    st: RefCell<Canvas2DState>,
}

#[repr(C)]
struct CanvasRow {
    strings: [CompactString; 8],
    nums: [f64; 3],
}

impl CanvasRow {
    fn fresh() -> Self {
        Self {
            strings: std::array::from_fn(|_| CompactString::const_new("")),
            nums: [1.0, 1.0, 0.0],
        }
    }
}

const S_FILL: usize = 0;
const S_STROKE: usize = 1;
const S_FONT: usize = 2;
const S_BASELINE: usize = 3;
const S_GCO: usize = 4;
const S_SHADOW: usize = 5;
const S_LINECAP: usize = 6;
const S_LINEJOIN: usize = 7;
const N_ALPHA: usize = 0;
const N_LW: usize = 1;
const N_BLUR: usize = 2;

type CtxReg = RefCell<Vec<Option<Persistent<Object<'static>>>>>;

thread_local! {
    static CTX2D: CtxReg = const { RefCell::new(Vec::new()) };
    static CTXGL: CtxReg = const { RefCell::new(Vec::new()) };
    static NEXT_CANVAS: RefCell<u32> = const { RefCell::new(1) };
    static GEN: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    static MK_U8C: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
    static ROWS: RefCell<Vec<CanvasRow>> = const { RefCell::new(Vec::new()) };
}

#[inline]
fn grow<T>(v: &mut Vec<T>, id: u32, fill: impl Fn() -> T) -> usize {
    let idx = (id - 1) as usize;
    if v.len() <= idx {
        v.resize_with(idx + 1, fill);
    }
    idx
}

#[inline]
fn set_style_slot(id: u32, slot: usize, val: &str) {
    ROWS.with(|r| {
        let mut rb = r.borrow_mut();
        let idx = grow(&mut rb, id, CanvasRow::fresh);
        rb[idx].strings[slot] = CompactString::new(val);
    });
}

#[inline]
fn font_kind_of() -> payload_gen::font::FontKind {
    if with_prof(|p| p.prof().platform.as_str() == "Windows") {
        payload_gen::font::FontKind::Windows
    } else {
        payload_gen::font::FontKind::Linux
    }
}

fn font_px_of(id: u32) -> f64 {
    let s = style_slot(id, S_FONT, "10px sans-serif");
    let b = s.as_bytes();
    let mut px: f64 = 10.0;
    let mut i = 0;
    while i < b.len() && !b[i].is_ascii_digit() {
        i += 1;
    }
    let start = i;
    while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
        i += 1;
    }
    if i > start {
        let n: f64 = s[start..i].parse().unwrap_or(10.0);
        if n.is_finite() && (1.0..=500.0).contains(&n) {
            px = n;
        }
    }
    px
}

#[inline]
fn style_slot(id: u32, slot: usize, default: &'static str) -> CompactString {
    ROWS.with(|r| {
        let rb = r.borrow();
        let row = rb.get((id - 1) as usize)?;
        let v = &row.strings[slot];
        if v.is_empty() { None } else { Some(v.clone()) }
    })
    .unwrap_or_else(|| CompactString::const_new(default))
}

#[inline]
fn set_num_slot(id: u32, slot: usize, v: f64) {
    ROWS.with(|r| {
        let mut rb = r.borrow_mut();
        let idx = grow(&mut rb, id, CanvasRow::fresh);
        rb[idx].nums[slot] = v;
    });
}

#[inline]
fn num_slot(id: u32, slot: usize, default: f64) -> f64 {
    ROWS.with(|r| {
        let rb = r.borrow();
        rb.get((id - 1) as usize)
            .map(|row| row.nums[slot])
            .unwrap_or(default)
    })
}


impl CanvasRenderingContext2D {
    #[inline]
    fn set_str_style<'js>(&mut self, tag: u8, slot: usize, v: &Value<'js>) {
        let sc = crate::webidl::value_to_str(v);
        let s: &str = sc.as_deref().unwrap_or("");
        self.st.borrow_mut().str_prop(tag, s);
        set_style_slot(self.id, slot, s);
    }

    fn style_get<'js>(
        &self,
        ctx: &Ctx<'js>,
        slot: usize,
        default: &'static str,
    ) -> rquickjs::Result<Value<'js>> {
        style_slot(self.id, slot, default).as_str().into_js(ctx)
    }

    fn set_num(&mut self, tag: u8, slot: usize, v: f64) {
        self.st.borrow_mut().op1(tag, v);
        set_num_slot(self.id, slot, v);
    }
}

fn ctx_slot<'js>(
    ctx: &Ctx<'js>,
    reg: &'static std::thread::LocalKey<CtxReg>,
    id: u32,
) -> Option<Object<'js>> {
    reg.with(|m| m.borrow().get((id - 1) as usize).cloned().flatten())
        .and_then(|p| p.restore(ctx).ok())
}

fn ctx_save<'js>(
    ctx: &Ctx<'js>,
    reg: &'static std::thread::LocalKey<CtxReg>,
    id: u32,
    obj: Object<'js>,
) {
    reg.with(|m| {
        let mut mb = m.borrow_mut();
        let idx = grow(&mut mb, id, || None);
        mb[idx] = Some(Persistent::save(ctx, obj));
    });
}

pub(crate) fn clear_registry() {
    CTX2D.with(|m| m.borrow_mut().clear());
    CTXGL.with(|m| m.borrow_mut().clear());
    NEXT_CANVAS.with(|n| *n.borrow_mut() = 1);
    GEN.with(|m| m.borrow_mut().clear());
    ROWS.with(|m| m.borrow_mut().clear());
}

pub(crate) fn clear_thunks() {
    MK_U8C.with(|c| *c.borrow_mut() = None);
}

pub(crate) fn init<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let mk: Function = ctx.eval("(function (n) { return new Uint8ClampedArray(n); })")?;
    MK_U8C.with(|c| *c.borrow_mut() = Some(Persistent::save(ctx, mk)));
    Ok(())
}

pub(crate) fn new_canvas<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    let id = NEXT_CANVAS.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n = bump_u32_id(*n);
        id
    });
    GEN.with(|m| {
        let mut mb = m.borrow_mut();
        let idx = grow(&mut mb, id, || 1);
        mb[idx] = 1;
    });
    ROWS.with(|r| {
        grow(&mut r.borrow_mut(), id, CanvasRow::fresh);
    });
    let class: Class<HTMLCanvasElement> =
        Class::instance(ctx.clone(), HTMLCanvasElement { id, w: 300, h: 150 })?;
    Ok(class.into_inner().into_value())
}

fn canvas_epoch(id: u32) -> u32 {
    GEN.with(|m| *m.borrow().get((id - 1) as usize).unwrap_or(&0))
}

fn bump_epoch(id: u32) {
    GEN.with(|m| {
        let mut mb = m.borrow_mut();
        let idx = grow(&mut mb, id, || 1);
        mb[idx] = bump_u32_id(mb[idx]);
    });
}

fn create_gradient<'js>(
    st: &RefCell<Canvas2DState>,
    ctx: &Ctx<'js>,
    tag: u8,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
) -> rquickjs::Result<Value<'js>> {
    st.borrow_mut().op4(tag, x0, y0, x1, y1);
    build_gradient(ctx)
}

#[rquickjs::methods]
impl CanvasRenderingContext2D {
    #[qjs(rename = "fillRect")]
    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        pay_cost(CANVAS_OP_FILL_RECT);
        let mut st = self.st.borrow_mut();
        st.op4(1, x, y, w, h);
        st.raster.fill_rect(x, y, w, h);
    }

    #[qjs(rename = "strokeRect")]
    pub fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let mut st = self.st.borrow_mut();
        st.op4(2, x, y, w, h);
        st.raster.stroke_rect(x, y, w, h);
    }

    #[qjs(rename = "clearRect")]
    pub fn clear_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let mut st = self.st.borrow_mut();
        st.op4(3, x, y, w, h);
        st.raster.clear_rect(x, y, w, h);
    }

    #[qjs(rename = "beginPath")]
    pub fn begin_path(&mut self) {
        let mut st = self.st.borrow_mut();
        st.op1(4, 0.0);
        st.raster.begin_path();
    }

    #[qjs(rename = "closePath")]
    pub fn close_path(&mut self) {
        let mut st = self.st.borrow_mut();
        st.op1(5, 0.0);
        st.raster.close_path();
    }

    #[qjs(rename = "moveTo")]
    pub fn move_to(&mut self, x: f64, y: f64) {
        let mut st = self.st.borrow_mut();
        st.op4(6, x, y, 0.0, 0.0);
        st.raster.move_to(x, y);
    }

    #[qjs(rename = "lineTo")]
    pub fn line_to(&mut self, x: f64, y: f64) {
        let mut st = self.st.borrow_mut();
        st.op4(7, x, y, 0.0, 0.0);
        st.raster.line_to(x, y);
    }

    #[qjs(rename = "quadraticCurveTo")]
    pub fn quadratic_curve_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
        let mut st = self.st.borrow_mut();
        st.op4(8, cx, cy, x, y);
        st.raster.quadratic_curve_to(cx, cy, x, y);
    }

    #[qjs(rename = "bezierCurveTo")]
    pub fn bezier_curve_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) {
        let mut st = self.st.borrow_mut();
        st.op4(9, c1x, c1y, c2x, c2y);
        st.op4(9, x, y, 0.0, 0.0);
        st.raster.bezier_curve_to(c1x, c1y, c2x, c2y, x, y);
    }

    pub fn arc(&mut self, x: f64, y: f64, r: f64, start: f64, end: f64) {
        let mut st = self.st.borrow_mut();
        st.op4(10, x, y, r, start);
        st.op1(11, end);
        st.raster.arc(x, y, r, start, end);
    }

    #[qjs(rename = "arcTo")]
    pub fn arc_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64) {
        let mut st = self.st.borrow_mut();
        st.op4(12, x1, y1, x2, y2);
        st.raster.line_to(x2, y2);
    }

    pub fn ellipse(&mut self, x: f64, y: f64, rx: f64, ry: f64) {
        let mut st = self.st.borrow_mut();
        st.op4(13, x, y, rx, ry);
        st.raster.ellipse(x, y, rx, ry, 0.0, 0.0, core::f64::consts::TAU);
    }

    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let mut st = self.st.borrow_mut();
        st.op4(14, x, y, w, h);
        st.raster.line_to(x, y);
        st.raster.line_to(x + w, y);
        st.raster.line_to(x + w, y + h);
        st.raster.line_to(x, y + h);
        st.raster.close_path();
    }

    pub fn fill(&mut self) {
        let mut st = self.st.borrow_mut();
        st.op1(15, 0.0);
        st.raster.fill();
    }

    pub fn stroke(&mut self) {
        let mut st = self.st.borrow_mut();
        st.op1(16, 0.0);
        st.raster.stroke();
    }

    pub fn clip(&mut self) {
        self.st.borrow_mut().op1(17, 0.0);
    }

    pub fn save(&mut self) {
        self.st.borrow_mut().op1(18, 0.0);
    }

    pub fn restore(&mut self) {
        self.st.borrow_mut().op1(19, 0.0);
    }

    pub fn translate(&mut self, x: f64, y: f64) {
        self.st.borrow_mut().op4(20, x, y, 0.0, 0.0);
    }

    pub fn rotate(&mut self, a: f64) {
        self.st.borrow_mut().op1(21, a);
    }

    pub fn scale(&mut self, x: f64, y: f64) {
        self.st.borrow_mut().op4(22, x, y, 0.0, 0.0);
    }

    pub fn transform(&mut self, a: f64, b: f64, c: f64, d: f64) {
        self.st.borrow_mut().op4(23, a, b, c, d);
    }

    #[qjs(rename = "setTransform")]
    pub fn set_transform(&mut self, a: f64, b: f64, c: f64, d: f64) {
        self.st.borrow_mut().op4(24, a, b, c, d);
    }

    #[qjs(rename = "fillText")]
    pub fn fill_text<'js>(&mut self, text: rquickjs::String<'js>, x: f64, y: f64) {
        pay_cost(CANVAS_OP_FILL_TEXT);
        if let Ok(tc) = text.to_cstring() {
            let px = font_px_of(self.id);
            let kind = font_kind_of();
            let mut st = self.st.borrow_mut();
            st.raster.set_font_kind(kind);
            st.text(25, tc.as_str(), x, y);
            st.raster.fill_text(tc.as_str(), x, y, px);
        }
    }

    #[qjs(rename = "strokeText")]
    pub fn stroke_text<'js>(&mut self, text: rquickjs::String<'js>, x: f64, y: f64) {
        if let Ok(tc) = text.to_cstring() {
            let px = font_px_of(self.id);
            let kind = font_kind_of();
            let mut st = self.st.borrow_mut();
            st.raster.set_font_kind(kind);
            st.text(26, tc.as_str(), x, y);
            st.raster.stroke_text(tc.as_str(), x, y, px);
        }
    }

    #[qjs(rename = "measureText")]
    pub fn measure_text<'js>(
        &mut self,
        ctx: Ctx<'js>,
        text: rquickjs::String<'js>,
    ) -> rquickjs::Result<Value<'js>> {
        let tc = text.to_cstring()?;
        let px = font_px_of(self.id);
        let kind = font_kind_of();
        let m = payload_gen::font::measure(kind, tc.as_str(), px);
        self.st.borrow_mut().op1(27, m.width);
        pay_cost(CANVAS_OP_MEASURE);
        let o = Object::new(ctx.clone())?;
        o.set("width", m.width)?;
        o.set("actualBoundingBoxLeft", m.actual_left)?;
        o.set("actualBoundingBoxRight", m.actual_right)?;
        o.set("actualBoundingBoxAscent", m.ascent)?;
        o.set("actualBoundingBoxDescent", m.descent)?;
        Ok(o.into_value())
    }

    #[qjs(rename = "drawImage")]
    pub fn draw_image(
        &mut self,
        _img: Value<'_>,
        dx: f64,
        dy: f64,
        dw: Option<f64>,
        dh: Option<f64>,
    ) {
        let mut st = self.st.borrow_mut();
        st.op4(28, dx, dy, dw.unwrap_or(0.0), dh.unwrap_or(0.0));
    }

    #[qjs(rename = "putImageData")]
    pub fn put_image_data(&mut self, data: Value<'_>, dx: f64, dy: f64) {
        let mut st = self.st.borrow_mut();
        let len = data
            .as_object()
            .and_then(|o| o.as_typed_array::<u8>())
            .map(|t| t.len())
            .unwrap_or(0);
        st.op4(29, len as f64, dx, dy, 0.0);
    }

    #[qjs(rename = "getImageData")]
    pub fn get_image_data<'js>(
        &mut self,
        ctx: Ctx<'js>,
        sx: f64,
        sy: f64,
        sw: f64,
        sh: f64,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::CANVAS_TO_DATA_URL);
        if !(sw.is_finite() && sh.is_finite())
            || sw <= 0.0
            || sh <= 0.0
            || sw > READBACK_MAX as f64
            || sh > READBACK_MAX as f64
        {
            return Err(rquickjs::Exception::throw_message(
                &ctx,
                "IndexSizeError: The source width is 0.",
            ));
        }
        let w = sw as u32;
        let h = sh as u32;
        let len = (w as usize) * (h as usize) * 4;
        pay_cost(CANVAS_OP_GET_IMAGE_DATA);
        let out = Object::new(ctx.clone())?;
        let Some(mk) = crate::webidl::restore_slot(&ctx, &MK_U8C) else {
            return Ok(out.into_value());
        };
        let arr: Value<'js> = mk.call((len as u32,))?;
        if let Some(ta) = arr
            .as_object()
            .and_then(|o| o.as_typed_array::<rquickjs::U8Clamped>())
            .and_then(|t| t.as_raw().map(|raw| (t.len(), raw)))
            .map(|(l, raw)| (l, raw.ptr.as_ptr(), raw.len))
        {
            let (l, ptr, rlen) = ta;
            if l >= len {
            let bytes = unsafe { std::slice::from_raw_parts_mut(ptr, rlen) };
            let dst = &mut bytes[..len];
            let mut st = self.st.borrow_mut();
            let epoch = canvas_epoch(self.id);
            if !st.live(epoch) {
                st.reset_raster();
                st.epoch = epoch;
            }
            let mix = st.draw_hash(st.epoch);
            st.raster.render_into(dst, w, h, sx, sy, mix);
            }
        }
        out.set("data", arr)?;
        out.set("width", w)?;
        out.set("height", h)?;
        Ok(out.into_value())
    }

    #[qjs(rename = "isPointInPath")]
    pub fn is_point_in_path(&mut self) -> bool {
        false
    }

    #[qjs(rename = "createLinearGradient")]
    pub fn create_linear_gradient<'js>(
        &mut self,
        ctx: Ctx<'js>,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    ) -> rquickjs::Result<Value<'js>> {
        create_gradient(&self.st, &ctx, 41, x0, y0, x1, y1)
    }

    #[qjs(rename = "createRadialGradient")]
    pub fn create_radial_gradient<'js>(
        &mut self,
        ctx: Ctx<'js>,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    ) -> rquickjs::Result<Value<'js>> {
        create_gradient(&self.st, &ctx, 42, x0, y0, x1, y1)
    }

    #[qjs(get, rename = "fillStyle")]
    pub fn get_fill_style<'js>(&self, c: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.style_get(&c, S_FILL, "#000000")
    }

    #[qjs(set, rename = "fillStyle")]
    pub fn fill_style<'js>(&mut self, v: Value<'js>) {
        self.set_str_style(30, S_FILL, &v);
        if let Some(s) = crate::webidl::value_to_str(&v) {
            let mut st = self.st.borrow_mut();
            st.raster.set_fill(payload_gen::parse_color(s.as_str()));
        }
    }

    #[qjs(get, rename = "strokeStyle")]
    pub fn get_stroke_style<'js>(&self, c: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.style_get(&c, S_STROKE, "#000000")
    }

    #[qjs(set, rename = "strokeStyle")]
    pub fn stroke_style<'js>(&mut self, v: Value<'js>) {
        self.set_str_style(31, S_STROKE, &v);
        if let Some(s) = crate::webidl::value_to_str(&v) {
            let mut st = self.st.borrow_mut();
            st.raster.set_stroke(payload_gen::parse_color(s.as_str()));
        }
    }

    #[qjs(get, rename = "font")]
    pub fn get_font<'js>(&self, c: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.style_get(&c, S_FONT, "10px sans-serif")
    }

    #[qjs(set, rename = "font")]
    pub fn font<'js>(&mut self, v: Value<'js>) {
        self.set_str_style(32, S_FONT, &v);
    }

    #[qjs(get, rename = "textBaseline")]
    pub fn get_text_baseline<'js>(&self, c: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.style_get(&c, S_BASELINE, "alphabetic")
    }

    #[qjs(set, rename = "textBaseline")]
    pub fn text_baseline<'js>(&mut self, v: Value<'js>) {
        self.set_str_style(33, S_BASELINE, &v);
    }

    #[qjs(get, rename = "globalCompositeOperation")]
    pub fn get_global_composite_operation<'js>(
        &self,
        c: Ctx<'js>,
    ) -> rquickjs::Result<Value<'js>> {
        self.style_get(&c, S_GCO, "source-over")
    }

    #[qjs(set, rename = "globalCompositeOperation")]
    pub fn global_composite_operation<'js>(&mut self, v: Value<'js>) {
        self.set_str_style(35, S_GCO, &v);
    }

    #[qjs(get, rename = "shadowColor")]
    pub fn get_shadow_color<'js>(&self, c: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.style_get(&c, S_SHADOW, "rgba(0, 0, 0, 0)")
    }

    #[qjs(set, rename = "shadowColor")]
    pub fn shadow_color<'js>(&mut self, v: Value<'js>) {
        self.set_str_style(38, S_SHADOW, &v);
    }

    #[qjs(get, rename = "lineCap")]
    pub fn get_line_cap<'js>(&self, c: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.style_get(&c, S_LINECAP, "butt")
    }

    #[qjs(set, rename = "lineCap")]
    pub fn line_cap<'js>(&mut self, v: Value<'js>) {
        self.set_str_style(39, S_LINECAP, &v);
    }

    #[qjs(get, rename = "lineJoin")]
    pub fn get_line_join<'js>(&self, c: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.style_get(&c, S_LINEJOIN, "miter")
    }

    #[qjs(set, rename = "lineJoin")]
    pub fn line_join<'js>(&mut self, v: Value<'js>) {
        self.set_str_style(40, S_LINEJOIN, &v);
    }

    #[qjs(get, rename = "globalAlpha")]
    pub fn get_global_alpha(&self) -> f64 {
        num_slot(self.id, N_ALPHA, 1.0)
    }

    #[qjs(set, rename = "globalAlpha")]
    pub fn set_global_alpha(&mut self, v: f64) {
        self.set_num(34, N_ALPHA, v);
        self.st.borrow_mut().raster.set_alpha(v);
    }

    #[qjs(get, rename = "lineWidth")]
    pub fn get_line_width(&self) -> f64 {
        num_slot(self.id, N_LW, 1.0)
    }

    #[qjs(set, rename = "lineWidth")]
    pub fn set_line_width(&mut self, v: f64) {
        self.set_num(36, N_LW, v);
        self.st.borrow_mut().raster.set_line_width(v);
    }

    #[qjs(get, rename = "shadowBlur")]
    pub fn get_shadow_blur(&self) -> f64 {
        num_slot(self.id, N_BLUR, 0.0)
    }

    #[qjs(set, rename = "shadowBlur")]
    pub fn set_shadow_blur(&mut self, v: f64) {
        self.set_num(37, N_BLUR, v);
    }
}

fn build_gradient<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    let g = Object::new(ctx.clone())?;
    let add_stop = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, _off: f64, _col: Value<'_>| -> Value<'_> { Value::new_undefined(c) },
    )?;
    crate::webidl::define_method(ctx, &g, "addColorStop", add_stop)?;
    Ok(g.into_value())
}

#[derive(Trace, JsLifetime, Clone)]
#[rquickjs::class(rename_all = "camelCase")]
pub(crate) struct HTMLCanvasElement {
    #[qjs(skip_trace)]
    id: u32,
    #[qjs(skip_trace)]
    w: u32,
    #[qjs(skip_trace)]
    h: u32,
}

pub(crate) fn is_canvas_value(v: &Value<'_>) -> bool {
    v.as_object()
        .and_then(Class::<HTMLCanvasElement>::from_object)
        .is_some()
}

pub(crate) fn link_webidl<'js>(ctx: &Ctx<'js>, html_parent: &Object<'js>) -> rquickjs::Result<()> {
    let proto = match Class::<HTMLCanvasElement>::prototype(ctx)? {
        Some(p) => p,
        None => return Ok(()),
    };
    proto.set_prototype(Some(html_parent))?;
    let ctor = crate::webidl::illegal_ctor_fn(ctx, "HTMLCanvasElement")?;
    crate::webidl::install_host_ctor(ctx, &ctor, &proto, "HTMLCanvasElement", 0, true)?;
    let ctx_proto = match Class::<CanvasRenderingContext2D>::prototype(ctx)? {
        Some(p) => p,
        None => return Ok(()),
    };
    let ctx_ctor = crate::webidl::illegal_ctor_fn(ctx, "CanvasRenderingContext2D")?;
    crate::webidl::install_host_ctor(ctx, &ctx_ctor, &ctx_proto, "CanvasRenderingContext2D", 0, true)?;
    let gl_proto = match Class::<WebGLRenderingContext>::prototype(ctx)? {
        Some(p) => p,
        None => return Ok(()),
    };
    let gl_ctor = crate::webidl::illegal_ctor_fn(ctx, "WebGLRenderingContext")?;
    crate::webidl::install_host_ctor(ctx, &gl_ctor, &gl_proto, "WebGLRenderingContext", 0, true)?;
    crate::webidl::register_canvas_proto(ctx, &proto);
    Ok(())
}

#[rquickjs::methods]
impl HTMLCanvasElement {
    #[qjs(get, rename = "width")]
    pub fn width(&self) -> u32 {
        self.w
    }

    #[qjs(set, rename = "width")]
    pub fn set_width(&mut self, v: u32) {
        self.w = v.min(CANVAS_MAX_DIM);
        bump_epoch(self.id);
    }

    #[qjs(get, rename = "height")]
    pub fn height(&self) -> u32 {
        self.h
    }

    #[qjs(set, rename = "height")]
    pub fn set_height(&mut self, v: u32) {
        self.h = v.min(CANVAS_MAX_DIM);
        bump_epoch(self.id);
    }

    #[qjs(rename = "getContext")]
    pub fn get_context<'js>(
        &self,
        ctx: Ctx<'js>,
        kind: rquickjs::String<'js>,
    ) -> rquickjs::Result<Value<'js>> {
        let kc = kind.to_cstring()?;
        match kc.as_str() {
            "2d" => {
                touch::touch_log_record(ApiKey::CANVAS_CONTEXT);
                if let Some(v) = ctx_slot(&ctx, &CTX2D, self.id) {
                    return Ok(v.into_value());
                }
                let st = Canvas2DState::new(seed_for(self.id), canvas_epoch(self.id));
                let class: Class<CanvasRenderingContext2D> = Class::instance(
                    ctx.clone(),
                    CanvasRenderingContext2D {
                        id: self.id,
                        st: RefCell::new(st),
                    },
                )?;
                let obj: Object = class.into_inner();
                ctx_save(&ctx, &CTX2D, self.id, obj.clone());
                Ok(obj.into_value())
            }
            "webgl" | "experimental-webgl" | "webgl2" => {
                touch::touch_log_record(ApiKey::WEBGL_CONTEXT);
                if let Some(v) = ctx_slot(&ctx, &CTXGL, self.id) {
                    return Ok(v.into_value());
                }
                let gl = build_gl(&ctx, self.id, kc.as_str() == "webgl2")?;
                ctx_save(&ctx, &CTXGL, self.id, gl.clone());
                Ok(gl.into_value())
            }
            _ => Ok(Value::new_null(ctx.clone())),
        }
    }

    #[qjs(skip)]
    fn render_synced<'js>(&self, ctx: &Ctx<'js>, f: &mut dyn FnMut(u32, u32, &[u8])) {
        let epoch = canvas_epoch(self.id);
        let c = ctx_slot(ctx, &CTX2D, self.id)
            .and_then(|o| Class::<CanvasRenderingContext2D>::from_object(&o));
        let Some(c) = c else { return };
        let class = c.borrow();
        let mut st = class.st.borrow_mut();
        if !st.live(epoch) {
            st.reset_raster();
            st.epoch = epoch;
        }
        let (w, h) = payload_gen::canvas::render_dims(self.w, self.h);
        f(w, h, st.frame(self.w, self.h));
    }

    #[qjs(skip)]
    fn render_frame<'js>(&self, ctx: &Ctx<'js>) -> Option<Vec<u8>> {
        let mut out = None;
        self.render_synced(ctx, &mut |_w, _h, px| {
            out = Some(px.to_vec());
        });
        out
    }

    #[qjs(rename = "toDataURL")]
    pub fn to_data_url<'js>(
        &self,
        ctx: Ctx<'js>,
        _mime: Rest<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::CANVAS_TO_DATA_URL);
        let mut url = None;
        self.render_synced(&ctx, &mut |w, h, px| {
            url = Some(png_data_url_pixels(w, h, px));
        });
        if url.is_none() {
            url = Some(png_data_url_pixels(1, 1, &[]));
        }
        pay_cost(CANVAS_OP_TO_URL);
        url.unwrap().into_js(&ctx)
    }

    #[qjs(rename = "toBlob")]
    pub fn to_blob<'js>(
        &self,
        ctx: Ctx<'js>,
        _rest: Rest<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::CANVAS_TO_DATA_URL);
        let mut bytes = None;
        self.render_synced(&ctx, &mut |w, h, px| {
            bytes = Some(payload_gen::png_bytes_pixels(w, h, px));
        });
        let bytes = bytes.unwrap_or_default();
        pay_cost(CANVAS_OP_TO_URL);
        if let Some(cb) = _rest.first().and_then(|v| v.as_function()) {
            let blob = Object::new(ctx.clone())?;
            blob.set("size", bytes.len() as u32)?;
            blob.set("type", "image/png")?;
            let _: rquickjs::Result<Value<'js>> = cb.call((This(blob),))?;
        }
        Ok(Value::new_undefined(ctx.clone()))
    }

}

const GL_EXT_LIST: [&str; 24] = [
    "ANGLE_instanced_arrays",
    "EXT_blend_minmax",
    "EXT_color_buffer_half_float",
    "EXT_disjoint_timer_query",
    "EXT_float_blend",
    "EXT_frag_depth",
    "EXT_shader_texture_lod",
    "EXT_texture_compression_bptc",
    "EXT_texture_filter_anisotropic",
    "OES_element_index_uint",
    "OES_fbo_render_mipmap",
    "OES_standard_derivatives",
    "OES_texture_float",
    "OES_texture_float_linear",
    "OES_texture_half_float",
    "OES_texture_half_float_linear",
    "OES_vertex_array_object",
    "WEBGL_color_buffer_float",
    "WEBGL_compressed_texture_s3tc",
    "WEBGL_debug_renderer_info",
    "WEBGL_depth_texture",
    "WEBGL_draw_buffers",
    "WEBGL_lose_context",
    "WEBGL_multi_draw",
];

const GL_INT_SLOT: [(i64, u8); 17] = [
    (3379, 0),
    (34076, 1),
    (35373, 2),
    (36349, 3),
    (36348, 4),
    (35379, 4),
    (36347, 7),
    (34921, 7),
    (35660, 8),
    (3410, 11),
    (3411, 11),
    (3412, 11),
    (3413, 11),
    (3414, 12),
    (3415, 11),
    (36183, 13),
    (35371, 13),
];

fn gl_param<'js>(ctx: &Ctx<'js>, seed: u64, p: f64, es2: bool) -> Value<'js> {
    if let Some(v) = gl_string(ctx, p as i64, es2) {
        return v;
    }
    let e = p as i64;
    if let Some(&(_, slot)) = GL_INT_SLOT.iter().find(|&&(k, _)| k == e) {
        return Value::new_number(ctx.clone(), f64::from(webgl_int_param(seed, slot)));
    }
    let slot = core_utils::rng::mix64(u64::from(e as u32)) as u8;
    Value::new_number(ctx.clone(), webgl_param(seed, slot))
}

fn gl_string<'js>(ctx: &Ctx<'js>, p: i64, es2: bool) -> Option<Value<'js>> {
    let s = match (p, es2) {
        (7936, _) => "WebKit",
        (7937, _) => "WebKit WebGL",
        (7938, false) => payload_gen::WEBGL1_VERSION,
        (7938, true) => payload_gen::WEBGL2_VERSION,
        (35724, false) => payload_gen::WEBGL1_GLSL,
        (35724, true) => payload_gen::WEBGL2_GLSL,
        _ => return None,
    };
    s.into_js(ctx).ok()
}

fn gl_object<'js>(c: &Ctx<'js>) -> Value<'js> {
    match Object::new(c.clone()) {
        Ok(o) => o.into_value(),
        Err(_) => Value::new_null(c.clone()),
    }
}

#[derive(Trace, JsLifetime, Clone)]
#[rquickjs::class(rename_all = "camelCase")]
pub(crate) struct WebGLRenderingContext {
    #[qjs(skip_trace)]
    seed: u64,
    #[qjs(skip_trace)]
    canvas_id: u32,
    #[qjs(skip_trace)]
    es2: bool,
}

fn build_gl<'js>(ctx: &Ctx<'js>, canvas_id: u32, es2: bool) -> rquickjs::Result<Object<'js>> {
    let gl_seed = core_utils::profile::gl_seed_of(prof_raster_seed(), canvas_id, canvas_epoch(canvas_id));
    let class: Class<WebGLRenderingContext> = Class::instance(
        ctx.clone(),
        WebGLRenderingContext {
            seed: gl_seed,
            canvas_id,
            es2,
        },
    )?;
    Ok(class.into_inner())
}

#[rquickjs::methods]
impl WebGLRenderingContext {
    #[qjs(rename = "getParameter")]
    pub fn get_parameter<'js>(&self, c: Ctx<'js>, p: f64) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::GET_PARAMETER);
        match p as i64 {
            37445 => {
                let v = crate::worker::with_prof(|p| p.prof().webgl_vendor());
                v.into_js(&c)
            }
            37446 => {
                let v = crate::worker::with_prof(|p| p.prof().webgl_renderer());
                v.into_js(&c)
            }
            _ => Ok(gl_param(&c, self.seed, p, self.es2)),
        }
    }

    #[qjs(rename = "getExtension")]
    pub fn get_extension<'js>(
        &self,
        c: Ctx<'js>,
        name: rquickjs::String<'js>,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::GET_SUPPORTED_EXTENSIONS);
        let nc = name.to_cstring()?;
        if !GL_EXT_LIST.contains(&nc.as_str()) {
            return Ok(Value::new_null(c));
        }
        let o = Object::new(c.clone())?;
        match nc.as_str() {
            "WEBGL_debug_renderer_info" => {
                o.set("UNMASKED_VENDOR_WEBGL", 37445i32)?;
                o.set("UNMASKED_RENDERER_WEBGL", 37446i32)?;
            }
            "EXT_texture_filter_anisotropic" => {
                o.set("MAX_TEXTURE_MAX_ANISOTROPY_EXT", 16_384i32)?;
                o.set("TEXTURE_MAX_ANISOTROPY_EXT", 34046i32)?;
            }
            "WEBGL_lose_context" => {
                let lose = Function::new(c.clone(), |c2: Ctx<'js>| -> Value<'_> {
                    Value::new_undefined(c2)
                })?;
                o.set("loseContext", lose)?;
            }
            _ => {}
        }
        Ok(o.into_value())
    }

    #[qjs(rename = "getSupportedExtensions")]
    pub fn get_supported_extensions<'js>(&self, c: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::GET_SUPPORTED_EXTENSIONS);
        let arr = rquickjs::Array::new(c.clone())?;
        for (i, e) in GL_EXT_LIST.iter().enumerate() {
            arr.set(i, *e)?;
        }
        Ok(arr.into_value())
    }

    #[qjs(rename = "getError")]
    pub fn get_error(&self) -> i32 {
        0
    }

    #[qjs(rename = "getContextAttributes")]
    pub fn get_context_attributes<'js>(&self, c: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::GET_CONTEXT_ATTRIBUTES);
        let o = Object::new(c.clone())?;
        o.set("alpha", true)?;
        o.set("antialias", true)?;
        o.set("depth", true)?;
        o.set("failIfMajorPerformanceCaveat", false)?;
        o.set("powerPreference", "default")?;
        o.set("premultipliedAlpha", true)?;
        o.set("preserveDrawingBuffer", false)?;
        o.set("stencil", false)?;
        o.set("desynchronized", false)?;
        Ok(o.into_value())
    }

    #[qjs(rename = "getShaderPrecisionFormat")]
    pub fn get_shader_precision_format<'js>(&self, c: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::GET_PARAMETER);
        let o = Object::new(c.clone())?;
        o.set("rangeMin", 127i32)?;
        o.set("rangeMax", 127i32)?;
        o.set("precision", 23i32)?;
        Ok(o.into_value())
    }

    #[qjs(rename = "compileShader")]
    pub fn compile_shader(&self) -> bool {
        true
    }

    #[qjs(rename = "getShaderParameter")]
    pub fn get_shader_parameter(&self) -> bool {
        true
    }

    #[qjs(rename = "getProgramParameter")]
    pub fn get_program_parameter(&self) -> bool {
        true
    }

    #[qjs(rename = "linkProgram")]
    pub fn link_program(&self) -> bool {
        true
    }

    #[qjs(rename = "createShader")]
    pub fn create_shader<'js>(&self, c: Ctx<'js>) -> Value<'js> {
        gl_object(&c)
    }

    #[qjs(rename = "createProgram")]
    pub fn create_program<'js>(&self, c: Ctx<'js>) -> Value<'js> {
        gl_object(&c)
    }

    #[qjs(rename = "createTexture")]
    pub fn create_texture<'js>(&self, c: Ctx<'js>) -> Value<'js> {
        gl_object(&c)
    }

    #[qjs(rename = "createBuffer")]
    pub fn create_buffer<'js>(&self, c: Ctx<'js>) -> Value<'js> {
        gl_object(&c)
    }

    #[qjs(rename = "createFramebuffer")]
    pub fn create_framebuffer<'js>(&self, c: Ctx<'js>) -> Value<'js> {
        gl_object(&c)
    }

    #[qjs(rename = "shaderSource")]
    pub fn shader_source(&self) {}

    #[qjs(rename = "attachShader")]
    pub fn attach_shader(&self) {}

    #[qjs(rename = "useProgram")]
    pub fn use_program(&self) {}

    #[qjs(rename = "bindBuffer")]
    pub fn bind_buffer(&self) {}

    #[qjs(rename = "bindTexture")]
    pub fn bind_texture(&self) {}

    #[qjs(rename = "viewport")]
    pub fn viewport(&self) {}

    #[qjs(rename = "clear")]
    pub fn clear(&self) {}

    #[qjs(rename = "readPixels")]
    pub fn read_pixels<'js>(
        &self,
        c: Ctx<'js>,
        _x: i32,
        _y: i32,
        w: i32,
        h: i32,
        _format: i32,
        _ty: i32,
        pixels: Value<'js>,
    ) -> rquickjs::Result<Value<'js>> {
        touch::touch_log_record(ApiKey::READ_PIXELS);
        if w <= 0 || h <= 0 || w > READBACK_MAX as i32 || h > READBACK_MAX as i32 {
            return Ok(Value::new_undefined(c.clone()));
        }
        let len = (w as usize) * (h as usize) * 4;
        pay_cost(CANVAS_OP_GET_IMAGE_DATA);
        let epoch = canvas_epoch(self.canvas_id);
        let draw_hash = ctx_slot(&c, &CTX2D, self.canvas_id)
            .and_then(|o| Class::<CanvasRenderingContext2D>::from_object(&o))
            .map(|class| class.borrow().st.borrow().draw_hash(epoch))
            .unwrap_or(core_utils::profile::BLANK_HASH);
        let seed = mix_ctx(seed_for(self.canvas_id), draw_hash);
        if let Some(bytes) = (unsafe {
            crate::webidl::ta_bytes_mut(&pixels).or_else(|| crate::webidl::ab_bytes_mut(&pixels))
        }) {
            if bytes.len() >= len {
                payload_gen::fill_pixels(&mut bytes[..len], w as u32, seed, draw_hash);
            }
        }
        Ok(Value::new_undefined(c))
    }
}
