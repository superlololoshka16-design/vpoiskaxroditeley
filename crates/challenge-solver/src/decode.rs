use bumpalo::Bump;
use core_utils::BytesExt as _;
use std::alloc::Layout;

pub(crate) const MAX_INPUT: usize = 2 * 1024 * 1024;

const MAX_META: usize = 512;
const MAX_PAYLOAD: usize = 1 << 20;
const DIGEST_LEN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    Alphabet,
    Base64,
    Length,
    Meta,
    Digest,
    Size,
}

#[derive(Debug, Clone, Copy)]
pub struct Meta {
    pub kind: TaskKind,
    pub len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    Pow,
    Slider,
    Unknown,
}

impl TaskKind {
    fn parse(s: &[u8]) -> Self {
        if crate::Algorithm::parse(s).is_some() {
            TaskKind::Pow
        } else if matches!(s, b"slider" | b"puzzle" | b"captcha") {
            TaskKind::Slider
        } else {
            TaskKind::Unknown
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            TaskKind::Pow => "pow",
            TaskKind::Slider => "slider",
            TaskKind::Unknown => "unknown",
        }
    }
}

pub struct Scratch {
    arena: Bump,
}

impl Default for Scratch {
    fn default() -> Self {
        Scratch::with_capacity(MAX_PAYLOAD)
    }
}

impl Scratch {
    pub fn with_capacity(payload_cap: usize) -> Self {
        let cap = payload_cap.min(MAX_PAYLOAD);
        Scratch {
            arena: Bump::with_capacity(cap * 3 + 8),
        }
    }

    fn b64_decode<'a>(&'a self, input: &[u8]) -> Result<&'a [u8], DecodeError> {
        let trimmed = input.trim_ascii_extra(b"");
        if trimmed.is_empty() {
            return Err(DecodeError::Alphabet);
        }
        if trimmed.len() > MAX_INPUT {
            return Err(DecodeError::Size);
        }
        let cap = core_utils::b64_decode_cap(trimmed.len());
        let layout = Layout::array::<u8>(cap).map_err(|_| DecodeError::Size)?;
        let ptr = self
            .arena
            .try_alloc_layout(layout)
            .map_err(|_| DecodeError::Size)?;
        let out = unsafe { std::slice::from_raw_parts_mut(ptr.as_ptr(), cap) };
        let n = trimmed
            .b64_decode_into(out)
            .map_err(|_| DecodeError::Base64)?;
        Ok(&out[..n])
    }
}

pub fn decode<'a>(envelope: &[u8], s: &'a mut Scratch) -> Result<(Meta, &'a [u8]), DecodeError> {
    s.arena.reset();
    let s: &'a Scratch = s;
    let mid = s.b64_decode(envelope)?;
    if mid.len() < 2 {
        return Err(DecodeError::Length);
    }
    let meta_len = u16::from_be_bytes([mid[0], mid[1]]) as usize;
    if meta_len == 0 || meta_len > MAX_META || 2 + meta_len > mid.len() {
        return Err(DecodeError::Meta);
    }
    let meta = &mid[2..2 + meta_len];
    let rest = &mid[2 + meta_len..];
    if rest.len() < DIGEST_LEN {
        return Err(DecodeError::Digest);
    }
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&rest[..DIGEST_LEN]);
    let payload = s.b64_decode(&rest[DIGEST_LEN..])?;
    if payload.len() > MAX_PAYLOAD {
        return Err(DecodeError::Size);
    }
    if core_utils::sha256(payload) != digest {
        return Err(DecodeError::Digest);
    }
    let kind = TaskKind::parse(meta.find_token(b"\"alg\":\"", b'"', 24).unwrap_or(b""));
    if let Some(l) = meta.find_u32(b"\"len\":")
        && l as usize != payload.len()
    {
        return Err(DecodeError::Size);
    }
    Ok((
        Meta {
            kind,
            len: payload.len() as u32,
        },
        payload,
    ))
}
