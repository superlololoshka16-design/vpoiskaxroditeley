use challenge_solver::cache::{self, AnswerCache};
use challenge_solver::slider::{self, SliderExchange};

const BG_W: u32 = 96;
const BG_H: u32 = 48;
const P_W: u32 = 12;
const P_H: u32 = 12;
const TRUE_X: u32 = 37;
const TRUE_Y: u32 = 19;

fn synth_payload() -> Vec<u8> {
    let mut bg = vec![0u8; (BG_W * BG_H * 4) as usize];
    for y in 0..BG_H {
        for x in 0..BG_W {
            let i = ((y * BG_W + x) * 4) as usize;
            let v =
                ((x.wrapping_mul(0x9E37_79B1)) ^ (y.wrapping_mul(0x85EB_CA6B)) ^ 0xC2B2_AE35) as u8;
            bg[i] = v;
            bg[i + 1] = v.wrapping_mul(3);
            bg[i + 2] = 255 - v;
            bg[i + 3] = 255;
        }
    }
    let mut piece = vec![0u8; (P_W * P_H * 4) as usize];
    for py in 0..P_H {
        for px in 0..P_W {
            let sx = TRUE_X + px;
            let sy = TRUE_Y + py;
            let b = ((sy * BG_W + sx) * 4) as usize;
            let p = ((py * P_W + px) * 4) as usize;
            piece[p] = bg[b];
            piece[p + 1] = bg[b + 1];
            piece[p + 2] = bg[b + 2];
            piece[p + 3] = if (px + py) % 11 == 0 { 0 } else { 255 };
        }
    }
    let mut out = Vec::with_capacity(8 + bg.len() + piece.len());
    SliderExchange::build(BG_W, BG_H, &bg, P_W, P_H, &piece, &mut out);
    out
}

#[test]
fn slider_finds_cut_position() {
    let payload = synth_payload();
    let ex = SliderExchange::parse(&payload).expect("exchange");
    let hit = slider::solve(&ex).expect("solve");
    assert_eq!(hit.x, TRUE_X);
    assert_eq!(hit.y, TRUE_Y);
    assert_eq!(hit.ssd, 0);
}

#[test]
fn slider_finds_cut_position_with_noisy_piece() {
    let mut payload = synth_payload();
    let bg_len = (BG_W * BG_H * 4) as usize;
    let piece = &mut payload[8 + bg_len..];
    let mut rng = 0x5EED_u64;
    for (i, b) in piece.iter_mut().enumerate() {
        if i % 4 == 3 {
            continue;
        }
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let noise = ((rng >> 33) as i64 % 9 - 4) as i32;
        *b = (*b as i32 + noise).clamp(0, 255) as u8;
    }
    let ex = SliderExchange::parse(&payload).expect("exchange");
    let hit = slider::solve(&ex).expect("solve");
    assert_eq!(hit.x, TRUE_X);
    assert_eq!(hit.y, TRUE_Y);
    assert!(hit.ssd > 0, "шум обязан дать SSD>0 в точке cut");
}

#[test]
fn cache_repeats_answer_without_resolve() {
    let c = AnswerCache::new(64);
    let payload = synth_payload();
    let o1 = cache::solve_with_cache(&c, &SliderExchange::parse(&payload).unwrap(), &payload)
        .expect("solve1");
    assert!(matches!(o1.kind, cache::OutcomeKind::Computed { .. }));
    assert_eq!((o1.x, o1.y), (TRUE_X, TRUE_Y));
    let o2 = cache::solve_with_cache(&c, &SliderExchange::parse(&payload).unwrap(), &payload)
        .expect("solve2");
    assert!(matches!(o2.kind, cache::OutcomeKind::CacheHit));
    assert_eq!((o2.x, o2.y), (TRUE_X, TRUE_Y));
    assert_eq!(c.len(), 1);
    c.record(cache::key_of(&payload), TRUE_X, TRUE_Y);
    assert_eq!(c.len(), 1);
}

#[test]
fn cache_evicts_under_pressure() {
    let c = AnswerCache::new(64);
    for i in 0..80u32 {
        let mut p = synth_payload();
        let last = p.len() - 1;
        p[last] = (p[last]).wrapping_add(i as u8);
        c.record(cache::key_of(&p), i, 0);
    }
    assert!(c.len() <= 64);
}

#[test]
fn malformed_exchange_rejected() {
    assert!(SliderExchange::parse(&[]).is_err());
    assert!(SliderExchange::parse(&[0u8; 4]).is_err());
    let payload = synth_payload();
    let mut short = payload.clone();
    short.pop();
    assert!(SliderExchange::parse(&short).is_err());
    let bad_dims = [0u8, 0, 48, 0, 12, 0, 12, 0];
    assert!(SliderExchange::parse(&bad_dims).is_err());
}
