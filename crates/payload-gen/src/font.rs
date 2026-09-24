use smallvec::SmallVec;
use std::sync::LazyLock;

pub const TTF_DEJAVU: &[u8] = include_bytes!("../../../third_party/fonts/DejaVuSans.ttf");
pub const TTF_LIBERATION: &[u8] =
    include_bytes!("../../../third_party/fonts/LiberationSans-Regular.ttf");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontKind {
    Linux,
    Windows,
}

static FONT_LINUX: LazyLock<Option<TtfTables>> =
    LazyLock::new(|| parse_tables(TTF_DEJAVU));
static FONT_WINDOWS: LazyLock<Option<TtfTables>> =
    LazyLock::new(|| parse_tables(TTF_LIBERATION));

fn font_tables(kind: FontKind) -> Option<&'static TtfTables> {
    match kind {
        FontKind::Linux => FONT_LINUX.as_ref(),
        FontKind::Windows => FONT_WINDOWS.as_ref(),
    }
}

struct TtfTables {
    d: &'static [u8],
    units_per_em: u16,
    ascender: i16,
    descender: i16,
    cmap: (usize, usize),
    hmtx: (usize, usize),
    num_h: u16,
    glyf: (usize, usize),
    loca: Vec<u32>,
    em_scale: f64,
}

#[inline]
fn u16o(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([d[o], d[o + 1]])
}

#[inline]
fn i16o(d: &[u8], o: usize) -> i16 {
    i16::from_be_bytes([d[o], d[o + 1]])
}

