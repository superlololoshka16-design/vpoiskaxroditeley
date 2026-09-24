use smallvec::SmallVec;

const EXCHANGE_HEADER: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderError {
    Malformed,
    Degenerate,
    TooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliderHit {
    pub x: u32,
    pub y: u32,
    pub ssd: u64,
}

pub struct SliderExchange<'a> {
    pub bg_w: u32,
    pub bg_h: u32,
    pub piece_w: u32,
    pub piece_h: u32,
    pub bg: &'a [u8],
    pub piece: &'a [u8],
}

impl<'a> SliderExchange<'a> {
    pub fn parse(payload: &'a [u8]) -> Result<Self, SliderError> {
        if payload.len() < EXCHANGE_HEADER {
            return Err(SliderError::Malformed);
        }
        let mut dims = [0u32; 4];
        for (k, dim) in dims.iter_mut().enumerate() {
            *dim = u16::from_le_bytes([payload[k * 2], payload[k * 2 + 1]]) as u32;
        }
        let [bg_w, bg_h, pw, ph] = dims;
        let bg_len = bg_w
            .checked_mul(bg_h)
            .and_then(|p| p.checked_mul(4))
            .ok_or(SliderError::TooLarge)? as usize;
        let p_len = pw
            .checked_mul(ph)
            .and_then(|p| p.checked_mul(4))
            .ok_or(SliderError::TooLarge)? as usize;
        if bg_w == 0 || bg_h == 0 || pw == 0 || ph == 0 || pw > bg_w || ph > bg_h {
            return Err(SliderError::Malformed);
        }
        if bg_len > (1 << 26) || p_len > (1 << 22) {
            return Err(SliderError::TooLarge);
        }
        if payload.len() != EXCHANGE_HEADER + bg_len + p_len {
            return Err(SliderError::Malformed);
        }
        Ok(SliderExchange {
            bg_w,
            bg_h,
            piece_w: pw,
            piece_h: ph,
            bg: &payload[EXCHANGE_HEADER..EXCHANGE_HEADER + bg_len],
            piece: &payload[EXCHANGE_HEADER + bg_len..],
        })
    }

    pub fn build(
        bg_w: u32,
        bg_h: u32,
        bg: &[u8],
        pw: u32,
        ph: u32,
        piece: &[u8],
        out: &mut Vec<u8>,
    ) {
        for dim in [bg_w, bg_h, pw, ph] {
            out.extend_from_slice(&(dim as u16).to_le_bytes());
        }
        out.extend_from_slice(bg);
        out.extend_from_slice(piece);
    }
}

struct MaskOff {
    dy: u32,
    dx: u32,
    pi: u32,
}

type MaskOffsets = SmallVec<[MaskOff; 512]>;

fn mask_offsets(w: u32, h: u32, piece: &[u8], step: u32) -> Option<MaskOffsets> {
    let mut offs: MaskOffsets = SmallVec::new();
    let mut y = 0u32;
    while y < h {
        let mut x = 0u32;
        while x < w {
            let i = ((y * w + x) * 4 + 3) as usize;
            if piece[i] > 0x7F {
                offs.push(MaskOff {
                    dy: y,
                    dx: x,
                    pi: (y * w + x) * 4,
                });
            }
            x += step;
        }
        y += step;
    }
    if offs.is_empty() { None } else { Some(offs) }
}

