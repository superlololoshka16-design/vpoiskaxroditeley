use crate::webidl::view_children;
use crate::worker::{DOC_GENERATION, with_doc};
use parser_pipeline::dom::TAG_UNKNOWN;
use parser_pipeline::dom::{node_flags, tags};
use std::cell::RefCell;
use taffy::TaffyTree;
use taffy::prelude::*;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Box2 {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Box2 {
    const ZERO: Box2 = Box2 {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    };
}

const LINE_H: f32 = 18.4;
const MAX_TEXT_W: f32 = 4096.0;

struct LayoutState {
    key: u64,
    boxes: Vec<Box2>,
    overlay_boxes: Vec<Box2>,
}

thread_local! {
    static LAYOUT: RefCell<Option<LayoutState>> = const { RefCell::new(None) };
}

pub(crate) fn clear() {
    LAYOUT.with(|l| *l.borrow_mut() = None);
}

fn char_w(c: char) -> f32 {
    if c.is_ascii_uppercase() {
        8.2
    } else if c.is_ascii_lowercase() || c.is_ascii_digit() {
        6.5
    } else if c == ' ' {
        3.6
    } else if (c as u32) >= 0x2E80 {
        12.0
    } else {
        4.2
    }
}

fn text_width(t: &str) -> f32 {
    t.chars().map(char_w).sum::<f32>().min(MAX_TEXT_W)
}

fn block_margin(tag: u16) -> (f32, f32) {
    match tag {
        tags::P => (16.0, 16.0),
        tags::H1 => (21.4, 21.4),
        tags::H2 => (19.7, 19.7),
        tags::H3 => (17.8, 17.8),
        tags::H4 => (15.2, 15.2),
        tags::H5 => (13.6, 13.6),
        tags::H6 => (12.3, 12.3),
        tags::OL | tags::UL | tags::BLOCKQUOTE => (16.0, 16.0),
        tags::FORM => (12.0, 12.0),
        _ => (0.0, 0.0),
    }
}

fn view_or_doc<T>(
    idx: u32,
    view: impl FnOnce(u32) -> T,
    dom: impl FnOnce(&parser_pipeline::PageData) -> T,
) -> Option<T> {
    if crate::webidl::is_overlay_id(idx) {
        return Some(view(idx));
    }
    with_doc(|d| d.map(dom))
}

fn tag_id_of(idx: u32) -> u16 {
    view_or_doc(idx, crate::webidl::view_tag_id, |d| d.dom.tag_id(idx)).unwrap_or(TAG_UNKNOWN)
}

fn flags_of(idx: u32) -> u8 {
    view_or_doc(idx, crate::webidl::view_flags, |d| d.dom.flags(idx)).unwrap_or(0)
}

fn is_leaf_tag(tag: u16) -> bool {
    matches!(
        tag,
        tags::CANVAS
            | tags::IMG
            | tags::IFRAME
            | tags::INPUT
            | tags::TEXTAREA
            | tags::SELECT
            | tags::BUTTON
    )
}

fn leaf_size(idx: u32) -> Size<Dimension> {
    let tag = tag_id_of(idx);
    let (w, h) = match tag {
        tags::CANVAS => (300.0, 150.0),
        tags::IMG | tags::IFRAME => {
            let w = crate::webidl::view_attr_f32(idx, "width").unwrap_or(0.0);
            let h = crate::webidl::view_attr_f32(idx, "height").unwrap_or(0.0);
            (w, h)
        }
        tags::INPUT | tags::TEXTAREA | tags::SELECT | tags::BUTTON => {
            let w = crate::webidl::view_attr_f32(idx, "width").unwrap_or(173.0);
            let h = crate::webidl::view_attr_f32(idx, "height").unwrap_or(21.0);
            (w, h)
        }
        _ => (0.0, 0.0),
    };
    Size {
        width: Dimension::length(w),
        height: Dimension::length(h),
    }
}

fn is_inline_ish(tag: u16) -> bool {
    matches!(
        tag,
        tags::A
            | tags::SPAN
            | tags::B
            | tags::I
            | tags::EM
            | tags::STRONG
            | tags::CODE
            | tags::SAMP
            | tags::LABEL
            | tags::CITE
            | tags::VAR
            | tags::SUB
            | tags::U
    )
}

#[inline]
pub(crate) fn is_inline_node(idx: u32) -> bool {
    is_inline_ish(tag_id_of(idx))
}

fn style_for(idx: u32) -> Style {
    let tag = tag_id_of(idx);
    let mut st = Style::default();
    if flags_of(idx) & node_flags::HIDDEN != 0 || crate::webidl::hidden_flag_of(idx) {
        st.display = Display::None;
        return st;
    }
    if is_inline_ish(tag) {
        st.size = Size {
            width: Dimension::auto(),
            height: Dimension::auto(),
        };
        return st;
    }
    let (mt, mb) = block_margin(tag);
    let m = |v: f32| LengthPercentageAuto::length(v);
    st.margin = Rect {
        top: m(mt),
        bottom: m(mb),
        left: m(0.0),
        right: m(0.0),
    };
    st.size.width = Dimension::percent(1.0);
    if tag == tags::HTML {
        st.size = Size {
            width: Dimension::percent(1.0),
            height: Dimension::percent(1.0),
        };
    } else if tag == tags::BODY {
        st.margin = Rect {
            top: m(8.0),
            bottom: m(8.0),
            left: m(8.0),
            right: m(8.0),
        };
    } else if tag == tags::OL || tag == tags::UL {
        let p = |v: f32| LengthPercentage::length(v);
        st.padding = Rect {
            top: p(0.0),
            bottom: p(0.0),
            left: p(40.0),
            right: p(40.0),
        };
    }
    st
}

fn text_style(t: &str) -> Style {
    Style {
        size: Size {
            width: Dimension::length(text_width(t)),
            height: Dimension::length(LINE_H),
        },
        ..Style::default()
    }
}

fn store_node(
    map: &mut Vec<Option<taffy::NodeId>>,
    overlay_map: &mut Vec<Option<taffy::NodeId>>,
    overlay: bool,
    idx: u32,
    n: taffy::NodeId,
) {
    if overlay {
        overlay_put(overlay_map, idx, n);
    } else if (idx as usize) < map.len() {
        map[idx as usize] = Some(n);
    }
}

fn build_node(
    tree: &mut TaffyTree,
    idx: u32,
    map: &mut Vec<Option<taffy::NodeId>>,
    overlay_map: &mut Vec<Option<taffy::NodeId>>,
) -> Option<taffy::NodeId> {
    let overlay = idx >= crate::webidl::OVERLAY_BASE;
    match crate::webidl::view_node_type(idx) {
        3 => {
            let t = crate::webidl::view_text(idx).unwrap_or_default();
            let n = tree.new_leaf(text_style(t.as_str())).ok()?;
            store_node(map, overlay_map, overlay, idx, n);
            return Some(n);
        }
        0 => {
            let n = tree.new_leaf(Style::default()).ok()?;
            if overlay {
                overlay_put(overlay_map, idx, n);
            }
            return Some(n);
        }
        _ => {}
    }
    let kids: Vec<taffy::NodeId> = view_children(idx)
        .into_iter()
        .filter_map(|c| build_node(tree, c, map, overlay_map))
        .collect();
    let st = style_for(idx);
    let n = if kids.is_empty() {
        let mut st = st;
        if is_leaf_tag(tag_id_of(idx)) {
            st.size = leaf_size(idx);
        }
        tree.new_leaf(st).ok()?
    } else {
        tree.new_with_children(st, &kids).ok()?
    };
    store_node(map, overlay_map, overlay, idx, n);
    Some(n)
}

fn taffy_boxes(tree: &taffy::TaffyTree, map: &[Option<taffy::NodeId>]) -> Vec<Box2> {
    let mut boxes = vec![Box2::ZERO; map.len()];
    for (i, node) in map.iter().enumerate() {
        if let Some(node) = node
            && let Ok(l) = tree.layout(*node)
        {
            boxes[i] = Box2 {
                x: l.location.x as f64,
                y: l.location.y as f64,
                w: l.size.width as f64,
                h: l.size.height as f64,
            };
        }
    }
    boxes
}

#[inline]
fn overlay_put(overlay_map: &mut Vec<Option<taffy::NodeId>>, idx: u32, n: taffy::NodeId) {
    let slot = (idx - crate::webidl::OVERLAY_BASE) as usize;
    if overlay_map.len() <= slot {
        overlay_map.resize(slot + 1, None);
    }
    overlay_map[slot] = Some(n);
}

fn ensure() -> bool {
    let key = crate::webidl::view_epoch(DOC_GENERATION.with(std::cell::Cell::get));
    let needs_rebuild = LAYOUT.with(|l| l.borrow().as_ref().is_none_or(|s| s.key != key));
    if !needs_rebuild {
        return true;
    }
    let built = with_doc(|doc| -> Option<()> {
        let page = doc?;
        let (vw, vh) = crate::worker::viewport();
        let mut tree = TaffyTree::new();
        let mut map: Vec<Option<taffy::NodeId>> = vec![None; page.dom.max_index()];
        let mut overlay_map: Vec<Option<taffy::NodeId>> = Vec::new();
        let mut roots = Vec::new();
        for r in view_children(u32::MAX) {
            if let Some(n) = build_node(&mut tree, r, &mut map, &mut overlay_map) {
                roots.push(n);
            }
        }
        let root = tree
            .new_with_children(
                Style {
                    size: Size {
                        width: Dimension::length(vw as f32),
                        height: Dimension::length(vh as f32),
                    },
                    ..Style::default()
                },
                &roots,
            )
            .ok()?;
        tree.compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(vw as f32),
                height: AvailableSpace::Definite(vh as f32),
            },
        )
        .ok()?;
        let boxes = taffy_boxes(&tree, &map);
        let overlay_boxes = taffy_boxes(&tree, &overlay_map);
        LAYOUT.with(|l| {
            *l.borrow_mut() = Some(LayoutState {
                key,
                boxes,
                overlay_boxes,
            })
        });
        Some(())
    });
    built.is_some()
}