#[inline]
fn u32o(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn parse_tables(d: &'static [u8]) -> Option<TtfTables> {
    if d.len() < 12 || d[..4] != [0, 1, 0, 0] {
        return None;
    }
    let num = u16o(d, 4) as usize;
    let mut cmap = (0, 0);
    let mut hmtx = (0, 0);
    let mut glyf = (0, 0);
    let mut head = (0, 0);
    let mut hhea = (0, 0);
    let mut loca = (0, 0);
    let mut maxp = (0, 0);
    let mut off = 12;
    for _ in 0..num {
        if off + 16 > d.len() {
            return None;
        }
        let t = &d[off..off + 4];
        let o = u32o(d, off + 8) as usize;
        let l = u32o(d, off + 12) as usize;
        match t {
            b"cmap" => cmap = (o, l),
            b"hmtx" => hmtx = (o, l),
            b"glyf" => glyf = (o, l),
            b"head" => head = (o, l),
            b"hhea" => hhea = (o, l),
            b"loca" => loca = (o, l),
            b"maxp" => maxp = (o, l),
            _ => {}
        }
        off += 16;
    }
    let units_per_em = u16o(d, head.0 + 18);
    if units_per_em == 0 {
        return None;
    }
    let loca_long = u16o(d, head.0 + 50) != 0;
    let n_glyphs = u16o(d, maxp.0 + 4) as usize;
    let num_h = u16o(d, hhea.0 + 34);
    let mut locs = Vec::with_capacity(n_glyphs + 1);
    if loca_long {
        for i in 0..=n_glyphs {
            locs.push(u32o(d, loca.0 + i * 4));
        }
    } else {
        for i in 0..=n_glyphs {
            locs.push(u16o(d, loca.0 + i * 2) as u32 * 2);
        }
    }
    let em_scale = 1.0 / f64::from(units_per_em);
    Some(TtfTables {
        d,
        units_per_em,
        ascender: i16o(d, hhea.0 + 4),
        descender: i16o(d, hhea.0 + 6),
        cmap,
        hmtx,
        num_h,
        glyf,
        loca: locs,
        em_scale,
    })
}


fn cmap4_lookup(t: &TtfTables, cp: u32) -> u16 {
    let (co, cl) = t.cmap;
    if cl < 4 {
        return 0;
    }
    let n = u16o(t.d, co + 2) as usize;
    let mut sub: Option<usize> = None;
    for i in 0..n {
        let pid = u16o(t.d, co + 4 + i * 8);
        let eid = u16o(t.d, co + 6 + i * 8);
        let o = u32o(t.d, co + 8 + i * 8) as usize;
        let fmt = u16o(t.d, co + o);
        if fmt == 4 && ((pid == 3 && eid == 1) || (pid == 0 && (eid == 3 || eid == 4))) {
            sub = Some(co + o);
            break;
        }
    }
    let Some(sub) = sub else { return 0 };
    let seg_x2 = u16o(t.d, sub + 6) as usize;
    let seg = seg_x2 / 2;
    let end_off = sub + 14;
    let starts = end_off + seg_x2 + 2;
    let deltas = starts + seg_x2;
    let ranges = deltas + seg_x2;
    for i in 0..seg {
        let end = u16o(t.d, end_off + i * 2) as u32;
        if cp <= end {
            let start = u16o(t.d, starts + i * 2) as u32;
            if cp < start {
                return 0;
            }
            let delta = i16o(t.d, deltas + i * 2);
            let rng = u16o(t.d, ranges + i * 2);
            if rng == 0 {
                return (u32::from(cp as u16) .wrapping_add(delta as u32) & 0xFFFF) as u16;
            }
            let gi_off = ranges + i * 2 + (cp - start) as usize;
            if gi_off + 2 > t.d.len() {
                return 0;
            }
            let g = u16o(t.d, gi_off);
            if g == 0 {
                return 0;
            }
            return (u32::from(g).wrapping_add(delta as u32) & 0xFFFF) as u16;
        }
    }
    0
}

fn advance_of(t: &TtfTables, g: u16) -> u16 {
    let (ho, _) = t.hmtx;
    let idx = if (g as usize) < t.num_h as usize {
        g as usize
    } else {
        t.num_h as usize - 1
    };
    u16o(t.d, ho + idx * 4)
}

#[inline]
pub fn advance_of_cp_pub(kind: FontKind, cp: u32, px: f64) -> f64 {
    let Some(t) = font_tables(kind) else { return px * 0.5 };
    advance_of_cp(t, cp, px)
}

fn advance_of_cp(t: &TtfTables, cp: u32, px: f64) -> f64 {
    let g = cmap4_lookup(t, cp);
    if g == 0 {
        return px * 0.5;
    }
    f64::from(advance_of(t, g)) * t.em_scale * px
}

pub fn advances(kind: FontKind, text: &str, px: f64) -> SmallVec<[f64; 32]> {
    let Some(t) = font_tables(kind) else {
        return text.chars().map(|_| px * 0.5).collect();
    };
    text.chars().map(|ch| advance_of_cp(t, ch as u32, px)).collect()
}

pub struct GlyphBox {
    pub x0: i16,
    pub y0: i16,
    pub x1: i16,
    pub y1: i16,
}

fn glyph_box(t: &TtfTables, cp: u32) -> Option<GlyphBox> {
    let g = cmap4_lookup(t, cp) as usize;
    if g == 0 || g + 1 >= t.loca.len() {
        return None;
    }
    let start = t.loca[g] as usize;
    let end = t.loca[g + 1] as usize;
    if end <= start || start + 10 > t.d.len() {
        return None;
    }
    let o = t.glyf.0 + start;
    let ncont = i16o(t.d, o);
    if ncont <= 0 {
        return None;
    }
    Some(GlyphBox {
        x0: i16o(t.d, o + 2),
        y0: i16o(t.d, o + 4),
        x1: i16o(t.d, o + 6),
        y1: i16o(t.d, o + 8),
    })
}

#[derive(Clone, Copy)]
pub struct FontMetrics {
    pub em_scale: f64,
    pub ascender: f64,
    pub descender: f64,
}

impl FontMetrics {
    #[inline]
    const fn fallback() -> Self {
        Self {
            em_scale: 1.0 / 2048.0,
            ascender: 0.8,
            descender: 0.2,
        }
    }

    #[inline]
    pub fn scaled(&self, px: f64) -> Self {
        Self {
            em_scale: self.em_scale * px,
            ascender: self.ascender * px,
            descender: self.descender * px,
        }
    }
}

#[inline]
pub fn font_metrics(kind: FontKind) -> FontMetrics {
    font_tables(kind).map_or(FontMetrics::fallback(), |t| FontMetrics {
        em_scale: t.em_scale,
        ascender: f64::from(t.ascender) * t.em_scale,
        descender: -f64::from(t.descender) * t.em_scale,
    })
}

#[derive(Clone, Copy)]
pub enum GlyphCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    Close,
}

