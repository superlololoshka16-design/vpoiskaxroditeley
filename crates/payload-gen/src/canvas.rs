use compact_str::CompactString;
use core::f64::consts::{FRAC_PI_2, TAU};

use core_utils::BytesExt as _;
use core_utils::rng::{GOLDEN, U64Ext as _};
use core_utils::b64_encoded_len;
use core_utils::{adler32_feed, crc32_feed};
use crate::font::FontKind;
use tiny_skia::{FillRule, Paint, Path, PathBuilder, PixmapMut, Stroke, Transform};


pub const CANVAS_OP_FILL_RECT: u8 = 0;
pub const CANVAS_OP_FILL_TEXT: u8 = 1;
pub const CANVAS_OP_GET_IMAGE_DATA: u8 = 2;
pub const CANVAS_OP_TO_URL: u8 = 3;
pub const CANVAS_OP_MEASURE: u8 = 4;

pub const CANVAS_MAX_DIM: u32 = 32767;



#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

const BLACK: Rgba8 = Rgba8 {
    r: 0,
    g: 0,
    b: 0,
    a: 255,
};

#[inline]
fn hex_val(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

#[inline]
fn hex_pair(b: &[u8], i: usize) -> u8 {
    hex_val(b[i]) * 16 + hex_val(b[i + 1])
}

fn parse_hex_color(hex: &str) -> Rgba8 {
    let b = hex.as_bytes();
    if !b.iter().all(|c| c.is_ascii_hexdigit()) {
        return BLACK;
    }
    let d = |i: usize| hex_val(b[i]);
    match b.len() {
        3 => Rgba8 {
            r: d(0) * 17,
            g: d(1) * 17,
            b: d(2) * 17,
            a: 255,
        },
        4 => Rgba8 {
            r: d(0) * 17,
            g: d(1) * 17,
            b: d(2) * 17,
            a: d(3) * 17,
        },
        6 => Rgba8 {
            r: hex_pair(b, 0),
            g: hex_pair(b, 2),
            b: hex_pair(b, 4),
            a: 255,
        },
        8 => Rgba8 {
            r: hex_pair(b, 0),
            g: hex_pair(b, 2),
            b: hex_pair(b, 4),
            a: hex_pair(b, 6),
        },
        _ => BLACK,
    }
}

fn parse_fn_color(low: &str) -> Option<Rgba8> {
    let (body, n) = if let Some(rest) = low.strip_prefix("rgba(") {
        (rest, 4)
    } else if let Some(rest) = low.strip_prefix("rgb(") {
        (rest, 3)
    } else {
        return None;
    };
    let mut it = body.strip_suffix(')')?.split(',').map(str::trim);
    let chan = |s: &str| -> Option<u8> {
        s.parse::<f64>()
            .ok()
            .map(|v| v.clamp(0.0, 255.0).round() as u8)
    };
    let r = chan(it.next()?)?;
    let g = chan(it.next()?)?;
    let b = chan(it.next()?)?;
    let a = if n == 4 {
        (it.next()?.parse::<f64>().ok()?.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        255
    };
    if it.next().is_some() {
        return None;
    }
    Some(Rgba8 { r, g, b, a })
}

fn named_color(low: &str) -> Rgba8 {
    let (r, g, b, a): (u8, u8, u8, u8) = match low {
        "transparent" => (0, 0, 0, 0),
        "black" => (0, 0, 0, 255),
        "white" => (255, 255, 255, 255),
        "red" => (255, 0, 0, 255),
        "green" => (0, 128, 0, 255),
        "blue" => (0, 0, 255, 255),
        "yellow" => (255, 255, 0, 255),
        "cyan" | "aqua" => (0, 255, 255, 255),
        "magenta" | "fuchsia" => (255, 0, 255, 255),
        "gray" | "grey" => (128, 128, 128, 255),
        "silver" => (192, 192, 192, 255),
        "maroon" => (128, 0, 0, 255),
        "olive" => (128, 128, 0, 255),
        "lime" => (0, 255, 0, 255),
        "teal" => (0, 128, 128, 255),
        "navy" => (0, 0, 128, 255),
        "purple" => (128, 0, 128, 255),
        "orange" => (255, 165, 0, 255),
        "pink" => (255, 192, 203, 255),
        "brown" => (165, 42, 42, 255),
        "beige" => (245, 245, 220, 255),
        "gold" => (255, 215, 0, 255),
        "indigo" => (75, 0, 130, 255),
        "violet" => (238, 130, 238, 255),
        "salmon" => (250, 128, 114, 255),
        "tan" => (210, 180, 140, 255),
        "turquoise" => (64, 224, 208, 255),
        "coral" => (255, 127, 80, 255),
        "crimson" => (220, 20, 60, 255),
        "darkgray" | "darkgrey" => (169, 169, 169, 255),
        "lightgray" | "lightgrey" => (211, 211, 211, 255),
        _ => return BLACK,
    };
    Rgba8 { r, g, b, a }
}

pub fn parse_color(s: &str) -> Rgba8 {
    let t = s.trim();
    if t.is_empty() {
        return BLACK;
    }
    if let Some(hex) = t.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    let mut buf = [0u8; 32];
    if t.len() <= buf.len() {
        buf[..t.len()].copy_from_slice(t.as_bytes());
        for b in buf[..t.len()].iter_mut() {
            *b |= 0x20 * u8::from(b.is_ascii_uppercase());
        }
        let low = unsafe { std::str::from_utf8_unchecked(&buf[..t.len()]) };
        return parse_fn_color(low).unwrap_or_else(|| named_color(low));
    }
    let owned = core_utils::ascii_lower_compact(t);
    let low = owned.as_str();
    parse_fn_color(low).unwrap_or_else(|| named_color(low))
}


#[derive(Clone)]
enum PathSeg {
    Move(f64, f64),
    Line(f64, f64),
    Quad(f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

#[derive(Clone, Copy)]
enum Draw {
    Fill(Rgba8),
    Stroke(Rgba8, f64),
}

#[derive(Clone)]
enum Cmd {
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        draw: Draw,
    },
    Text {
        text: CompactString,
        x: f64,
        y: f64,
        px: f64,
        draw: Draw,
    },
    Path {
        seg_start: u32,
        seg_len: u32,
        draw: Draw,
    },
    Clear {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
}

#[inline]
fn fin<const N: usize>(v: [f64; N]) -> bool {
    v.into_iter().all(f64::is_finite)
}

const MAX_RENDER_PIXELS: u64 = 1 << 26;

pub fn render_dims(w: u32, h: u32) -> (u32, u32) {
    let cw = w.clamp(1, CANVAS_MAX_DIM);
    let ch = h.clamp(1, CANVAS_MAX_DIM);
    if u64::from(cw) * u64::from(ch) > MAX_RENDER_PIXELS {
        return (1, 1);
    }
    (cw, ch)
}

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(x as f32, y as f32);
    pb.line_to((x + w) as f32, y as f32);
    pb.line_to((x + w) as f32, (y + h) as f32);
    pb.line_to(x as f32, (y + h) as f32);
    pb.close();
    pb.finish()
}

fn build_path(segs: &[PathSeg]) -> Option<Path> {
    if segs.is_empty() {
        return None;
    }
    let mut pb = PathBuilder::with_capacity(segs.len() + 1, segs.len() * 3 + 1);
    for seg in segs {
        match *seg {
            PathSeg::Move(x, y) => pb.move_to(x as f32, y as f32),
            PathSeg::Line(x, y) => pb.line_to(x as f32, y as f32),
            PathSeg::Quad(cx, cy, x, y) => pb.quad_to(cx as f32, cy as f32, x as f32, y as f32),
            PathSeg::Cubic(c1x, c1y, c2x, c2y, x, y) => {
                pb.cubic_to(
                    c1x as f32, c1y as f32, c2x as f32, c2y as f32, x as f32, y as f32,
                );
            }
            PathSeg::Close => pb.close(),
        }
    }
    pb.finish()
}




fn push_text(pb: &mut PathBuilder, kind: FontKind, text: &str, x: f64, y: f64, px: f64) {
    let mut cx = x;
    for ch in text.chars() {
        let cp = ch as u32;
        if let Some(cmds) = crate::font::glyph_contours(kind, cp, px) {
            for cmd in cmds {
                match cmd {
                    crate::font::GlyphCmd::MoveTo(gx, gy) => {
                        pb.move_to((cx + f64::from(gx)) as f32, (y - f64::from(gy)) as f32);
                    }
                    crate::font::GlyphCmd::LineTo(gx, gy) => {
                        pb.line_to((cx + f64::from(gx)) as f32, (y - f64::from(gy)) as f32);
                    }
                    crate::font::GlyphCmd::QuadTo(qx, qy, ex, ey) => {
                        pb.quad_to(
                            (cx + f64::from(qx)) as f32,
                            (y - f64::from(qy)) as f32,
                            (cx + f64::from(ex)) as f32,
                            (y - f64::from(ey)) as f32,
                        );
                    }
                    crate::font::GlyphCmd::Close => {
                        pb.close();
                    }
                }
            }
        }
        cx += crate::font::advance_of_cp_pub(kind, cp, px);
    }
}

fn paint_of(color: Rgba8) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.r, color.g, color.b, color.a);
    paint.anti_alias = true;
    paint
}

fn fill_pm(pm: &mut PixmapMut<'_>, path: &Path, color: Rgba8, ts: Transform) {
    if color.a == 0 {
        return;
    }
    pm.fill_path(path, &paint_of(color), FillRule::Winding, ts, None);
}

fn stroke_pm(pm: &mut PixmapMut<'_>, path: &Path, color: Rgba8, lw: f64, ts: Transform) {
    if color.a == 0 {
        return;
    }
    let stroke = Stroke {
        width: lw as f32,
        ..Stroke::default()
    };
    pm.stroke_path(path, &paint_of(color), &stroke, ts, None);
}

fn clear_region(pm: &mut PixmapMut<'_>, x: f64, y: f64, w: f64, h: f64) {
    if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return;
    }
    let pw = pm.width() as i64;
    let ph = pm.height() as i64;
    let x0 = (x.floor() as i64).clamp(0, pw);
    let y0 = (y.floor() as i64).clamp(0, ph);
    let x1 = ((x + w).ceil() as i64).clamp(x0, pw);
    let y1 = ((y + h).ceil() as i64).clamp(y0, ph);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let stride = pw as usize * 4;
    let data = pm.data_mut();
    for row in y0 as usize..y1 as usize {
        data[row * stride + x0 as usize * 4..row * stride + x1 as usize * 4].fill(0);
    }
}

