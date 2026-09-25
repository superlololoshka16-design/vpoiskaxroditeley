use core_utils::BytesExt as _;
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
    ascender: i16,
    descender: i16,
    cmap: (usize, usize),
    hmtx: (usize, usize),
    num_h: u16,
    glyf: (usize, usize),
    loca: Vec<u32>,
    em_scale: f64,
}

fn parse_tables(d: &'static [u8]) -> Option<TtfTables> {
    if d.len() < 12 || d[..4] != [0, 1, 0, 0] {
        return None;
    }
    let num = d.be_u16(4) as usize;
    if 12 + num.checked_mul(16)? > d.len() {
        return None;
    }
    let mut cmap = (0usize, 0usize);
    let mut hmtx = (0usize, 0usize);
    let mut glyf = (0usize, 0usize);
    let mut head = None;
    let mut hhea = None;
    let mut loca = (0usize, 0usize);
    let mut maxp = None;
    let mut off = 12;
    for _ in 0..num {
        let t = &d[off..off + 4];
        let o = d.be_u32(off + 8) as usize;
        let l = d.be_u32(off + 12) as usize;
        let valid = o.checked_add(l).is_some_and(|e| e <= d.len());
        if valid {
            match t {
                b"cmap" => cmap = (o, l),
                b"hmtx" => hmtx = (o, l),
                b"glyf" => glyf = (o, l),
                b"head" if l >= 54 => head = Some((o, l)),
                b"hhea" if l >= 36 => hhea = Some((o, l)),
                b"loca" => loca = (o, l),
                b"maxp" if l >= 6 => maxp = Some((o, l)),
                _ => {}
            }
        }
        off += 16;
    }
    let (ho, _) = head?;
    let (hho, _) = hhea?;
    let (mo, _) = maxp?;
    let units_per_em = d.be_u16(ho + 18);
    if units_per_em == 0 {
        return None;
    }
    let loca_long = d.be_u16(ho + 50) != 0;
    let n_glyphs = d.be_u16(mo + 4) as usize;
    let num_h = d.be_u16(hho + 34);
    if num_h == 0 {
        return None;
    }
    let entry = if loca_long { 4 } else { 2 };
    let need = (n_glyphs + 1).checked_mul(entry)?;
    if need > loca.1 || loca.0.checked_add(need)? > d.len() {
        return None;
    }
    if (num_h as usize).checked_mul(4).is_some_and(|n| hmtx.0 + n > d.len()) {
        return None;
    }
    let mut locs = Vec::with_capacity(n_glyphs + 1);
    if loca_long {
        for i in 0..=n_glyphs {
            locs.push(d.be_u32(loca.0 + i * 4));
        }
    } else {
        for i in 0..=n_glyphs {
            locs.push(d.be_u16(loca.0 + i * 2) as u32 * 2);
        }
    }
    if locs.iter().any(|&o| {
        o as usize > glyf.1 || glyf.0.checked_add(o as usize).is_none_or(|a| a > d.len())
    }) {
        return None;
    }
    if cmap.1 < 4 || cmap.0 + 4 > d.len() {
        return None;
    }
    let em_scale = 1.0 / f64::from(units_per_em);
    Some(TtfTables {
        d,
        ascender: d.be_i16(hho + 4),
        descender: d.be_i16(hho + 6),
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
    let n = t.d.be_u16(co + 2) as usize;
    let mut sub: Option<usize> = None;
    for i in 0..n {
        let pid = t.d.be_u16(co + 4 + i * 8);
        let eid = t.d.be_u16(co + 6 + i * 8);
        let o = t.d.be_u32(co + 8 + i * 8) as usize;
        let fmt = t.d.be_u16(co + o);
        if fmt == 4 && ((pid == 3 && eid == 1) || (pid == 0 && (eid == 3 || eid == 4))) {
            sub = Some(co + o);
            break;
        }
    }
    let Some(sub) = sub else { return 0 };
    let seg_x2 = t.d.be_u16(sub + 6) as usize;
    let seg = seg_x2 / 2;
    let end_off = sub + 14;
    let starts = end_off + seg_x2 + 2;
    let deltas = starts + seg_x2;
    let ranges = deltas + seg_x2;
    for i in 0..seg {
        let end = t.d.be_u16(end_off + i * 2) as u32;
        if cp <= end {
            let start = t.d.be_u16(starts + i * 2) as u32;
            if cp < start {
                return 0;
            }
            let delta = t.d.be_i16(deltas + i * 2);
            let rng = t.d.be_u16(ranges + i * 2);
            if rng == 0 {
                return (u32::from(cp as u16) .wrapping_add(delta as u32) & 0xFFFF) as u16;
            }
            let gi_off = ranges + i * 2 + (cp - start) as usize;
            if gi_off + 2 > t.d.len() {
                return 0;
            }
            let g = t.d.be_u16(gi_off);
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
    t.d.be_u16(ho + idx * 4)
}

#[inline]
pub fn safe_px(px: f64) -> f64 {
    if px.is_finite() && px > 0.0 { px } else { 1.0 }
}

#[inline]
pub fn px_scale(em_scale: f64, px: f64) -> f64 {
    em_scale * safe_px(px)
}

#[inline]
fn tofu_advance(px: f64) -> f64 {
    safe_px(px) * 0.5
}

#[inline]
pub fn advance_of_cp_pub(kind: FontKind, cp: u32, px: f64) -> f64 {
    let Some(t) = font_tables(kind) else { return tofu_advance(px) };
    advance_of_cp(t, cp, px)
}

fn advance_of_cp(t: &TtfTables, cp: u32, px: f64) -> f64 {
    let g = cmap4_lookup(t, cp);
    if g == 0 {
        return tofu_advance(px);
    }
    f64::from(advance_of(t, g)) * px_scale(t.em_scale, px)
}

pub fn advances(kind: FontKind, text: &str, px: f64) -> SmallVec<[f64; 32]> {
    let Some(t) = font_tables(kind) else {
        return text.chars().map(|_| tofu_advance(px)).collect();
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
    let ncont = t.d.be_i16(o);
    if ncont <= 0 {
        return None;
    }
    Some(GlyphBox {
        x0: t.d.be_i16(o + 2),
        y0: t.d.be_i16(o + 4),
        x1: t.d.be_i16(o + 6),
        y1: t.d.be_i16(o + 8),
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
        let s = safe_px(px);
        Self {
            em_scale: self.em_scale * s,
            ascender: self.ascender * s,
            descender: self.descender * s,
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

#[inline]
fn scale_glyphs(out: &mut SmallVec<[GlyphCmd; 40]>, scale: f32) {
    if scale == 1.0 {
        return;
    }
    for cmd in out.iter_mut() {
        match cmd {
            GlyphCmd::MoveTo(x, y) | GlyphCmd::LineTo(x, y) => {
                *x *= scale;
                *y *= scale;
            }
            GlyphCmd::QuadTo(x, y, ex, ey) => {
                *x *= scale;
                *y *= scale;
                *ex *= scale;
                *ey *= scale;
            }
            GlyphCmd::Close => {}
        }
    }
}

pub fn glyph_contours(kind: FontKind, cp: u32, px: f64) -> Option<SmallVec<[GlyphCmd; 40]>> {
    let t = font_tables(kind)?;
    let gid = u32::from(cmap4_lookup(t, cp));
    if gid == 0 {
        return None;
    }
    let cache_key = (kind as u8, gid);
    let scale = px_scale(t.em_scale, px) as f32;
    let mut em = match CONTOUR_CACHE.read_sync(&cache_key, |_, v| v.clone()) {
        Some(hit) => hit,
        None => {
            let decoded = decode_contours_em(t, gid)?;
            if CONTOUR_CACHE.len() < 65536 && decoded.len() <= 40 {
                let _ = CONTOUR_CACHE.insert_sync(cache_key, decoded.clone());
            }
            decoded
        }
    };
    scale_glyphs(&mut em, scale);
    Some(em)
}

fn decode_contours_em(t: &TtfTables, gid: u32) -> Option<SmallVec<[GlyphCmd; 40]>> {
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
    let ncont = t.d.be_i16(o);
    if ncont <= 0 {
        return None;
    }
    let end_pts_off = o + 10;
    let npts = t.d.be_u16(end_pts_off + (ncont as usize - 1) * 2) as usize + 1;
    let flags_off = end_pts_off + ncont as usize * 2
        + 2
        + t.d.be_u16(end_pts_off + ncont as usize * 2) as usize;
    let mut flags: SmallVec<[u8; 64]> = SmallVec::new();
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
    let mut xs: SmallVec<[i32; 64]> = SmallVec::new();
    let mut x: i32 = 0;
    let mut p2 = p;
    for &f in flags.iter() {
        if f & 2 != 0 {
            let dx = i32::from(t.d[p2]);
            p2 += 1;
            x = if f & 16 != 0 { x + dx } else { x - dx };
        } else if f & 16 == 0 {
            let dx = i32::from(t.d.be_i16(p2));
            p2 += 2;
            x += dx;
        }
        xs.push(x);
    }
    let mut ys: SmallVec<[i32; 64]> = SmallVec::new();
    let mut y: i32 = 0;
    let mut p3 = p2;
    for &f in flags.iter() {
        if f & 4 != 0 {
            let dy = i32::from(t.d[p3]);
            p3 += 1;
            y = if f & 32 != 0 { y + dy } else { y - dy };
        } else if f & 32 == 0 {
            let dy = i32::from(t.d.be_i16(p3));
            p3 += 2;
            y += dy;
        }
        ys.push(y);
    }
    let mut out: SmallVec<[GlyphCmd; 40]> = SmallVec::new();
    let mut pt = 0usize;
    for c in 0..ncont as usize {
        let end_pt = t.d.be_u16(end_pts_off + c * 2) as usize;
        let on_curve = |i: usize| flags[i] & 1 != 0;
        let first = pt;
        let last = end_pt;
        let mut i = first;
        let start_on = on_curve(i);
        let mut start_off_prev: Option<usize> = None;
        if !start_on {
            let prev = if i == first { last } else { i - 1 };
            if !on_curve(prev) {
                start_off_prev = Some(prev);
                let sx = (xs[first] + xs[prev]) / 2;
                let sy = (ys[first] + ys[prev]) / 2;
                out.push(GlyphCmd::MoveTo(sx as f32, sy as f32));
                i = if i + 1 <= last { i + 1 } else { first };
            } else {
                out.push(GlyphCmd::MoveTo(xs[prev] as f32, ys[prev] as f32));
            }
        } else {
            out.push(GlyphCmd::MoveTo(xs[i] as f32, ys[i] as f32));
            i += 1;
        }
        if i > last && start_off_prev.is_none() {
            out.push(GlyphCmd::Close);
            pt = end_pt + 1;
            continue;
        }
        let mut prev_off: Option<(f32, f32)> = None;
        while i <= last {
            let px = xs[i] as f32;
            let py = ys[i] as f32;
            if on_curve(i) {
                match prev_off.take() {
                    Some((ox, oy)) => out.push(GlyphCmd::QuadTo(ox, oy, px, py)),
                    None => out.push(GlyphCmd::LineTo(px, py)),
                }
            } else if let Some((ox, oy)) = prev_off {
                out.push(GlyphCmd::QuadTo(ox, oy, (ox + px) / 2.0, (oy + py) / 2.0));
                prev_off = Some((px, py));
            } else {
                prev_off = Some((px, py));
            }
            i += 1;
        }
        if let Some((ox, oy)) = prev_off {
            let (ex, ey) = if start_off_prev.is_some() {
                (xs[first] as f32, ys[first] as f32)
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
            None => tofu_advance(px),
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