static CONTOUR_CACHE: std::sync::LazyLock<
    scc::HashMap<(u8, u32), SmallVec<[GlyphCmd; 40]>>,
> = std::sync::LazyLock::new(scc::HashMap::new);

pub fn glyph_contours(kind: FontKind, cp: u32, px: f64) -> Option<SmallVec<[GlyphCmd; 40]>> {
    let t = font_tables(kind)?;
    let gid = u32::from(cmap4_lookup(t, cp));
    if gid == 0 {
        return None;
    }
    let mut px_scale = (t.em_scale * px) as f32;
    if px_scale == 0.0 || !px_scale.is_finite() {
        px_scale = 1.0;
    }
    let cache_key = (kind as u8, gid);
    if let Some(hit) = CONTOUR_CACHE.read_sync(&cache_key, |_, v| v.clone()) {
        if px_scale == 1.0 {
            return Some(hit);
        }
        let mut scaled = hit;
        for cmd in scaled.iter_mut() {
            match cmd {
                GlyphCmd::MoveTo(x, y) => {
                    *x *= px_scale;
                    *y *= px_scale;
                }
                GlyphCmd::LineTo(x, y) => {
                    *x *= px_scale;
                    *y *= px_scale;
                }
                GlyphCmd::QuadTo(x, y, ex, ey) => {
                    *x *= px_scale;
                    *y *= px_scale;
                    *ex *= px_scale;
                    *ey *= px_scale;
                }
                GlyphCmd::Close => {}
            }
        }
        return Some(scaled);
    }
    let g = gid as usize;
    if g + 1 >= t.loca.len() {
        return None;
    }
    let start = t.loca[g] as usize;
    let end = t.loca[g + 1] as usize;
    let len = end.saturating_sub(start);
    if len < 10 {
        return None;
    }
    let o = t.glyf.0 + start;
    let ncont = i16o(t.d, o);
    if ncont <= 0 {
        return None;
    }
    let scale = px_scale;
    let end_pts_off = o + 10;
    let npts = u16o(t.d, end_pts_off + (ncont as usize - 1) * 2) as usize + 1;
    let flags_off = end_pts_off + ncont as usize * 2
        + 2
        + u16o(t.d, end_pts_off + ncont as usize * 2) as usize;
    let mut flags: Vec<u8> = Vec::with_capacity(npts);
    let mut p = flags_off;
    while flags.len() < npts {
        let f = t.d[p];
        p += 1;
        flags.push(f);
        if f & 8 != 0 {
            let rep = t.d[p];
            p += 1;
            for _ in 0..rep {
                flags.push(f);
            }
        }
    }
    let mut xs: Vec<i32> = Vec::with_capacity(npts);
    let mut x: i32 = 0;
    let mut p2 = p;
    for &f in flags.iter() {
        if f & 2 != 0 {
            let dx = i32::from(t.d[p2]);
            p2 += 1;
            x = if f & 16 != 0 { x + dx } else { x - dx };
        } else if f & 16 == 0 {
            let dx = i32::from(i16o(t.d, p2));
            p2 += 2;
            x += dx;
        }
        xs.push(x);
    }
    let mut ys: Vec<i32> = Vec::with_capacity(npts);
    let mut y: i32 = 0;
    let mut p3 = p2;
    for &f in flags.iter() {
        if f & 4 != 0 {
            let dy = i32::from(t.d[p3]);
            p3 += 1;
            y = if f & 32 != 0 { y + dy } else { y - dy };
        } else if f & 32 == 0 {
            let dy = i32::from(i16o(t.d, p3));
            p3 += 2;
            y += dy;
        }
        ys.push(y);
    }
    let mut out: SmallVec<[GlyphCmd; 40]> = SmallVec::new();
    let mut pt = 0usize;
    for c in 0..ncont as usize {
        let end_pt = u16o(t.d, end_pts_off + c * 2) as usize;
        let n = end_pt - pt + 1;
        let on_curve = |i: usize| flags[i] & 1 != 0;
        let first = pt;
        let last = end_pt;
        let mut i = first;
        let mut start_on = on_curve(i);
        let mut start_off_prev: Option<usize> = None;
        if !start_on {
            let prev = if i == first { last } else { i - 1 };
            if !on_curve(prev) {
                start_off_prev = Some(prev);
                let sx = (xs[first] + xs[prev]) / 2;
                let sy = (ys[first] + ys[prev]) / 2;
                out.push(GlyphCmd::MoveTo(sx as f32 * scale, sy as f32 * scale));
                start_on = true;
                i = if i + 1 <= last { i + 1 } else { first };
            } else {
                out.push(GlyphCmd::MoveTo(
                    xs[prev] as f32 * scale,
                    ys[prev] as f32 * scale,
                ));
            }
        } else {
            out.push(GlyphCmd::MoveTo(
                xs[i] as f32 * scale,
                ys[i] as f32 * scale,
            ));
            i += 1;
        }
        if i > last && start_off_prev.is_none() {
            out.push(GlyphCmd::Close);
            pt = end_pt + 1;
            continue;
        }
        let mut prev_off: Option<(f32, f32)> = None;
        while i <= last {
            if on_curve(i) {
                match prev_off.take() {
                    Some((ox, oy)) => out.push(GlyphCmd::QuadTo(
                        ox,
                        oy,
                        xs[i] as f32 * scale,
                        ys[i] as f32 * scale,
                    )),
                    None => out.push(GlyphCmd::LineTo(
                        xs[i] as f32 * scale,
                        ys[i] as f32 * scale,
                    )),
                }
            } else if let Some((ox, oy)) = prev_off {
                let mid_x = (ox + xs[i] as f32 * scale) / 2.0;
                let mid_y = (oy + ys[i] as f32 * scale) / 2.0;
                out.push(GlyphCmd::QuadTo(ox, oy, mid_x, mid_y));
                prev_off = Some((xs[i] as f32 * scale, ys[i] as f32 * scale));
            } else {
                prev_off = Some((xs[i] as f32 * scale, ys[i] as f32 * scale));
            }
            i += 1;
        }
        if let Some((ox, oy)) = prev_off {
            let (ex, ey) = if start_off_prev.is_some() {
                (xs[first] as f32 * scale, ys[first] as f32 * scale)
            } else {
                match out.first() {
                    Some(GlyphCmd::MoveTo(mx, my)) => (*mx, *my),
                    _ => (0.0, 0.0),
                }
            };
            out.push(GlyphCmd::QuadTo(ox, oy, ex, ey));
        }
        out.push(GlyphCmd::Close);
        pt = end_pt + 1;
    }
    if CONTOUR_CACHE.len() < 65536 && out.len() <= 40 {
        let _ = CONTOUR_CACHE.insert_sync(cache_key, out.clone());
    }
    Some(out)
}