impl Cmd {
    fn apply(&self, segs: &[PathSeg], pm: &mut PixmapMut<'_>, _seed: u64, ts: Transform) {
        let (path, draw) = match self {
            Cmd::Clear { x, y, w, h } => return clear_region(pm, *x, *y, *w, *h),
            Cmd::Rect { x, y, w, h, draw } => (rect_path(*x, *y, *w, *h), *draw),
            Cmd::Text { text, x, y, px, draw } => {
                let mut pb = PathBuilder::new();
                push_text(&mut pb, self.kind, text, *x, *y, *px);
                (pb.finish(), *draw)
            }
            Cmd::Path {
                seg_start,
                seg_len,
                draw,
            } => (
                build_path(&segs[*seg_start as usize..(*seg_start + *seg_len) as usize]),
                *draw,
            ),
        };
        if let Some(path) = path {
            match draw {
                Draw::Fill(c) => fill_pm(pm, &path, c, ts),
                Draw::Stroke(c, lw) => stroke_pm(pm, &path, c, lw, ts),
            }
        }
    }
}

#[inline]
fn farble_hash(seed: u64, x: u64, y: u64) -> u64 {
    (seed ^ x.wrapping_mul(GOLDEN) ^ y.wrapping_mul(core_utils::rng::seeds::SALT_FARBLE)).mix()
}

