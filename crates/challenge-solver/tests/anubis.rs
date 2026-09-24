use challenge_solver::Algorithm;
use challenge_solver::anubis;
use core_utils::sha256;

const CHALLENGE_JSON: &[u8] = br#"{"rules":{"algorithm":"fast","difficulty":4,"report_as":4},"challenge":{"issuedAt":"2026-09-06T17:25:44.720860281Z","metadata":{"User-Agent":"Mozilla/5.0 ...","X-Real-Ip":"2a02:3100:..."},"id":"01a07817-7a86-7e54-8fce-36e99de23289","method":"fast","randomData":"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUV","difficulty":4}}"#;

#[test]
fn parse_full_challenge() {
    let ch = anubis::AnubisChallenge::parse(CHALLENGE_JSON).expect("parse");
    assert_eq!(ch.id.as_slice(), b"01a07817-7a86-7e54-8fce-36e99de23289");
    assert_eq!(ch.difficulty, 4);
    assert_eq!(ch.algorithm, Algorithm::Sha256);
    assert_eq!(ch.random_data.len(), 110);
}

#[test]
fn parse_minimal_challenge() {
    let raw = br#"{"challenge":{"id":"abc","randomData":"xyz","difficulty":3}}"#;
    let ch = anubis::AnubisChallenge::parse(raw).expect("parse");
    assert_eq!(ch.algorithm, Algorithm::Sha256);
    assert_eq!(ch.difficulty, 3);
    assert!(anubis::AnubisChallenge::parse(br#"{"x":1}"#).is_err());
    assert!(
        anubis::AnubisChallenge::parse(
            br#"{"algorithm":"wat","id":"a","randomData":"b","difficulty":1}"#
        )
        .is_err()
    );
}

#[test]
fn parse_takes_challenge_fields_not_rules() {
    let raw = br#"{"rules":{"algorithm":"fast","difficulty":9,"report_as":9},"challenge":{"id":"zzz","method":"fast","randomData":"abc","difficulty":2}}"#;
    let ch = anubis::AnubisChallenge::parse(raw).expect("parse");
    assert_eq!(ch.difficulty, 2, "difficulty обязан браться из challenge, не из rules");
    assert_eq!(ch.id.as_slice(), b"zzz");
}

#[test]
fn emu_elapsed_is_deterministic_and_inside_gauss_band() {
    let core = 1000.0f64 / 3000.0 + 25.0;
    for j in [-1.5f64, 0.0, 1.5] {
        let a = anubis::emu_elapsed_ms(1000, 1.0, j);
        assert_eq!(a, anubis::emu_elapsed_ms(1000, 1.0, j));
        assert!(
            (a - core).abs() < 0.35 * core,
            "джиттер обязан быть полосой вокруг {core}, получил {a}"
        );
    }
    assert_ne!(
        anubis::emu_elapsed_ms(1000, 1.0, 1.0),
        anubis::emu_elapsed_ms(1000, 1.0, -1.0),
        "джиттер не должен вырождаться в ноль"
    );
}

#[test]
fn emu_elapsed_scales_with_cpu_and_attempts() {
    let fast = anubis::emu_elapsed_ms(1_000_000, 0.5, 0.0);
    let slow = anubis::emu_elapsed_ms(1_000_000, 2.5, 0.0);
    assert!(fast < slow, "fast={fast} slow={slow}");
    assert!(anubis::emu_elapsed_ms(10_000_000, 1.0, 0.0) > anubis::emu_elapsed_ms(0, 1.0, 0.0));
    let base = anubis::emu_elapsed_ms(0, 1.0, 0.0);
    assert!(
        base > 15.0 && base < 40.0,
        "overhead-ядро ~25мс, получил {base}"
    );
}

#[test]
fn emu_elapsed_jitter_is_gaussian_not_sawtooth() {
    let n = 2000u64;
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut distinct = std::collections::HashSet::new();
    for a in 0..n {
        let core = a as f64 / 3000.0 + 25.0;
        let j = ((a * 2654435761 % 10007) as f64 / 10007.0 - 0.5) * 2.0;
        let r = anubis::emu_elapsed_ms(a, 1.0, j) / core - 1.0;
        sum += r;
        sum_sq += r * r;
        distinct.insert((r * 1e9).round() as i64);
    }
    let mean = sum / n as f64;
    let var = sum_sq / n as f64 - mean * mean;
    assert!(
        mean.abs() < 0.006,
        "гаусс обязан быть нулевым средним, mean={mean}"
    );
    assert!(
        var > 0.0005 && var < 0.0033,
        "σ²={var} — ждём разброс джиттера σ≈0.05 (uniform-драйвер var≈0.0008)"
    );
    assert!(
        distinct.len() > 1900,
        "равномерный остаток от деления дал бы повторы, уникальных: {}",
        distinct.len()
    );
}

#[test]
fn emu_elapsed_survives_garbage_cpu_scale() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0, 100.0] {
        let v = anubis::emu_elapsed_ms(1000, bad, 0.0);
        assert!(v.is_finite() && v > 0.0, "cpu_scale={bad} отдал {v}");
    }
    assert!(anubis::emu_elapsed_ms(0, f64::NAN, 0.0) < 40.0);
}

fn hex_bytes(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, pair) in s.as_bytes().chunks(2).take(32).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    out
}