pub struct TextMetrics {
    pub width: f64,
    pub actual_left: f64,
    pub actual_right: f64,
    pub ascent: f64,
    pub descent: f64,
}

pub fn measure(kind: FontKind, text: &str, px: f64) -> TextMetrics {
    let t = font_tables(kind);
    let m = font_metrics(kind).scaled(px);
    let scale = m.em_scale;
    let asc = m.ascender;
    let desc = m.descender;
    let mut width = 0.0;
    let mut origin_ink = 0.0;
    let mut ink_left = f64::MAX;
    let mut ink_right = f64::MIN;
    let mut ink_top = f64::MIN;
    let mut ink_bottom = f64::MAX;
    for ch in text.chars() {
        let cp = ch as u32;
        if let Some(t) = t
            && let Some(b) = glyph_box(t, cp)
        {
            let l = origin_ink + f64::from(b.x0) * scale;
            let r = origin_ink + f64::from(b.x1) * scale;
            let top = f64::from(b.y1) * scale;
            let bot = f64::from(b.y0) * scale;
            ink_left = ink_left.min(l);
            ink_right = ink_right.max(r);
            ink_top = ink_top.max(top);
            ink_bottom = ink_bottom.min(bot);
        }
        width += match t {
            Some(t) => advance_of_cp(t, cp, px),
            None => px * 0.5,
        };
        origin_ink = width;
    }
    if ink_left == f64::MAX {
        ink_left = 0.0;
        ink_right = width;
        ink_top = asc;
        ink_bottom = -desc;
    }
    TextMetrics {
        width,
        actual_left: ink_left.max(0.0),
        actual_right: ink_right.max(0.0),
        ascent: (asc - ink_top).max(0.0),
        descent: (ink_bottom + desc).max(0.0),
    }
}