fn farble_pixels_offset(out: &mut [u8], w: u32, ox: i64, oy: i64, seed: u64) {
    let w = w as usize;
    if w == 0 {
        return;
    }
    let row_bytes = w * 4;
    for (y, row) in out.chunks_exact_mut(row_bytes).enumerate() {
        let ay = (oy + y as i64) as u64;
        for (x, px) in row.chunks_exact_mut(4).enumerate() {
            unpremul_px(px);
            let z = farble_hash(seed, (ox + x as i64) as u64, ay);
            let gate = u8::from(px[3] != 0) & u8::from(z % 20 == 0);
            px[0] ^= gate & ((z >> 8) & 1) as u8;
            px[1] ^= gate & ((z >> 9) & 1) as u8;
            px[2] ^= gate & ((z >> 10) & 1) as u8;
        }
    }
}

const fn unpremul_lut() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut d = 1usize;
    while d < 256 {
        t[d] = ((65280 + d / 2) / d) as u32;
        d += 1;
    }
    t[0] = t[1];
    t
}

static UNPREMUL_INV: [u32; 256] = unpremul_lut();


#[inline(always)]
fn unpremul_px(px: &mut [u8]) {
    let inv = UNPREMUL_INV[px[3] as usize];
    px[0] = (((px[0] as u32) * inv) >> 8).min(255) as u8;
    px[1] = (((px[1] as u32) * inv) >> 8).min(255) as u8;
    px[2] = (((px[2] as u32) * inv) >> 8).min(255) as u8;
}
#[derive(Clone)]
pub struct CanvasRaster {
    seed: u64,
    kind: FontKind,
    fill: Rgba8,
    stroke: Rgba8,
    line_width: f64,
    alpha: f64,
    cmds: Vec<Cmd>,
    segs: Vec<PathSeg>,
    seg_base: u32,
    pen: Option<(f64, f64)>,
    start: Option<(f64, f64)>,
}

