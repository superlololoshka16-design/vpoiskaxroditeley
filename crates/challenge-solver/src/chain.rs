use crate::pow::PowCtx;
use core_utils::crypto::{H0, sha256_block, words_be32};

use crate::scan::{BatchCtx, ScanPlan, pad64};

pub struct ChainCtx {
    first: PowCtx,
    rounds: u32,
}

impl ChainCtx {
    pub fn new(salt: &[u8], rounds: u32, difficulty: u8) -> Self {
        ChainCtx {
            first: PowCtx::new(salt, difficulty),
            rounds: rounds.max(1),
        }
    }

    pub fn digest(&self, nonce: u64, width: usize) -> [u8; 32] {
        let mut h = self.first.digest(nonce, width);
        for _ in 1..self.rounds {
            h = self.chained(h);
        }
        h
    }

    #[inline(always)]
    fn chained(&self, h: [u8; 32]) -> [u8; 32] {
        let mut blk = [0u8; 64];
        chain_block(&h, &mut blk);
        let mut st = H0;
        sha256_block(&mut st, &blk);
        words_be32(&st)
    }
}

fn chain_block(h: &[u8; 32], out: &mut [u8; 64]) {
    out[..32].copy_from_slice(h);
    pad64(out, 32, 32);
}

impl BatchCtx for ChainCtx {
    fn plan_of(&self, width: usize) -> ScanPlan {
        self.first.plan_of(width)
    }

    fn batch_off(&self) -> usize {
        self.first.batch_off()
    }

    fn batch_prefix(&self) -> [u32; 8] {
        self.first.batch_prefix()
    }

    fn need_bits(&self) -> u32 {
        self.first.need_bits()
    }

    fn avx512(&self) -> bool {
        self.first.avx512()
    }

    fn mid_one(&self, st: &[u32; 8]) -> [u8; 32] {
        let mut h = words_be32(st);
        for _ in 1..self.rounds {
            h = self.chained(h);
        }
        h
    }

    unsafe fn mid_step(&self, st: &mut [[u32; 8]; 8], blocks: &mut [[u8; 64]; 8]) {
        for _ in 1..self.rounds {
            for (k, stk) in st.iter_mut().enumerate() {
                chain_block(&words_be32(stk), &mut blocks[k]);
                *stk = H0;
            }
            core_utils::compress8(st, blocks);
        }
    }
}

pub fn solve(salt: &[u8], rounds: u32, difficulty: u8, threads: usize) -> Option<(u64, [u8; 32])> {
    crate::scan::batched_solve(ChainCtx::new(salt, rounds, difficulty), threads)
}

pub fn solve_until(
    salt: &[u8],
    rounds: u32,
    difficulty: u8,
    threads: usize,
    abort: &std::sync::atomic::AtomicBool,
) -> Option<(u64, [u8; 32])> {
    crate::scan::batched_solve_until(
        ChainCtx::new(salt, rounds, difficulty),
        threads,
        abort,
    )
}
