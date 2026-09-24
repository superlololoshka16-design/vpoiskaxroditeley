use crate::input::event::{RawEvent, kind as input};
use session_state::Persona;
use core_utils::SplitMix64Rng;
use core_utils::rng::seeds;

const MIN_KEY_INTERVAL_MS: u32 = 45;
const MAX_KEY_INTERVAL_MS: u32 = 680;
const HOLD_MIN_MS: u32 = 25;
const HOLD_MAX_MS: u32 = 160;
const BACKSPACE_KEY: u16 = 8;

enum KeyPhase {
    Down(char),
    Up(char),
    BackDown(char),
    BackUp(char),
    Done,
}

pub fn bigram_latency_ms(prev: char, cur: char) -> u32 {
    let p = prev.to_ascii_lowercase();
    let c = cur.to_ascii_lowercase();
    match (p, c) {
        ('t', 'h') => 80,
        ('h', 'e') => 85,
        ('i', 'n') => 90,
        ('e', 'r') => 88,
        ('a', 'n') => 92,
        ('r', 'e') => 90,
        ('o', 'n') => 95,
        ('a', 't') => 92,
        ('e', 'n') => 90,
        ('n', 'd') => 95,
        ('t', 'i') => 88,
        ('e', 's') => 90,
        ('o', 'r') => 92,
        ('t', 'e') => 88,
        ('o', 'f') => 95,
        ('e', 'd') => 90,
        ('i', 's') => 88,
        ('i', 't') => 90,
        ('a', 'l') => 92,
        ('a', 'r') => 90,
        ('s', 't') => 85,
        ('t', 'o') => 88,
        ('n', 't') => 90,
        ('n', 'g') => 92,
        ('s', 'e') => 90,
        ('h', 'a') => 92,
        ('a', 's') => 90,
        ('o', 'u') => 95,
        ('i', 'o') => 100,
        ('l', 'e') => 92,
        ('n', 'o') => 95,
        ('u', 's') => 95,
        ('c', 'o') => 100,
        ('m', 'e') => 92,
        ('d', 'e') => 95,
        ('h', 'i') => 95,
        ('r', 'i') => 95,
        ('r', 'o') => 95,
        ('i', 'c') => 100,
        ('n', 'e') => 92,
        ('e', 'a') => 95,
        ('r', 'a') => 95,
        ('c', 'e') => 100,
        ('q', 'x')
        | ('z', 'j')
        | ('k', 'x')
        | ('j', 'x')
        | ('q', 'z')
        | ('x', 'z')
        | ('j', 'q')
        | ('v', 'k')
        | ('b', 'x')
        | ('p', 'x')
        | ('z', 'z')
        | ('q', 'q')
        | ('x', 'x') => 200,
        _ => 120,
    }
}

#[inline]
fn key_ev(c: char, kind: u8) -> RawEvent {
    RawEvent::new(c as u16, 0, 0, kind, u8::from(is_shifted_char(c)))
}

#[inline]
fn is_shifted_char(c: char) -> bool {
    c.is_ascii_uppercase()
        || matches!(
            c,
            '!' | '@'
                | '#'
                | '$'
                | '%'
                | '^'
                | '&'
                | '*'
                | '('
                | ')'
                | '_'
                | '+'
                | '{'
                | '}'
                | '|'
                | ':'
                | '"'
                | '<'
                | '>'
                | '?'
                | '~'
        )
}

pub struct TypingCursor {
    persona: Persona,
    chars: Vec<char>,
    idx: usize,
    prev: char,
    phase: KeyPhase,
    burst_left: u32,
    next_due_us: u64,
    finished: bool,
    rng: SplitMix64Rng,
}

impl TypingCursor {
    pub fn new(persona: Persona, text: &str, now_us: u64, seed: u64) -> Self {
        let mut rng = SplitMix64Rng::new(seed ^ seeds::SALT_TYPING_RNG);
        let wpm_scale = 60.0 / persona.wpm.max(10.0);
        let phase = match text.chars().next() {
            Some(c) => KeyPhase::Down(c),
            None => KeyPhase::Done,
        };
        Self {
            persona,
            chars: text.chars().collect(),
            idx: 0,
            prev: ' ',
            phase,
            burst_left: persona.burst_len,
            next_due_us: now_us + rng.lognormal_us(240.0 * wpm_scale, 0.35),
            finished: false,
            rng,
        }
    }

    #[inline]
    pub fn done(&self) -> bool {
        self.finished
    }

    #[inline]
    pub fn next_due_us(&self) -> u64 {
        self.next_due_us
    }

    #[inline]
    fn key_hold_us(&mut self, median_ms: f64) -> u64 {
        self.rng
            .lognormal_ms(median_ms, 0.28)
            .clamp(HOLD_MIN_MS, HOLD_MAX_MS) as u64
            * 1000
    }

    pub fn step(&mut self, now_us: u64) -> Option<RawEvent> {
        if self.finished || now_us < self.next_due_us {
            return None;
        }
        let wpm_scale = 60.0 / self.persona.wpm.max(10.0);
        match self.phase {
            KeyPhase::Done => {
                self.finished = true;
                None
            }
            KeyPhase::Down(c) => {
                self.phase = KeyPhase::Up(c);
                self.next_due_us = now_us + self.key_hold_us(62.0);
                Some(key_ev(c, input::KEY_DOWN))
            }
            KeyPhase::Up(c) => {
                self.phase = self.advance(c, wpm_scale, now_us);
                Some(key_ev(c, input::KEY_UP))
            }
            KeyPhase::BackDown(c) => {
                self.phase = KeyPhase::BackUp(c);
                self.next_due_us = now_us + self.key_hold_us(58.0);
                Some(RawEvent::new(BACKSPACE_KEY, 0, 0, input::KEY_DOWN, 0))
            }
            KeyPhase::BackUp(c) => {
                self.phase = KeyPhase::Down(c);
                let base = bigram_latency_ms(self.prev, c) as f64 * wpm_scale;
                let dt = self.rng.lognormal_us(base, self.persona.typing_sigma_ln);
                self.next_due_us = now_us + dt;
                Some(RawEvent::new(BACKSPACE_KEY, 0, 0, input::KEY_UP, 0))
            }
        }
    }

    fn advance(&mut self, c: char, wpm_scale: f64, now_us: u64) -> KeyPhase {
        if self.idx > 0 && self.rng.chance(self.persona.backspace_p) {
            return KeyPhase::BackDown(c);
        }
        self.prev = c;
        self.idx += 1;
        self.burst_left = self.burst_left.saturating_sub(1);
        let base = match self.chars.get(self.idx) {
            Some(&next) => bigram_latency_ms(self.prev, next) as f64 * wpm_scale,
            None => return KeyPhase::Done,
        };
        let mut dt = self.rng.lognormal_ms(base, self.persona.typing_sigma_ln);
        dt = dt.clamp(MIN_KEY_INTERVAL_MS, MAX_KEY_INTERVAL_MS);
        if self.burst_left == 0 {
            let pause = self
                .rng
                .lognormal_ms(self.persona.burst_pause_median_ms, 0.35);
            dt = dt.saturating_add(pause);
            self.burst_left = self.persona.burst_len;
        }
        self.next_due_us = now_us + dt as u64 * 1000;
        KeyPhase::Down(self.chars[self.idx])
    }
}

crate::input_cursor!(TypingCursor);