impl CanvasRaster {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            kind: FontKind::Linux,
            fill: BLACK,
            stroke: BLACK,
            line_width: 1.0,
            alpha: 1.0,
            cmds: Vec::new(),
            segs: Vec::new(),
            seg_base: 0,
            pen: None,
            start: None,
        }
    }

    pub fn set_font_kind(&mut self, kind: FontKind) {
        self.kind = kind;
    }

    pub fn resize(&mut self) {
        self.fill = BLACK;
        self.stroke = BLACK;
        self.line_width = 1.0;
        self.alpha = 1.0;
        self.cmds.clear();
        self.segs.clear();
        self.seg_base = 0;
        self.pen = None;
        self.start = None;
    }

    pub fn set_fill(&mut self, c: Rgba8) {
        self.fill = c;
    }

    pub fn set_stroke(&mut self, c: Rgba8) {
        self.stroke = c;
    }

    pub fn set_line_width(&mut self, w: f64) {
        if w.is_finite() && w > 0.0 {
            self.line_width = w;
        }
    }

    pub fn set_alpha(&mut self, a: f64) {
        if a.is_finite() && (0.0..=1.0).contains(&a) {
            self.alpha = a;
        }
    }

    fn tinted(&self, c: Rgba8) -> Rgba8 {
        if self.alpha >= 1.0 {
            return c;
        }
        let a = (f64::from(c.a) * self.alpha).round().clamp(0.0, 255.0) as u8;
        Rgba8 { a, ..c }
    }

    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        if fin([x, y, w, h]) {
            let draw = Draw::Fill(self.tinted(self.fill));
            self.cmds.push(Cmd::Rect { x, y, w, h, draw });
        }
    }

    pub fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        if fin([x, y, w, h]) {
            let draw = Draw::Stroke(self.tinted(self.stroke), self.line_width);
            self.cmds.push(Cmd::Rect { x, y, w, h, draw });
        }
    }

    pub fn clear_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        if fin([x, y, w, h]) {
            self.cmds.push(Cmd::Clear { x, y, w, h });
        }
    }

    pub fn fill_text(&mut self, text: &str, x: f64, y: f64, px: f64) {
        self.push_text_cmd(text, x, y, px, Draw::Fill(self.tinted(self.fill)));
    }

    pub fn stroke_text(&mut self, text: &str, x: f64, y: f64, px: f64) {
        self.push_text_cmd(
            text,
            x,
            y,
            px,
            Draw::Stroke(self.tinted(self.stroke), self.line_width),
        );
    }

    fn push_text_cmd(&mut self, text: &str, x: f64, y: f64, px: f64, draw: Draw) {
        if fin([x, y]) {
            self.cmds.push(Cmd::Text {
                text: CompactString::new(text),
                x,
                y,
                px,
                draw,
            });
        }
    }

    pub fn begin_path(&mut self) {
        self.segs.truncate(self.seg_base as usize);
        self.pen = None;
        self.start = None;
    }

    pub fn close_path(&mut self) {
        if self.start.is_some() {
            self.segs.push(PathSeg::Close);
            self.pen = self.start;
        }
    }

    pub fn move_to(&mut self, x: f64, y: f64) {
        if fin([x, y]) {
            self.segs.push(PathSeg::Move(x, y));
            self.pen = Some((x, y));
            self.start = Some((x, y));
        }
    }

    pub fn line_to(&mut self, x: f64, y: f64) {
        if !fin([x, y]) {
            return;
        }
        if self.pen.is_none() {
            self.segs.push(PathSeg::Move(x, y));
            self.start = Some((x, y));
        } else {
            self.segs.push(PathSeg::Line(x, y));
        }
        self.pen = Some((x, y));
    }

    pub fn arc(&mut self, x: f64, y: f64, r: f64, a0: f64, a1: f64) {
        self.push_ellipse(x, y, r, r, 0.0, a0, a1);
    }

    pub fn ellipse(&mut self, x: f64, y: f64, rx: f64, ry: f64, rot: f64, a0: f64, a1: f64) {
        self.push_ellipse(x, y, rx, ry, rot, a0, a1);
    }

    fn push_ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, rot: f64, a0: f64, a1: f64) {
        if !fin([cx, cy, rx, ry, rot, a0, a1]) || rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let diff = a1 - a0;
        let sweep = if diff >= TAU {
            TAU
        } else {
            diff.rem_euclid(TAU)
        };
        let sx = cx + rx * a0.cos();
        let sy = cy + ry * a0.sin();
        if self.pen.is_some() {
            self.segs.push(PathSeg::Line(sx, sy));
        } else {
            self.segs.push(PathSeg::Move(sx, sy));
        }
        if self.start.is_none() {
            self.start = Some((sx, sy));
        }
        self.pen = Some((sx, sy));
        if sweep <= 0.0 {
            return;
        }
        let n = (sweep / FRAC_PI_2).ceil().max(1.0) as usize;
        let d = sweep / n as f64;
        let k = (4.0 / 3.0) * (d / 4.0).tan();
        let rc = rot.cos();
        let rs = rot.sin();
        for i in 0..n {
            let t0 = a0 + d * i as f64;
            let t1 = t0 + d;
            let (c0, s0) = (t0.cos(), t0.sin());
            let (c1, s1) = (t1.cos(), t1.sin());
            let p0x = cx + rx * c0;
            let p0y = cy + ry * s0;
            let p3x = cx + rx * c1;
            let p3y = cy + ry * s1;
            let d0x = -rx * s0;
            let d0y = ry * c0;
            let d1x = -rx * s1;
            let d1y = ry * c1;
            let c1x = p0x + k * (d0x * rc - d0y * rs);
            let c1y = p0y + k * (d0x * rs + d0y * rc);
            let c2x = p3x - k * (d1x * rc - d1y * rs);
            let c2y = p3y - k * (d1x * rs + d1y * rc);
            self.segs.push(PathSeg::Cubic(c1x, c1y, c2x, c2y, p3x, p3y));
            self.pen = Some((p3x, p3y));
        }
    }

    pub fn bezier_curve_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) {
        if !fin([c1x, c1y, c2x, c2y, x, y]) {
            return;
        }
        if self.pen.is_none() {
            self.segs.push(PathSeg::Move(c1x, c1y));
            self.start = Some((c1x, c1y));
            self.pen = Some((c1x, c1y));
        }
        self.segs.push(PathSeg::Cubic(c1x, c1y, c2x, c2y, x, y));
        self.pen = Some((x, y));
    }

    pub fn quadratic_curve_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
        if !fin([cx, cy, x, y]) {
            return;
        }
        if self.pen.is_none() {
            self.segs.push(PathSeg::Move(cx, cy));
            self.start = Some((cx, cy));
            self.pen = Some((cx, cy));
        }
        self.segs.push(PathSeg::Quad(cx, cy, x, y));
        self.pen = Some((x, y));
    }

    pub fn fill(&mut self) {
        self.push_path_cmd(Draw::Fill(self.tinted(self.fill)));
    }

    pub fn stroke(&mut self) {
        self.push_path_cmd(Draw::Stroke(self.tinted(self.stroke), self.line_width));
    }

    fn push_path_cmd(&mut self, draw: Draw) {
        if !self.segs.is_empty() {
            let seg_start = self.seg_base;
            let seg_len = self.segs.len() as u32 - seg_start;
            self.cmds.push(Cmd::Path {
                seg_start,
                seg_len,
                draw,
            });
            self.seg_base = self.segs.len() as u32;
        }
    }

    pub fn render(&self, w: u32, h: u32) -> Vec<u8> {
        let (w, h) = render_dims(w, h);
        let mut out = vec![0u8; w as usize * h as usize * 4];
        self.render_into(&mut out, w, h, 0.0, 0.0, 0);
        out
    }

    pub fn render_into(&self, out: &mut [u8], w: u32, h: u32, dx: f64, dy: f64, mix: u64) {
        let seed = self.seed ^ mix;
        let ts = if dx == 0.0 && dy == 0.0 {
            Transform::identity()
        } else {
            Transform::from_translate(-dx as f32, -dy as f32)
        };
        if let Some(mut pm) = PixmapMut::from_bytes(out, w, h) {
            for cmd in &self.cmds {
                cmd.apply(&self.segs, &mut pm, seed, ts);
            }
            let data = pm.data_mut();
            farble_pixels_offset(data, w, dx as i64, dy as i64, seed);
        } else {
            out.fill(0);
        }
    }
}

