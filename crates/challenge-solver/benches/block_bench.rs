#[cfg(target_arch = "x86_64")]
fn main() {
    use core_utils::crypto::{H0, sha256_block_ni_2x};
    use std::hint::black_box;
    use std::time::Instant;

    unsafe {
        let mut st = [H0; 8];
        let blk = [[0xABu8; 64]; 8];
        let n = 1u64 << 21;
        let t0 = Instant::now();
        for _ in 0..n {
            for k in 0..4 {
                let a = k * 2;
                let (sa, sb) = core_utils::pair_mut(&mut st, a);
                sha256_block_ni_2x(sa, sb, &blk[a], &blk[a + 1]);
            }
        }
        black_box(st);
        let dt = t0.elapsed().as_secs_f64().max(1e-9);
        let blocks = n * 8;
        println!(
            "2x-interleave: {} blocks in {:.3}s = {:.2} MH/s",
            blocks,
            dt,
            blocks as f64 / dt / 1e6
        );
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn main() {
    println!("sha-ni bench: x86_64 only, skipped");
}
