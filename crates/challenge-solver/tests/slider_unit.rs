use challenge_solver::slider::{self, SliderError, SliderExchange};

const BG_W: u32 = 24;
const BG_H: u32 = 8;
const P_W: u32 = 6;
const P_H: u32 = 4;
const CUT_X: u32 = 3;
const CUT_Y: u32 = 2;

fn synth_bg() -> Vec<u8> {
    let mut bg = vec![0u8; (BG_W * BG_H * 4) as usize];
    for y in 0..BG_H {
        for x in 0..BG_W {
            let i = ((y * BG_W + x) * 4) as usize;
            let v = (x.wrapping_mul(41) ^ y.wrapping_mul(97) ^ 0x5A) as u8;
            bg[i] = v;
            bg[i + 1] = v.wrapping_mul(5);
            bg[i + 2] = 255 - v;
            bg[i + 3] = 255;
        }
    }
    bg
}

fn cut_piece(bg: &[u8]) -> Vec<u8> {
    let mut piece = vec![0u8; (P_W * P_H * 4) as usize];
    for py in 0..P_H {
        for px in 0..P_W {
            let b = (((CUT_Y + py) * BG_W + (CUT_X + px)) * 4) as usize;
            let p = ((py * P_W + px) * 4) as usize;
            piece[p..p + 3].copy_from_slice(&bg[b..b + 3]);
            piece[p + 3] = 255;
        }
    }
    piece
}

fn exchange(bg: &[u8], piece: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    SliderExchange::build(BG_W, BG_H, bg, P_W, P_H, piece, &mut out);
    out
}

fn scalar_ssd(ex: &SliderExchange, x: u32, y: u32, step: u32) -> u64 {
    let mut acc = 0u64;
    let mut dy = 0;
    while dy < ex.piece_h {
        let mut dx = 0;
        while dx < ex.piece_w {
            let b = (((y + dy) * ex.bg_w + (x + dx)) * 4) as usize;
            let p = ((dy * ex.piece_w + dx) * 4) as usize;
            for c in 0..3 {
                let d = ex.bg[b + c] as i32 - ex.piece[p + c] as i32;
                acc += (d * d) as u64;
            }
            dx += step;
        }
        dy += step;
    }
    acc
}

#[test]
fn solver_matches_scalar_reference_on_avx2_lanes() {
    if !core_utils::cpu_avx2() {
        return;
    }
    let bg = synth_bg();
    let mut piece = cut_piece(&bg);
    let mut rng = 0x5EED_0001u64;
    for (i, b) in piece.iter_mut().enumerate() {
        if i % 4 == 3 {
            continue;
        }
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let noise = ((rng >> 33) as i64 % 5 - 2) as i32;
        *b = (*b as i32 + noise).clamp(0, 255) as u8;
    }
    let payload = exchange(&bg, &piece);
    let ex = SliderExchange::parse(&payload).expect("exchange");
    assert!(
        ex.bg_w - ex.piece_w >= 8,
        "окно должно гнать полные avx2-лэйны"
    );
    let hit = slider::solve(&ex).expect("solve");
    let mut best = (u64::MAX, 0u32, 0u32);
    for y in 0..=ex.bg_h - ex.piece_h {
        for x in 0..=ex.bg_w - ex.piece_w {
            let s = scalar_ssd(&ex, x, y, 1);
            if s < best.0 {
                best = (s, x, y);
            }
        }
    }
    assert_eq!(
        (hit.x, hit.y),
        (best.1, best.2),
        "avx2-ядро разошлось со скалярной ссылкой"
    );
    assert_eq!(hit.ssd, scalar_ssd(&ex, hit.x, hit.y, 2));
}

#[test]
fn degenerate_transparent_piece_rejected() {
    let bg = synth_bg();
    let piece = vec![0u8; (P_W * P_H * 4) as usize];
    let payload = exchange(&bg, &piece);
    let ex = SliderExchange::parse(&payload).expect("exchange");
    assert!(matches!(slider::solve(&ex), Err(SliderError::Degenerate)));
}

#[test]
fn odd_parity_piece_falls_back_to_full_mask() {
    let bg = synth_bg();
    let mut piece = vec![0u8; (P_W * P_H * 4) as usize];
    for py in (1..P_H).step_by(2) {
        for px in (1..P_W).step_by(2) {
            let b = (((CUT_Y + py) * BG_W + (CUT_X + px)) * 4) as usize;
            let p = ((py * P_W + px) * 4) as usize;
            piece[p..p + 3].copy_from_slice(&bg[b..b + 3]);
            piece[p + 3] = 255;
        }
    }
    let payload = exchange(&bg, &piece);
    let ex = SliderExchange::parse(&payload).expect("exchange");
    let hit = slider::solve(&ex).expect("odd alpha mask is solvable");
    assert_eq!((hit.x, hit.y), (CUT_X, CUT_Y));
}

#[test]
fn piece_larger_than_background_rejected() {
    let bg = synth_bg();
    let piece = cut_piece(&bg);
    let mut payload = exchange(&bg, &piece);
    payload[4] = (BG_W + 1) as u8;
    payload[5] = 0;
    assert!(matches!(
        SliderExchange::parse(&payload),
        Err(SliderError::Malformed)
    ));
}

#[test]
fn truncated_header_rejected() {
    for len in 0..8usize {
        assert!(
            matches!(
                SliderExchange::parse(&vec![0u8; len]),
                Err(SliderError::Malformed)
            ),
            "хвост длины {len} обязан отвергаться"
        );
    }
}