const PNG_RAW_CAP: u64 = 64 * 1024 * 1024;

fn png_dims(w: u32, h: u32) -> (u32, u32) {
    let cw = w.clamp(1, CANVAS_MAX_DIM);
    let ch = h.clamp(1, CANVAS_MAX_DIM);
    if u64::from(cw) * u64::from(ch) * 4 + u64::from(ch) > PNG_RAW_CAP {
        return (1, 1);
    }
    (cw, ch)
}

#[inline]
fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

pub fn png_bytes_pixels(w: u32, h: u32, pixels: &[u8]) -> Vec<u8> {
    let (cw, ch) = png_dims(w, h);
    let stride = 1 + cw as usize * 4;
    let raw_len = stride * ch as usize;
    let blocks = raw_len.div_ceil(65535);
    let zlib_len = 2 + raw_len + blocks * 5 + 4;
    let mut png = Vec::with_capacity(8 + 25 + 12 + zlib_len + 12);
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&cw.to_be_bytes());
    ihdr[4..8].copy_from_slice(&ch.to_be_bytes());
    ihdr[8..13].copy_from_slice(&[8, 6, 0, 0, 0]);
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&ihdr);
    push_u32(&mut png, !crc32_feed(crc32_feed(0xFFFF_FFFF, b"IHDR"), &ihdr));
    png.extend_from_slice(&(zlib_len as u32).to_be_bytes());
    png.extend_from_slice(b"IDAT");
    let idat_head = png.len();

    png.extend_from_slice(&[0x78, 0x01]);
    let mut adler = 1u32;
    let row_bytes = cw as usize * 4;
    let mut block_pos = 0usize;
    let mut block_left = 0usize;
    let mut block_data = 0usize;
    for y in 0..ch as usize {
        let src = (y * row_bytes).min(pixels.len());
        let take = (pixels.len() - src).min(row_bytes) & !3;
        let row = &pixels[src..src + take];
        let mut written = 0usize;
        let mut filter_done = false;
        loop {
            if block_left == 0 {
                if block_pos != 0 {
                    let len16 = block_data as u16;
                    png[block_pos + 1..block_pos + 3].copy_from_slice(&len16.to_le_bytes());
                    png[block_pos + 3..block_pos + 5]
                        .copy_from_slice(&(!len16).to_le_bytes());
                }
                block_pos = png.len();
                png.extend_from_slice(&[0u8, 0, 0, 0, 0]);
                block_left = 65535;
                block_data = 0;
            }
            if !filter_done {
                png.push(0);
                adler = core_utils::adler32_zeros(adler, 1);
                block_left -= 1;
                block_data += 1;
                filter_done = true;
            }
            if written == row_bytes {
                break;
            }
            let chunk = (row_bytes - written).min(block_left);
            let have = row.len().saturating_sub(written);
            let n = chunk.min(have);
            if n > 0 {
                let s = &row[written..written + n];
                png.extend_from_slice(s);
                adler = adler32_feed(adler, s);
            }
            if chunk > n {
                png.resize(png.len() + (chunk - n), 0);
                adler = core_utils::adler32_zeros(adler, chunk - n);
            }
            written += chunk;
            block_left -= chunk;
            block_data += chunk;
        }
    }
    png[block_pos] = 1;
    let take16 = block_data as u16;
    png[block_pos + 1..block_pos + 3].copy_from_slice(&take16.to_le_bytes());
    png[block_pos + 3..block_pos + 5].copy_from_slice(&(!take16).to_le_bytes());
    push_u32(&mut png, adler);
    let idat_crc = crc32_feed(
        crc32_feed(0xFFFF_FFFF, b"IDAT"),
        &png[idat_head..png.len()],
    );
    push_u32(&mut png, !idat_crc);
    png.extend_from_slice(&0u32.to_be_bytes());
    png.extend_from_slice(b"IEND");
    push_u32(&mut png, !crc32_feed(crc32_feed(0xFFFF_FFFF, b"IEND"), &[]));
    png
}

