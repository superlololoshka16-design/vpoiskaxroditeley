use std::hint::black_box;
use std::time::Instant;

fn bench_diff(data: &[u8], diff: u8, threads: usize) {
    let t0 = Instant::now();
    let r = challenge_solver::pow::solve(data, diff, threads);
    let dt = t0.elapsed().as_secs_f64().max(1e-9);
    let n = r.map(|(n, _)| n + 1).unwrap_or(0);
    println!(
        "diff={} threads={} nonce={} time={:.3}s rate={:.2} MH/s",
        diff,
        threads,
        n.saturating_sub(1),
        dt,
        n as f64 / dt / 1e6
    );
}

fn main() {
    let data = b"bench-randomData-0123456789abcdefghijklmnopqrstuvwxyz";
    bench_diff(data, 4, 1);
    bench_diff(data, 4, 2);
    bench_diff(data, 4, 4);
    bench_diff(data, 5, 4);
    let _ = black_box(data);
}