#[inline(always)]
fn px3(bg: &[u8], base: usize, piece: &[u8], pi: usize) -> u64 {
    let d0 = bg[base] as i32 - piece[pi] as i32;
    let d1 = bg[base + 1] as i32 - piece[pi + 1] as i32;
    let d2 = bg[base + 2] as i32 - piece[pi + 2] as i32;
    (d0 * d0) as u64 + (d1 * d1) as u64 + (d2 * d2) as u64
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn flush_acc8(acc: std::arch::x86_64::__m256i, acc64: &mut [u64; 8]) {
    use std::arch::x86_64::*;
    unsafe {
        let half = _mm256_srli_epi32::<1>(acc);
        let mut slots = [0u32; 8];
        _mm256_storeu_si256(slots.as_mut_ptr() as *mut __m256i, half);
        for (k, s) in slots.iter().enumerate() {
            acc64[k] += u64::from(*s);
        }
    }
}

#[cfg(target_arch = "x86_64")]
const CHAN_MASKS: [u8; 96] = {
    let mut raw = [0x80u8; 96];
    let mut c = 0usize;
    while c < 3 {
        let mut k = 0usize;
        while k < 8 {
            let src = (k * 4 + c) as u8;
            raw[c * 32 + k * 4 + 1] = src;
            raw[c * 32 + k * 4 + 3] = src;
            k += 1;
        }
        c += 1;
    }
    raw
};

#[cfg(target_arch = "x86_64")]
thread_local! {
    static CHAN_SHUFFLE: [std::arch::x86_64::__m256i; 3] = unsafe {
        use std::arch::x86_64::*;
        [
            _mm256_loadu_si256(CHAN_MASKS.as_ptr() as *const __m256i),
            _mm256_loadu_si256(CHAN_MASKS.as_ptr().add(32) as *const __m256i),
            _mm256_loadu_si256(CHAN_MASKS.as_ptr().add(64) as *const __m256i),
        ]
    };
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn ssd_row8_avx2(
    bg: &[u8],
    piece: &[u8],
    base: usize,
    pi: usize,
    masks: &[std::arch::x86_64::__m256i; 3],
    acc: &mut std::arch::x86_64::__m256i,
) {
    use std::arch::x86_64::*;
    unsafe {
        let px = _mm256_loadu_si256(bg.as_ptr().add(base) as *const __m256i);
        for c in 0..3usize {
            let ch16 = _mm256_shuffle_epi8(px, masks[c]);
            let ch = _mm256_srli_epi16::<8>(ch16);
            let p = _mm256_set1_epi16(*piece.get_unchecked(pi + c) as i16);
            let d = _mm256_sub_epi16(ch, p);
            *acc = _mm256_add_epi32(*acc, _mm256_madd_epi16(d, d));
        }
    }
}
const SSD_FLUSH_EVERY: usize = 8192;

#[cfg(target_arch = "x86_64")]
unsafe fn ssd_block8_avx2(
    bg: &[u8],
    piece: &[u8],
    row_base: &[usize],
    offs: &[MaskOff],
    masks: &[std::arch::x86_64::__m256i; 3],
    xb4: usize,
    acc64: &mut [u64; 8],
) {
    use std::arch::x86_64::*;
    unsafe {
        let mut acc8 = _mm256_setzero_si256();
        for (j, off) in offs.iter().enumerate() {
            ssd_row8_avx2(
                bg,
                piece,
                row_base[j] + xb4,
                off.pi as usize,
                masks,
                &mut acc8,
            );
        }
        flush_acc8(acc8, acc64);
    }
}
fn scan(
    ex: &SliderExchange,
    mo: &MaskOffsets,
    y0: u32,
    y1: u32,
    x0: u32,
    x1: u32,
    best: &mut SliderHit,
) {
    let offs = mo.as_slice();
    let bg_w = ex.bg_w;
    #[cfg(target_arch = "x86_64")]
    let avx2 = core_utils::cpu_avx2();
    #[cfg(not(target_arch = "x86_64"))]
    let avx2 = false;

    let stride = (bg_w * 4) as usize;
    #[cfg(target_arch = "x86_64")]
    let masks: [std::arch::x86_64::__m256i; 3] = CHAN_SHUFFLE.with(|m| *m);
    let mut row_base: SmallVec<[usize; 1024]> = SmallVec::with_capacity(offs.len());
    for off in offs {
        row_base.push(((y0 + off.dy) * bg_w + off.dx) as usize * 4);
    }
    let mut y = y0;
    while y < y1 {
        let mut xb = x0;
        while xb < x1 {
            let lanes = (x1 - xb).min(8) as usize;
            let mut acc64 = [0u64; 8];
            if avx2 && lanes == 8 {
                unsafe {
                    ssd_block8_avx2(
                        &ex.bg,
                        &ex.piece,
                        &row_base,
                        offs,
                        &masks,
                        xb as usize * 4,
                        &mut acc64,
                    )
                };
            } else {
                for i in 0..lanes {
                    let xi4 = (xb as usize + i) * 4;
                    let mut acc = 0u64;
                    for (j, off) in offs.iter().enumerate() {
                        acc = acc.wrapping_add(px3(
                            &ex.bg,
                            row_base[j] + xi4,
                            &ex.piece,
                            off.pi as usize,
                        ));
                    }
                    acc64[i] = acc;
                }
            }

            for i in 0..lanes {
                if acc64[i] < best.ssd {
                    best.ssd = acc64[i];
                    best.x = xb + i as u32;
                    best.y = y;
                }
            }
            xb += 8;
        }
        for rb in row_base.iter_mut() {
            *rb += stride;
        }
        y += 1;
    }
}

pub fn solve(ex: &SliderExchange) -> Result<SliderHit, SliderError> {
    let max_x = ex.bg_w - ex.piece_w;
    let max_y = ex.bg_h - ex.piece_h;
    let (coarse, fine) = {
        let fine_fallback = mask_offsets(ex.piece_w, ex.piece_h, ex.piece, 1);
        match mask_offsets(ex.piece_w, ex.piece_h, ex.piece, 2) {
            Some(c) => (c, fine_fallback.ok_or(SliderError::Degenerate)?),
            None => {
                let f = fine_fallback.ok_or(SliderError::Degenerate)?;
                (MaskOffsets::new(), f)
            }
        }
    };
    let mut best = SliderHit {
        x: 0,
        y: 0,
        ssd: u64::MAX,
    };
    if !coarse.is_empty() {
        scan(ex, &coarse, 0, max_y + 1, 0, max_x + 1, &mut best);
    } else {
        scan(ex, &fine, 0, max_y + 1, 0, max_x + 1, &mut best);
    }
    let fy = best.y.saturating_sub(2).min(max_y);
    let fy1 = (best.y + 3).min(max_y + 1);
    let fx = best.x.saturating_sub(2).min(max_x);
    let fx1 = (best.x + 3).min(max_x + 1);
    let mut fine_best = SliderHit {
        x: best.x,
        y: best.y,
        ssd: best.ssd,
    };
    scan(ex, &fine, fy, fy1, fx, fx1, &mut fine_best);
    Ok(fine_best)
}