pub fn png_data_url_pixels(w: u32, h: u32, pixels: &[u8]) -> String {
    let png = png_bytes_pixels(w, h, pixels);
    let cap = b64_encoded_len(png.len());
    let mut buf: Vec<u8> = Vec::with_capacity(22 + cap);
    buf.extend_from_slice(b"data:image/png;base64,");
    buf.resize(22 + cap, 0);
    let mut n = 22 + cap;
    if let Ok(k) = png.b64_encode_into(&mut buf[22..]) {
        n = 22 + k;
    }
    buf.truncate(n);

    String::from_utf8(buf).expect("base64 is ascii")
}


pub use core_utils::bench::bench_jitter;
pub use core_utils::profile::canvas_time_cost_us;

const AUDIO_ROOT: u64 = core_utils::rng::seeds::SALT_AUDIO_ROOT;
const S_AUDIO_CHANNEL: u64 = core_utils::rng::seeds::SALT_AUDIO_CHANNEL;
const AUDIO_FP_LO: f64 = 124.0;
const AUDIO_FP_HI: f64 = 124.08;

pub fn audio_fp(seed: u64) -> f64 {
    core_utils::Identity::new(seed)
        .at(AUDIO_ROOT)
        .at(S_AUDIO_CHANNEL)
        .f64_in(AUDIO_FP_LO, AUDIO_FP_HI)
}

