use core_utils::crypto::{H0, sha256_block, sha256_midstate, words_be32};

use crate::scan::{BatchCtx, ScanPlan, pad64, tail_digest};

pub struct HmacCtx {
    inner_prefix: [u32; 8],
    msg_tail: [u8; 63],
    msg_tail_len: usize,
    msg_len: usize,
    outer_prefix: [u32; 8],
    outer_tail: [u8; 32],
    need_bits: u32,
    avx512: bool,
}

impl HmacCtx {
    pub fn new(key: &[u8], msg: &[u8], difficulty: u8) -> Self {
        let mut key_norm = [0u8; 64];
        if key.len() > 64 {
            let h = core_utils::sha256(key);
            key_norm[..32].copy_from_slice(&h);
        } else {
            key_norm[..key.len()].copy_from_slice(key);
        }
        let mut ipad = [0x36u8; 64];
        let mut opad = [0x5cu8; 64];
        for (i, &b) in key_norm.iter().enumerate() {
            ipad[i] ^= b;
            opad[i] ^= b;
        }
        let mut inner_prefix = H0;
        sha256_block(&mut inner_prefix, &ipad);
        let (inner_prefix, msg_tail, tail_len) = sha256_midstate(inner_prefix, msg);
        let mut outer_prefix = H0;
        sha256_block(&mut outer_prefix, &opad);
        let mut outer_blk = [0u8; 64];
        pad64(&mut outer_blk, 32, 96);
        HmacCtx {
            inner_prefix,
            msg_tail,
            msg_tail_len: tail_len,
            msg_len: msg.len(),
            outer_prefix,
            outer_tail: outer_blk[32..64].try_into().unwrap(),
            need_bits: crate::scan::need_bits_of(difficulty),
            avx512: crate::scan::cpu_avx512_enabled(),
        }
    }

    fn total_of(&self, width: usize) -> usize {
        64 + self.msg_len + width
    }

    pub fn digest(&self, nonce: u64, width: usize) -> [u8; 32] {
        let inner = tail_digest(
            self.inner_prefix,
            &self.msg_tail[..self.msg_tail_len],
            self.total_of(width),
            nonce,
            width,
        );
        self.outer_round(inner)
    }

    #[inline(always)]
    fn outer_round(&self, inner: [u8; 32]) -> [u8; 32] {
        let mut outer = [0u8; 64];
        outer[..32].copy_from_slice(&inner);
        outer[32..64].copy_from_slice(&self.outer_tail);
        let mut os = self.outer_prefix;
        sha256_block(&mut os, &outer);
        words_be32(&os)
    }
}

impl BatchCtx for HmacCtx {
    fn plan_of(&self, width: usize) -> ScanPlan {
        ScanPlan::build(
            width,
            &self.msg_tail[..self.msg_tail_len],
            self.total_of(width),
        )
    }

    fn batch_off(&self) -> usize {
        self.msg_tail_len
    }

    fn batch_prefix(&self) -> [u32; 8] {
        self.inner_prefix
    }

    fn need_bits(&self) -> u32 {
        self.need_bits
    }

    fn avx512(&self) -> bool {
        self.avx512
    }

    fn mid_one(&self, st: &[u32; 8]) -> [u8; 32] {
        self.outer_round(words_be32(st))
    }

    unsafe fn init_blocks(&self, blocks: &mut [[u8; 64]; 8]) {
        for ok in blocks.iter_mut() {
            ok[32..64].copy_from_slice(&self.outer_tail);
        }
    }

    unsafe fn mid_step(&self, st: &mut [[u32; 8]; 8], blocks: &mut [[u8; 64]; 8]) {
        for (k, stk) in st.iter_mut().enumerate() {
            blocks[k][..32].copy_from_slice(&words_be32(stk));
            *stk = self.outer_prefix;
        }
        core_utils::compress8(st, blocks);
    }
}

pub fn solve(key: &[u8], msg: &[u8], difficulty: u8, threads: usize) -> Option<(u64, [u8; 32])> {
    crate::scan::batched_solve(HmacCtx::new(key, msg, difficulty), threads)
}

pub fn solve_until(
    key: &[u8],
    msg: &[u8],
    difficulty: u8,
    threads: usize,
    abort: &std::sync::atomic::AtomicBool,
) -> Option<(u64, [u8; 32])> {
    crate::scan::batched_solve_until(HmacCtx::new(key, msg, difficulty), threads, abort)
}
