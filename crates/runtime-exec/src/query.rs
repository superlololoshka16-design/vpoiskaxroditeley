use crate::webidl::{view_attr_eq, view_attr_is_some, view_parent, view_tag_name_ci};
use smallvec::SmallVec;
use std::cell::RefCell;
use std::sync::Arc;

const RESULT_CAP: usize = 256;

thread_local! {
    static ALL_ELEMS: RefCell<Option<(u64, Arc<SmallVec<[u32; 512]>>)>> = const { RefCell::new(None) };
}

#[inline]
fn elems_key() -> u64 {
    crate::webidl::view_epoch(crate::worker::DOC_GENERATION.with(std::cell::Cell::get))
}

fn all_elements() -> Arc<SmallVec<[u32; 512]>> {
    let key = elems_key();
    if let Some(v) = ALL_ELEMS.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| Arc::clone(v))
    }) {
        return v;
    }
    let mut all: SmallVec<[u32; 512]> = SmallVec::new();
    crate::webidl::walk_elements_into(&mut all);
    let all = Arc::new(all);
    ALL_ELEMS.with(|c| {
        *c.borrow_mut() = Some((key, Arc::clone(&all)));
    });
    all
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartKind {
    Tag,
    Id,
    Class,
    Attr,
    AttrEq,
}

#[derive(Debug, Clone, Copy)]
struct Part<'a> {
    kind: PartKind,
    value: &'a str,
    value2: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Combinator {
    Descendant,
    Child,
}

#[derive(Debug, Clone)]
struct Compound<'a> {
    parts: SmallVec<[Part<'a>; 4]>,
}

#[derive(Debug, Clone)]
struct Selector<'a> {
    chain: SmallVec<[(Compound<'a>, Combinator); 4]>,
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':'
}

fn take_name<'a>(b: &'a [u8], s: &'a str, i: &mut usize) -> Option<&'a str> {
    let start = *i;
    let mut j = start;
    while j < b.len() && is_name_byte(b[j]) {
        j += 1;
    }
    if j == start {
        return None;
    }
    *i = j;
    Some(&s[start..j])
}

fn parse_compound(s: &str) -> Option<Compound<'_>> {
    let mut parts: SmallVec<[Part; 4]> = SmallVec::new();
    let b = s.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'#' => {
                i += 1;
                let name = take_name(b, s, &mut i)?;
                parts.push(Part {
                    kind: PartKind::Id,
                    value: name,
                    value2: "",
                });
            }
            b'.' => {
                i += 1;
                let name = take_name(b, s, &mut i)?;
                parts.push(Part {
                    kind: PartKind::Class,
                    value: name,
                    value2: "",
                });
            }
            b'[' => {
                let close = s[i..].find(']')? + i;
                let inner = &s[i + 1..close];
                if let Some(eq) = inner.find('=') {
                    let name = inner[..eq].trim();
                    let mut val = inner[eq + 1..].trim();
                    if val.len() >= 2
                        && ((val.starts_with('"') && val.ends_with('"'))
                            || (val.starts_with('\'') && val.ends_with('\'')))
                    {
                        val = &val[1..val.len() - 1];
                    }
                    if name.is_empty() {
                        return None;
                    }
                    parts.push(Part {
                        kind: PartKind::AttrEq,
                        value: name,
                        value2: val,
                    });
                } else {
                    let name = inner.trim();
                    if name.is_empty() {
                        return None;
                    }
                    parts.push(Part {
                        kind: PartKind::Attr,
                        value: name,
                        value2: "",
                    });
                }
                i = close + 1;
            }
            _ => {
                let name = take_name(b, s, &mut i)?;
                parts.insert(
                    0,
                    Part {
                        kind: PartKind::Tag,
                        value: name,
                        value2: "",
                    },
                );
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(Compound { parts })
}

fn parse_selector(q: &str) -> Option<Selector<'_>> {
    let q = q.trim();
    if q.is_empty() || q.len() > 256 {
        return None;
    }
    let mut chain: SmallVec<[(Compound, Combinator); 4]> = SmallVec::new();
    let mut direct = false;
    let mut seen = false;
    for seg in q.split_whitespace() {
        if seg == ">" {
            if !seen {
                return None;
            }
            direct = true;
            continue;
        }
        if seg
            .chars()
            .any(|c| c == ',' || c == '+' || c == '~' || c == '*')
        {
            return None;
        }
        let compound = parse_compound(seg)?;
        let comb = if !seen {
            Combinator::Descendant
        } else if direct {
            direct = false;
            Combinator::Child
        } else {
            Combinator::Descendant
        };
        seen = true;
        chain.push((compound, comb));
    }
    if chain.is_empty() {
        return None;
    }
    Some(Selector { chain })
}