pub const WEBGL1_VERSION: &str = "WebGL 1.0 (OpenGL ES 2.0 Chromium)";
pub const WEBGL1_GLSL: &str = "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)";

pub const WEBGL_INT_PARAMS: [(u8, i32); 16] = [
    (0, 16384),
    (1, 16384),
    (2, 16),
    (3, 4095),
    (4, 30),
    (5, 1024),
    (6, 32),
    (7, 16),
    (8, 16),
    (9, 16384),
    (10, 16384),
    (11, 8),
    (12, 24),
    (13, 4),
    (14, 2048),
    (15, 2048),
];

const WEBGL_ROOT: u64 = core_utils::rng::seeds::SALT_WEBGL_ROOT;

#[inline]
pub fn webgl_int_param(seed: u64, slot: u8) -> i32 {
    let (pid, base) = WEBGL_INT_PARAMS[(slot as usize) % WEBGL_INT_PARAMS.len()];
    let id = core_utils::Identity::new(seed)
        .at(WEBGL_ROOT)
        .at(u64::from(pid));
    match pid {
        11 | 12 => base,
        3 => base - (id.f64_unit() * 8.0) as i32,
        _ => {
            if id.chance(0.12) {
                base / 2
            } else {
                base
            }
        }
    }
}

const WEBGL_RANGES: [(f64, f64); 8] = [
    (1.0, 2048.0),
    (1.0, 1024.0),
    (3379.0, 16384.0),
    (0.0, 1.0),
    (8.0, 16.0),
    (64.0, 4096.0),
    (2.0, 16.0),
    (0.1, 1.0),
];

pub const READBACK_MAX: usize = 4096;

pub const WEBGL2_VERSION: &str = "WebGL 2.0 (OpenGL ES 3.0 Chromium)";
pub const WEBGL2_GLSL: &str = "WebGL GLSL ES 3.0 (OpenGL ES GLSL ES 3.0 Chromium)";

pub fn webgl_param(seed: u64, slot: u8) -> f64 {
    let (lo, hi) = WEBGL_RANGES[(slot as usize) % WEBGL_RANGES.len()];
    core_utils::Identity::new(seed)
        .at(core_utils::rng::seeds::SALT_WEBGL_PARAM)
        .at(u64::from(slot))
        .f64_in(lo, hi)
}

pub fn pixel_at(seed: u64, draw_hash: u64, x: u32, y: u32, c: u32) -> u8 {
    if c == 3 {
        return 255;
    }
    farble_hash(
        seed ^ draw_hash.rotate_left(13) ^ (c as u64).wrapping_mul(core_utils::rng::seeds::SALT_PIXEL_CHAN),
        x as u64,
        y as u64,
    ) as u8
}

