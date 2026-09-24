use challenge_solver::argon::{self, ArgonCtx};

fn fresh_digest(salt: &[u8], m_cost: u32, t_cost: u32) -> [u8; 32] {
    let ctx = ArgonCtx::new(salt, 0, m_cost, t_cost);
    let mut mem = vec![[0u64; 128]; ctx.m_prime() as usize];
    ctx.digest(0, 1, &mut mem)
}

#[test]
fn arena_resizes_when_m_cost_grows_on_same_thread() {
    let small = ArgonCtx::new(b"arena-resize-check", 0, 8, 1);
    let big = ArgonCtx::new(b"arena-resize-check", 0, 32, 1);
    assert_eq!(small.m_prime(), 8);
    assert_eq!(big.m_prime(), 32);
    assert_ne!(
        fresh_digest(b"arena-resize-check", 8, 1),
        fresh_digest(b"arena-resize-check", 32, 1)
    );

    let (n_small, h_small) = argon::solve(b"arena-seed-a", 0, 8, 1, 1).expect("small solve");
    let (n_big, h_big) = argon::solve(b"arena-seed-b", 0, 32, 1, 1).expect("big solve grows arena");
    assert_eq!((n_small, n_big), (0, 0));
    assert_eq!(h_small, fresh_digest(b"arena-seed-a", 8, 1));
    assert_eq!(h_big, fresh_digest(b"arena-seed-b", 32, 1));
}

#[test]
fn argon_clamps_degenerate_costs() {
    let ctx = ArgonCtx::new(b"edge", 0, 0, 0);
    assert_eq!(ctx.m_prime(), 8);
    assert_eq!(fresh_digest(b"edge", 0, 0), fresh_digest(b"edge", 8, 1));
}

#[test]
fn argon_empty_salt_is_stable() {
    assert_eq!(fresh_digest(b"", 8, 1), fresh_digest(b"", 8, 1));
    let (n, tag) = argon::solve(b"", 0, 8, 1, 1).expect("solve with empty salt");
    assert_eq!(n, 0);
    assert_eq!(tag, fresh_digest(b"", 8, 1));
}
