use core_utils::rng::*;

#[test]
fn next_below_is_uniform_and_bounded() {
    let mut rng = SplitMix64Rng::new(42);
    let mut seen = [false; 7];
    for _ in 0..700 {
        let v = rng.next_below(7);
        assert!(v < 7);
        seen[v as usize] = true;
    }
    assert!(
        seen.iter().all(|&s| s),
        "next_below(7) должен покрывать все значения"
    );
    let mut rng = SplitMix64Rng::new(7);
    assert_eq!(rng.next_below(0), 0);
    assert!(rng.next_below(100) < 100);
}

#[test]
fn xoshiro_known_vectors() {
    let mut r = Rng::new(0xDEADBEEF);
    let first = r.next_u64();
    let second = r.next_u64();

    assert_ne!(first, 0);
    assert_ne!(second, 0);
    assert_ne!(first, second);

    let mut r2 = Rng::new(0xDEADBEEF);
    assert_eq!(r2.next_u64(), first);
    assert_eq!(r2.next_u64(), second);

    let mut sm = SplitMix64Rng::new(0xDEADBEEF).stepped(GOLDEN);
    let s = [sm.next_u64(), sm.next_u64(), sm.next_u64(), sm.next_u64()];
    let mut r3 = Rng::new(0xDEADBEEF);
    let _ = r3.next_u64();

    let mut r4 = Rng::new(0xDEADBEEF);
    let outs: [u64; 4] = [r4.next_u64(), r4.next_u64(), r4.next_u64(), r4.next_u64()];
    let mut all_distinct = true;
    for i in 0..4 {
        for j in i + 1..4 {
            if outs[i] == outs[j] {
                all_distinct = false;
            }
        }
    }
    assert!(
        all_distinct,
        "xoshiro выдал коллизию на первых 4 выходах: {outs:?}"
    );
    assert!(s.iter().any(|&v| v != 0));
}

#[test]
fn rng_zero_state_guard() {
    for k in 0..64u64 {
        let mut r = Rng::new(k);
        let v = r.next_u64();
        let _ = v;
        let mut r2 = Rng::new(k ^ 0xFFFF);
        assert_ne!(r2.next_u64(), r2.next_u64());
    }

    let mut r = Rng::new(0);
    assert_ne!(r.next_u64(), 0);
}

#[test]
fn gauss_smoke() {
    let mut r = Rng::new(7);
    let mut v = [0f64; 10_000];
    for slot in &mut v {
        *slot = r.gauss();
    }
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    assert!(mean.abs() < 0.05, "gauss mean off: {mean}");
    let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / v.len() as f64;
    assert!((0.85..=1.15).contains(&var), "gauss variance off: {var}");
    for x in v {
        assert!(x.is_finite());
    }
}