fn match_compound(node: u32, c: &Compound<'_>) -> bool {
    for p in &c.parts {
        let ok = match p.kind {
            PartKind::Tag => view_tag_name_ci(node, p.value),
            PartKind::Id => view_attr_eq(node, "id", p.value),
            PartKind::Class => crate::webidl::view_attr_contains_word(node, "class", p.value),
            PartKind::Attr => view_attr_is_some(node, p.value),
            PartKind::AttrEq => view_attr_eq(node, p.value, p.value2),
        };
        if !ok {
            return false;
        }
    }
    true
}

fn chain_ok(node: u32, sel: &Selector, depth: usize) -> bool {
    if !match_compound(node, &sel.chain[depth].0) {
        return false;
    }
    if depth == 0 {
        return true;
    }
    match sel.chain[depth].1 {
        Combinator::Child => match view_parent(node) {
            Some(p) => chain_ok(p, sel, depth - 1),
            None => false,
        },
        Combinator::Descendant => {
            let mut cur = view_parent(node);
            while let Some(p) = cur {
                if chain_ok(p, sel, depth - 1) {
                    return true;
                }
                cur = view_parent(p);
            }
            false
        }
    }
}

fn collect_matches_into(
    all: impl IntoIterator<Item = u32>,
    sel: &Selector,
    out: &mut SmallVec<[u32; 16]>,
) {
    let max_depth = sel.chain.len() - 1;
    for n in all {
        if chain_ok(n, sel, max_depth) {
            out.push(n);
            if out.len() >= RESULT_CAP {
                break;
            }
        }
    }
}

fn select_all_view(sel: &Selector) -> SmallVec<[u32; 16]> {
    let mut out: SmallVec<[u32; 16]> = SmallVec::new();
    collect_matches_into(all_elements().iter().copied(), sel, &mut out);
    out
}

pub(crate) fn select_all(query: &str) -> Option<SmallVec<[u32; 16]>> {
    let sel = parse_selector(query)?;
    Some(select_all_view(&sel))
}

pub(crate) fn select_all_under_view(root: u32, query: &str) -> Option<SmallVec<[u32; 16]>> {
    let sel = parse_selector(query)?;
    let mut all: SmallVec<[u32; 512]> = SmallVec::new();
    crate::webidl::walk_subtree_into(root, &mut all);
    let mut out: SmallVec<[u32; 16]> = SmallVec::new();
    collect_matches_into(all, &sel, &mut out);
    Some(out)
}

pub(crate) fn select_first(query: &str) -> Option<u32> {
    select_all(query).and_then(|v| v.first().copied())
}

fn scan_tag_into(tag: &str, cap: usize, out: &mut SmallVec<[u32; 32]>) {
    for n in all_elements().iter() {
        if out.len() >= cap {
            return;
        }
        if view_tag_name_ci(*n, tag) {
            out.push(*n);
        }
    }
}

pub(crate) fn find_first_by_tag_view(tag: &str) -> Option<u32> {
    all_elements()
        .iter()
        .copied()
        .find(|&n| view_tag_name_ci(n, tag))
        .or_else(|| crate::webidl::overlay_first_tag(tag))
}

pub(crate) fn collect_by_tag_view_into(tag: &str, out: &mut SmallVec<[u32; 32]>) {
    if out.len() >= RESULT_CAP {
        return;
    }
    scan_tag_into(tag, RESULT_CAP, out);
}