#[test]
fn solve_and_answer_json() {
    let ch = anubis::AnubisChallenge::parse(CHALLENGE_JSON).expect("parse");
    let sol = anubis::solve(&ch, 2, 1.0, None).expect("solve");
    assert!(sol.elapsed_time_ms > 0.0);
    assert!(
        sol.elapsed_time_ms > 10.0 && sol.elapsed_time_ms < 500.0,
        "elapsed={}",
        sol.elapsed_time_ms
    );
    assert!(sol.end_time_ms >= sol.start_time_ms);
    let mut out = String::with_capacity(192);
    anubis::answer_json(&sol, &mut out);
    assert!(out.contains("\"nonce\":"));
    assert!(out.contains("\"response\":\""));
    assert!(out.contains("\"elapsedTime\":"));
    assert!(out.contains("\"startMs\":"));
    assert!(out.contains("\"endMs\":"));
    let mut msg = ch.random_data.to_vec();
    msg.extend_from_slice(sol.nonce.to_string().as_bytes());
    let h = sha256(&msg);
    assert_eq!(
        h,
        hex_bytes(std::str::from_utf8(&sol.response_hex).unwrap())
    );
}

#[test]
fn pass_url_format() {
    let ch = anubis::AnubisChallenge::parse(CHALLENGE_JSON).expect("parse");
    let mut rh = [b'0'; 64];
    rh[..10].copy_from_slice(b"000011c5ce");
    let mut url = String::new();
    anubis::build_pass_url(
        "https://baresearch.org/",
        &ch.id,
        &rh,
        56381,
        58.79,
        "https://baresearch.org/search?q=test",
        &mut url,
    );
    assert!(
        url.starts_with(
            "https://baresearch.org/.within.website/x/cmd/anubis/api/pass-challenge?id="
        )
    );
    assert!(url.contains("nonce=56381"));
    assert!(url.contains("response=000011c5ce"));
    assert!(url.contains("elapsedTime="));
    assert!(url.contains("redir=https%3A%2F%2Fbaresearch.org%2Fsearch%3Fq%3Dtest"));
}

#[test]
fn solve_dispatches_without_sha_ni() {
    let ch = anubis::AnubisChallenge::parse(CHALLENGE_JSON).expect("parse");
    let sol = anubis::solve(&ch, 1, 1.0, None).expect("solve via dispatched compress");
    let mut msg = ch.random_data.to_vec();
    msg.extend_from_slice(sol.nonce.to_string().as_bytes());
    assert_eq!(
        sha256(&msg),
        hex_bytes(std::str::from_utf8(&sol.response_hex).unwrap())
    );
}

#[test]
fn parse_rejects_difficulty_bounds() {
    for d in [0u32, 17] {
        let json = format!("{{\"id\":\"a\",\"randomData\":\"b\",\"difficulty\":{d}}}");
        assert!(
            anubis::AnubisChallenge::parse(json.as_bytes()).is_err(),
            "difficulty {d} должен отвергаться"
        );
    }

    for d in [1u32, 16] {
        let json = format!("{{\"id\":\"a\",\"randomData\":\"b\",\"difficulty\":{d}}}");
        let ch = anubis::AnubisChallenge::parse(json.as_bytes())
            .unwrap_or_else(|e| panic!("difficulty {d} легален: {e:?}"));
        assert_eq!(ch.difficulty, d as u8);
    }
}

#[test]
fn parse_rejects_degenerate_id_and_random_data() {
    assert!(matches!(
        anubis::AnubisChallenge::parse(br#"{"id":"","randomData":"b","difficulty":1}"#),
        Err(anubis::ParseError::Id)
    ));

    let long_id = "x".repeat(65);
    let json = format!("{{\"id\":\"{long_id}\",\"randomData\":\"b\",\"difficulty\":1}}");
    assert!(matches!(
        anubis::AnubisChallenge::parse(json.as_bytes()),
        Err(anubis::ParseError::Id)
    ));

    let long_rd = "y".repeat(1025);
    let json = format!("{{\"id\":\"a\",\"randomData\":\"{long_rd}\",\"difficulty\":1}}");
    assert!(matches!(
        anubis::AnubisChallenge::parse(json.as_bytes()),
        Err(anubis::ParseError::RandomData)
    ));

    let ok = format!(
        "{{\"id\":\"{}\",\"randomData\":\"{}\",\"difficulty\":1}}",
        "z".repeat(64),
        "w".repeat(1024)
    );
    let ch = anubis::AnubisChallenge::parse(ok.as_bytes()).expect("boundary sizes are legal");
    assert_eq!(ch.id.len(), 64);
    assert_eq!(ch.random_data.len(), 1024);
}

#[test]
fn parse_rejects_truncated_json() {
    assert!(
        anubis::AnubisChallenge::parse(br#"{"challenge":{"id":"a","randomData":"abc"#).is_err()
    );

    assert!(
        anubis::AnubisChallenge::parse(br#"{"id":"a","randomData":"b","difficulty":"#).is_err()
    );
}

#[test]
fn parse_difficulty_float_fraction_is_swallowed() {
    let ch = anubis::AnubisChallenge::parse(br#"{"id":"a","randomData":"b","difficulty":4.5}"#)
        .expect("find_u32 takes the integer part");
    assert_eq!(ch.difficulty, 4);
}