pub fn fill_pixels(buf: &mut [u8], w: u32, seed: u64, draw_hash: u64) {
    let w = w.max(1) as usize;
    let mut x = 0usize;
    let mut y = 0usize;
    for px in buf.chunks_exact_mut(4) {
        px[0] = pixel_at(seed, draw_hash, x as u32, y as u32, 0);
        px[1] = pixel_at(seed, draw_hash, x as u32, y as u32, 1);
        px[2] = pixel_at(seed, draw_hash, x as u32, y as u32, 2);
        px[3] = 255;
        x += 1;
        if x == w {
            x = 0;
            y += 1;
        }
    }
}

pub const GENERIC_FONTS: &[&str] = &[
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
    "-apple-system",
    "BlinkMacSystemFont",
];

pub const WIN_FONTS: &[&str] = &[
    "Arial",
    "Arial Black",
    "Bahnschrift",
    "Calibri",
    "Cambria",
    "Candara",
    "Comic Sans MS",
    "Consolas",
    "Constantia",
    "Corbel",
    "Courier New",
    "Ebrima",
    "Franklin Gothic Medium",
    "Gabriola",
    "Gadugi",
    "Georgia",
    "Impact",
    "Ink Free",
    "Javanese Text",
    "Leelawadee UI",
    "Lucida Console",
    "Lucida Sans Unicode",
    "Malgun Gothic",
    "Marlett",
    "Microsoft Himalaya",
    "Microsoft JhengHei",
    "Microsoft New Tai Lue",
    "Microsoft Sans Serif",
    "Microsoft Tai Le",
    "Microsoft YaHei",
    "MingLiU-ExtB",
    "Mongolian Baiti",
    "MS Gothic",
    "MV Boli",
    "Myanmar Text",
    "Nirmala UI",
    "Palatino Linotype",
    "Segoe MDL2 Assets",
    "Segoe Print",
    "Segoe Script",
    "Segoe UI",
    "Segoe UI Emoji",
    "Segoe UI Historic",
    "Segoe UI Symbol",
    "SimSun",
    "Sitka",
    "Sylfaen",
    "Symbol",
    "Tahoma",
    "Times New Roman",
    "Trebuchet MS",
    "Verdana",
    "Webdings",
    "Wingdings",
    "Yu Gothic",
];

pub const MAC_FONTS: &[&str] = &[
    "American Typewriter",
    "Andale Mono",
    "Apple Color Emoji",
    "Apple SD Gothic Neo",
    "Arial",
    "Arial Rounded MT Bold",
    "Avenir",
    "Avenir Next",
    "Baskerville",
    "Bodoni 72",
    "Bradley Hand",
    "Brush Script MT",
    "Chalkboard",
    "Chalkduster",
    "Charter",
    "Cochin",
    "Comic Sans MS",
    "Copperplate",
    "Courier",
    "Courier New",
    "DIN Alternate",
    "DIN Condensed",
    "Futura",
    "Geneva",
    "Georgia",
    "Gill Sans",
    "Helvetica",
    "Helvetica Neue",
    "Hoefler Text",
    "Impact",
    "Lucida Grande",
    "Luminari",
    "Marker Felt",
    "Menlo",
    "Mistral",
    "Monaco",
    "Noteworthy",
    "Optima",
    "Palatino",
    "Papyrus",
    "Perth",
    "Rockwell",
    "San Francisco",
    "Savoye LET",
    "SignPainter",
    "Skia",
    "Snell Roundhand",
    "Spectrum",
    "Stencil",
    "Times New Roman",
    "Trattatello",
    "Trebuchet MS",
    "Verdana",
    "Zapfino",
];

pub const LINUX_FONTS: &[&str] = &[
    "DejaVu Sans",
    "DejaVu Sans Mono",
    "DejaVu Serif",
    "Liberation Mono",
    "Liberation Sans",
    "Liberation Serif",
    "Noto Sans",
    "Noto Sans CJK SC",
    "Noto Serif",
    "Ubuntu",
    "Ubuntu Mono",
    "Cantarell",
    "Droid Sans",
    "FreeMono",
    "FreeSans",
    "FreeSerif",
    "Lato",
    "Open Sans",
    "Roboto",
    "Arial",
    "Times New Roman",
    "Courier New",
    "Helvetica",
    "Verdana",
    "Tahoma",
    "Georgia",
    "Comic Sans MS",
    "Impact",
    "Trebuchet MS",
];

pub fn platform_fonts(platform: &str) -> &'static [&'static str] {
    match platform {
        "Windows" => WIN_FONTS,
        "macOS" => MAC_FONTS,
        _ => LINUX_FONTS,
    }
}
