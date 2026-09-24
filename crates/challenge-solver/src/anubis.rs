use crate::{Algorithm, Challenge, Solution};
use compact_str::CompactString;
use core_utils::BytesExt as _;
use core_utils::SplitMix64Rng;
use smallvec::SmallVec;

const BASE_HASHES_PER_MS: f64 = 3000.0;
const OVERHEAD_BASE_MS: f64 = 25.0;
const JITTER_SIGMA: f64 = 0.05;
const CHALLENGE_ROUNDS: u32 = 2;
const CHALLENGE_MEM_COST: u32 = 8;
const CHALLENGE_TIME_COST: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Algorithm,
    Difficulty,
    Id,
    RandomData,
}

pub struct AnubisChallenge {
    pub id: SmallVec<[u8; 48]>,
    pub random_data: SmallVec<[u8; 192]>,
    pub difficulty: u8,
    pub algorithm: Algorithm,
}

pub struct SolvedAnubis {
    pub nonce: u64,
    pub response_hex: [u8; 64],
    pub elapsed_time_ms: f64,
    pub start_time_ms: u64,
    pub end_time_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolveError {
    NoSolution,
}

impl AnubisChallenge {
    pub fn parse(json: &[u8]) -> Result<Self, ParseError> {
        let scope = match json.find_sub(b"\"challenge\":") {
            Some(p) => {
                let rest = &json[p + b"\"challenge\":".len()..];
                object_span(rest).unwrap_or(rest)
            }
            None => json,
        };
        let alg_bytes = scope
            .find_quoted(b"\"method\":\"")
            .or_else(|| scope.find_quoted(b"\"algorithm\":\""));
        let algorithm = match alg_bytes {
            Some(b) => Algorithm::parse(b).ok_or(ParseError::Algorithm)?,
            None => Algorithm::Sha256,
        };
        let rd_src = scope
            .find_quoted(b"\"randomData\":\"")
            .ok_or(ParseError::RandomData)?;
        if rd_src.is_empty() || rd_src.len() > 1024 {
            return Err(ParseError::RandomData);
        }
        let random_data: SmallVec<[u8; 192]> = SmallVec::from_slice(rd_src);
        let id_src = scope.find_quoted(b"\"id\":\"").ok_or(ParseError::Id)?;
        if id_src.is_empty() || id_src.len() > 64 {
            return Err(ParseError::Id);
        }
        let id: SmallVec<[u8; 48]> = SmallVec::from_slice(id_src);
        let difficulty = scope
            .find_u32(b"\"difficulty\":")
            .ok_or(ParseError::Difficulty)?;
        if difficulty == 0 || difficulty > 16 {
            return Err(ParseError::Difficulty);
        }
        Ok(AnubisChallenge {
            id,
            random_data,
            difficulty: difficulty as u8,
            algorithm,
        })
    }

    pub fn to_challenge(&self, threads: usize) -> Challenge {
        Challenge {
            algorithm: self.algorithm,
            salt: self.random_data.clone(),
            key: SmallVec::new(),
            difficulty: self.difficulty,
            rounds: CHALLENGE_ROUNDS,
            mem_cost: CHALLENGE_MEM_COST,
            time_cost: CHALLENGE_TIME_COST,
            threads,
        }
    }
}

pub fn emu_elapsed_ms(attempts: u64, cpu_scale: f64, jitter: f64) -> f64 {
    let scale = core_utils::bench::scale_clamp(cpu_scale);
    let calc_ms = attempts as f64 * scale / BASE_HASHES_PER_MS;
    let jitter = jitter.clamp(-2.0, 2.0);
    (calc_ms + OVERHEAD_BASE_MS) * (1.0 + JITTER_SIGMA * jitter)
}

pub fn solve(
    ch: &AnubisChallenge,
    threads: usize,
    cpu_scale: f64,
    deadline: Option<std::time::Instant>,
) -> Result<SolvedAnubis, SolveError> {
    let task = ch.to_challenge(threads);
    let start_real_ms = core_utils::unix_ms();
    let solved = match deadline {
        Some(d) => crate::solve_until(&task, d),
        None => crate::solve(&task),
    };
    let Solution { nonce, digest } = solved.ok_or(SolveError::NoSolution)?;
    let attempts = nonce + 1;
    let jitter = SplitMix64Rng::new(start_real_ms ^ (nonce as u64) << 21).gauss();
    let elapsed_time_ms = emu_elapsed_ms(attempts, cpu_scale, jitter);
    let end_time_ms = core_utils::unix_ms();
    let start_time_ms =
        start_real_ms.max(end_time_ms.saturating_sub(elapsed_time_ms.round() as u64));
    let mut response_hex = [0u8; 64];
    digest.hex_lower_into(&mut response_hex);
    Ok(SolvedAnubis {
        nonce,
        response_hex,
        elapsed_time_ms,
        start_time_ms,
        end_time_ms,
    })
}

pub fn answer_json(sol: &SolvedAnubis, out: &mut String) {
    use std::fmt::Write;
    let hex = unsafe { std::str::from_utf8_unchecked(&sol.response_hex) };
    let _ = write!(
        out,
        "{{\"nonce\":{},\"response\":\"{}\",\"elapsedTime\":{},\"startMs\":{},\"endMs\":{}}}",
        sol.nonce,
        hex,
        core_utils::format_js_float(sol.elapsed_time_ms),
        sol.start_time_ms,
        sol.end_time_ms
    );
}

pub fn build_pass_url(
    origin: &str,
    id: &[u8],
    response_hex: &[u8],
    nonce: u64,
    elapsed_ms: f64,
    redir: &str,
    out: &mut String,
) {
    use std::fmt::Write;
    let _ = write!(
        out,
        "{}/.within.website/x/cmd/anubis/api/pass-challenge",
        origin.trim_end_matches('/')
    );
    let mut enc: SmallVec<[u8; 128]> = SmallVec::new();
    out.push_str("?id=");
    core_utils::percent_encode_into(id, &mut enc);
    out.push_str(unsafe { std::str::from_utf8_unchecked(enc.as_slice()) });
    out.push_str("&response=");
    out.push_str(unsafe { std::str::from_utf8_unchecked(response_hex) });
    let _ = write!(out, "&nonce={}", nonce);
    out.push_str("&redir=");
    enc.clear();
    core_utils::percent_encode_into(redir.as_bytes(), &mut enc);
    out.push_str(unsafe { std::str::from_utf8_unchecked(enc.as_slice()) });
    out.push_str("&elapsedTime=");
    out.push_str(core_utils::format_js_float(elapsed_ms).as_str());
}


fn object_span(b: &[u8]) -> Option<&[u8]> {
    let start = b.iter().position(|&c| c == b'{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, &c) in b.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&b[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}