pub(crate) fn rect_for(node: u32) -> Box2 {
    if !ensure() {
        return Box2::ZERO;
    }
    LAYOUT.with(|l| {
        l.borrow()
            .as_ref()
            .and_then(|s| {
                if crate::webidl::is_overlay_id(node) {
                    s.overlay_boxes
                        .get((node - crate::webidl::OVERLAY_BASE) as usize)
                        .copied()
                } else {
                    s.boxes.get(node as usize).copied()
                }
            })
            .unwrap_or(Box2::ZERO)
    })
}

pub(crate) fn element_from_point(x: f64, y: f64) -> Option<u32> {
    if !ensure() {
        return None;
    }
    let mut best: Option<(f64, u32)> = None;
    let consider = |best: &mut Option<(f64, u32)>, idx: u32, b: &Box2, visible: bool| {
        if visible && b.w > 0.0 && b.h > 0.0 && x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h
        {
            let area = b.w * b.h;
            if best.is_none_or(|(ba, _)| area < ba) {
                *best = Some((area, idx));
            }
        }
    };
    LAYOUT.with(|l| {
        let state = l.borrow();
        let state = state.as_ref()?;
        for (i, b) in state.boxes.iter().enumerate() {
            let idx = i as u32;
            let f = flags_of(idx);
            consider(&mut best, idx, b, f & node_flags::ELEMENT != 0 && f & node_flags::HIDDEN == 0);
        }
        for (i, b) in state.overlay_boxes.iter().enumerate() {
            let idx = crate::webidl::OVERLAY_BASE + i as u32;
            consider(&mut best, idx, b, true);
        }
        Some(())
    });
    best.map(|(_, idx)| idx)
}
