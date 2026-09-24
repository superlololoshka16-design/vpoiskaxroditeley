use compact_str::CompactString;
use core_utils::{BytesExt, FxBuild, fx_map};
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::scratch::{NIL_SPAN, Span, push_str};

pub mod tags {
    macro_rules! tag_defs {
        ($($name:ident = $id:expr),* $(,)?) => {
            $(pub const $name: u16 = $id;)*
        };
    }
    tag_defs! {
        A = 1, ABBR = 2, ADDRESS = 3, AREA = 4, ARTICLE = 5, ASIDE = 6, AUDIO = 7,
        B = 8, BASE = 9, BDI = 10, BDO = 11, BLOCKQUOTE = 12, BODY = 13, BR = 14,
        BUTTON = 15, CANVAS = 16, CAPTION = 17, CITE = 18, CODE = 19, COL = 20,
        COLGROUP = 21, DATA = 22, DATALIST = 23, DD = 24, DEL = 25, DETAILS = 26,
        DFN = 27, DIALOG = 28, DIV = 29, DL = 30, DT = 31, EM = 32, EMBED = 33,
        FIELDSET = 34, FIGCAPTION = 35, FIGURE = 36, FOOTER = 37, FORM = 38,
        H1 = 39, H2 = 40, H3 = 41, H4 = 42, H5 = 43, H6 = 44, HEAD = 45, HEADER = 46,
        HGROUP = 47, HR = 48, HTML = 49, I = 50, IFRAME = 51, IMG = 52, INPUT = 53,
        INS = 54, KBD = 55, LABEL = 56, LEGEND = 57, LI = 58, LINK = 59, MAIN = 60,
        MAP = 61, MARK = 62, MENU = 63, META = 64, METER = 65, NAV = 66, NOSCRIPT = 67,
        OBJECT = 68, OL = 69, OPTGROUP = 70, OPTION = 71, OUTPUT = 72, P = 73,
        PARAM = 74, PICTURE = 75, PRE = 76, PROGRESS = 77, Q = 78, RP = 79, RT = 80,
        RUBY = 81, S = 82, SAMP = 83, SCRIPT = 84, SEARCH = 85, SECTION = 86,
        SELECT = 87, SLOT = 88, SMALL = 89, SOURCE = 90, SPAN = 91, STRONG = 92,
        STYLE = 93, SUB = 94, SUMMARY = 95, SUP = 96, TABLE = 97, TBODY = 98, TD = 99,
        TEMPLATE = 100, TEXTAREA = 101, TFOOT = 102, TH = 103, THEAD = 104, TIME = 105,
        TITLE = 106, TRACK = 107, TR = 108, U = 109, UL = 110, VAR = 111, VIDEO = 112,
        WBR = 113,
    }
}

macro_rules! tags_from_defs {
    ($($lit:literal => $name:ident),* $(,)?) => {
        phf::phf_map! {
            $($lit => tags::$name,)*
        }
    };
}

pub static TAGS: phf::Map<&'static str, u16> = tags_from_defs! {
    "a" => A, "abbr" => ABBR, "address" => ADDRESS, "area" => AREA, "article" => ARTICLE,
    "aside" => ASIDE, "audio" => AUDIO, "b" => B, "base" => BASE, "bdi" => BDI, "bdo" => BDO,
    "blockquote" => BLOCKQUOTE, "body" => BODY, "br" => BR, "button" => BUTTON,
    "canvas" => CANVAS, "caption" => CAPTION, "cite" => CITE, "code" => CODE, "col" => COL,
    "colgroup" => COLGROUP, "data" => DATA, "datalist" => DATALIST, "dd" => DD, "del" => DEL,
    "details" => DETAILS, "dfn" => DFN, "dialog" => DIALOG, "div" => DIV, "dl" => DL,
    "dt" => DT, "em" => EM, "embed" => EMBED, "fieldset" => FIELDSET,
    "figcaption" => FIGCAPTION, "figure" => FIGURE, "footer" => FOOTER, "form" => FORM,
    "h1" => H1, "h2" => H2, "h3" => H3, "h4" => H4, "h5" => H5, "h6" => H6,
    "head" => HEAD, "header" => HEADER, "hgroup" => HGROUP, "hr" => HR, "html" => HTML,
    "i" => I, "iframe" => IFRAME, "img" => IMG, "input" => INPUT, "ins" => INS, "kbd" => KBD,
    "label" => LABEL, "legend" => LEGEND, "li" => LI, "link" => LINK, "main" => MAIN,
    "map" => MAP, "mark" => MARK, "menu" => MENU, "meta" => META, "meter" => METER,
    "nav" => NAV, "noscript" => NOSCRIPT, "object" => OBJECT, "ol" => OL,
    "optgroup" => OPTGROUP, "option" => OPTION, "output" => OUTPUT, "p" => P,
    "param" => PARAM, "picture" => PICTURE, "pre" => PRE, "progress" => PROGRESS, "q" => Q,
    "rp" => RP, "rt" => RT, "ruby" => RUBY, "s" => S, "samp" => SAMP, "script" => SCRIPT,
    "search" => SEARCH, "section" => SECTION, "select" => SELECT, "slot" => SLOT,
    "small" => SMALL, "source" => SOURCE, "span" => SPAN, "strong" => STRONG,
    "style" => STYLE, "sub" => SUB, "summary" => SUMMARY, "sup" => SUP,
    "table" => TABLE, "tbody" => TBODY, "td" => TD, "template" => TEMPLATE,
    "textarea" => TEXTAREA, "tfoot" => TFOOT, "th" => TH, "thead" => THEAD, "time" => TIME,
    "title" => TITLE, "tr" => TR, "track" => TRACK, "u" => U, "ul" => UL, "var" => VAR,
    "video" => VIDEO, "wbr" => WBR,
};

pub const TAG_UNKNOWN: u16 = u16::MAX;
const DYNAMIC_TAG_BASE: u16 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId {
    pub index: u32,
    pub generation: u32,
}

static TAG_NAME_BY_ID: LazyLock<Vec<Option<&'static str>>> =
    LazyLock::new(|| invert(&TAGS, DYNAMIC_TAG_BASE));

static ATTR_NAME_BY_ID: LazyLock<Vec<Option<&'static str>>> =
    LazyLock::new(|| invert(&ATTR_NAMES, DYNAMIC_ATTR_BASE));

fn invert(map: &phf::Map<&'static str, u16>, cap: u16) -> Vec<Option<&'static str>> {
    let mut v: Vec<Option<&'static str>> = vec![None; cap as usize];
    for (name, id) in map.entries() {
        v[*id as usize] = Some(*name);
    }
    v
}

#[inline]
pub fn static_tag_name(tag: u16) -> Option<&'static str> {
    if tag == TAG_UNKNOWN || tag >= DYNAMIC_TAG_BASE {
        return None;
    }
    TAG_NAME_BY_ID[tag as usize]
}

#[inline]
pub fn static_attr_name(id: u16) -> Option<&'static str> {
    if id == ATTR_UNKNOWN || id >= DYNAMIC_ATTR_BASE {
        return None;
    }
    ATTR_NAME_BY_ID[id as usize]
}
#[inline]
pub fn is_void_tag(tag: u16) -> bool {
    matches!(
        tag,
        tags::AREA
            | tags::BASE
            | tags::BR
            | tags::COL
            | tags::EMBED
            | tags::HR
            | tags::IMG
            | tags::INPUT
            | tags::LINK
            | tags::META
            | tags::PARAM
            | tags::SOURCE
            | tags::TRACK
            | tags::WBR
    )
}

pub const ATTR_UNKNOWN: u16 = u16::MAX;

macro_rules! attrs_from_defs {
    ($($lit:literal => $name:ident),* $(,)?) => {
        phf::phf_map! {
            $($lit => attrs::$name,)*
        }
    };
}

pub static ATTR_NAMES: phf::Map<&'static str, u16> = attrs_from_defs! {
    "accept" => ACCEPT, "action" => ACTION, "alt" => ALT, "async" => ASYNC,
    "charset" => CHARSET, "checked" => CHECKED, "class" => CLASS, "cols" => COLS,
    "content" => CONTENT, "defer" => DEFER, "dir" => DIR, "disabled" => DISABLED,
    "for" => FOR, "headers" => HEADERS, "height" => HEIGHT, "href" => HREF, "id" => ID,
    "lang" => LANG, "loading" => LOADING, "max" => MAX, "maxlength" => MAXLENGTH,
    "media" => MEDIA, "method" => METHOD, "min" => MIN, "multiple" => MULTIPLE,
    "name" => NAME, "pattern" => PATTERN, "placeholder" => PLACEHOLDER,
    "property" => PROPERTY, "rel" => REL, "required" => REQUIRED, "rows" => ROWS,
    "sandbox" => SANDBOX, "scope" => SCOPE, "selected" => SELECTED, "shape" => SHAPE,
    "size" => SIZE, "sizes" => SIZES, "span" => SPAN, "src" => SRC, "srcdoc" => SRCDOC,
    "srclang" => SRCLANG, "srcset" => SRCSET, "start" => START, "step" => STEP,
    "style" => STYLE, "tabindex" => TABINDEX, "target" => TARGET, "title" => TITLE,
    "type" => TYPE, "usemap" => USEMAP, "value" => VALUE, "width" => WIDTH, "wrap" => WRAP,
    "role" => ROLE, "aria-hidden" => ARIA_HIDDEN, "aria-label" => ARIA_LABEL,
    "crossorigin" => CROSSORIGIN, "integrity" => INTEGRITY, "referrerpolicy" => REFERRERPOLICY,
    "nomodule" => NOMODULE, "kind" => KIND, "label" => LABEL, "open" => OPEN,
    "datetime" => DATETIME, "download" => DOWNLOAD, "hidden" => HIDDEN,
};

const DYNAMIC_ATTR_BASE: u16 = 100;

pub mod attrs {
    macro_rules! attr_defs {
        ($($name:ident = $id:expr),* $(,)?) => {
            $(pub const $name: u16 = $id;)*
        };
    }
    attr_defs! {
        ACCEPT = 1, ACTION = 2, ALT = 3, ASYNC = 4, CHARSET = 5, CHECKED = 6,
        CLASS = 7, COLS = 8, CONTENT = 9, DEFER = 10, DIR = 11, DISABLED = 12,
        FOR = 13, HEADERS = 14, HEIGHT = 15, HREF = 16, ID = 17, LANG = 18,
        LOADING = 19, MAX = 20, MAXLENGTH = 21, MEDIA = 22, METHOD = 23, MIN = 24,
        MULTIPLE = 25, NAME = 26, PATTERN = 27, PLACEHOLDER = 28, PROPERTY = 29,
        REL = 30, REQUIRED = 31, ROWS = 32, SANDBOX = 33, SCOPE = 34, SELECTED = 35,
        SHAPE = 36, SIZE = 37, SIZES = 38, SPAN = 39, SRC = 40, SRCDOC = 41,
        SRCLANG = 42, SRCSET = 43, START = 44, STEP = 45, STYLE = 46, TABINDEX = 47,
        TARGET = 48, TITLE = 49, TYPE = 50, USEMAP = 51, VALUE = 52, WIDTH = 53,
        WRAP = 54, ROLE = 55, ARIA_HIDDEN = 56, ARIA_LABEL = 57, CROSSORIGIN = 58,
        INTEGRITY = 59, REFERRERPOLICY = 60, NOMODULE = 61, KIND = 62, LABEL = 63,
        OPEN = 64, DATETIME = 65, DOWNLOAD = 66, HIDDEN = 67,
    }
}

const MAX_NODES: usize = 20_000;
const MAX_ATTRS: usize = 16_384;
const ID_INDEX_CAP: usize = 4096;
const POOL_BYTES: usize = 256 * 1024;
const TEXT_NODE_BYTES: usize = 4 * 1024;

#[inline]
pub(crate) fn is_hidden_type(value: &[u8]) -> bool {
    value.eq_ci(b"hidden")
}

pub mod node_flags {
    pub const ELEMENT: u8 = 1;
    pub const TEXT: u8 = 2;
    pub const HIDDEN: u8 = 4;
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Attr {
    pub name: u16,
    pub value: Span,
}

pub struct DomTree {
    parents: Vec<u32>,
    first_child: Vec<u32>,
    last_child: Vec<u32>,
    next_sibling: Vec<u32>,
    prev_sibling: Vec<u32>,
    tag_ids: Vec<u16>,
    flags: Vec<u8>,
    generations: Vec<u32>,
    spans: Vec<Span>,
    attr_start: Vec<u32>,
    attr_count: Vec<u8>,
    attrs: Vec<Attr>,
    pool: Vec<u8>,
    tag_names: Vec<CompactString>,
    attr_name_table: Vec<CompactString>,
    free: Vec<u32>,
    roots_head: u32,
    roots_tail: u32,
    truncated: bool,
    id_index: Vec<(u64, NodeId)>,
    pre_order: Vec<u32>,
    pub scripts: Vec<u32>,
    post_order: Vec<u32>,
    pub forms: Vec<u32>,
    pub inputs: Vec<u32>,
    pub title_node: Option<u32>,
    pub meta_desc_node: Option<u32>,
}

impl DomTree {
    pub fn last_script_node(&self) -> Option<u32> {
        self.scripts.last().copied()
    }


    pub fn new() -> Self {
        Self {
            parents: Vec::new(),
            first_child: Vec::new(),
            last_child: Vec::new(),
            next_sibling: Vec::new(),
            prev_sibling: Vec::new(),
            tag_ids: Vec::new(),
            flags: Vec::new(),
            generations: Vec::new(),
            spans: Vec::new(),
            attr_start: Vec::new(),
            attr_count: Vec::new(),
            attrs: Vec::new(),
            pool: Vec::new(),
            tag_names: Vec::new(),
            attr_name_table: Vec::new(),
            free: Vec::new(),
            roots_head: u32::MAX,
            roots_tail: u32::MAX,
            truncated: false,
            id_index: Vec::new(),
            pre_order: Vec::new(),
            scripts: Vec::new(),
            post_order: Vec::new(),
            forms: Vec::new(),
            inputs: Vec::new(),
            title_node: None,
            meta_desc_node: None,
        }
    }

    fn alloc_slot(&mut self) -> Option<u32> {
        if let Some(index) = self.free.pop() {
            Some(index)
        } else if self.parents.len() < MAX_NODES {
            Some(self.parents.len() as u32)
        } else {
            self.truncated = true;
            None
        }
    }

    fn push_slot(&mut self, parent: u32, tag: u16, flags: u8, span: Span) -> Option<u32> {
        let index = self.alloc_slot()?;
        let i = index as usize;
        if i == self.parents.len() {
            self.parents.push(u32::MAX);
            self.first_child.push(u32::MAX);
            self.last_child.push(u32::MAX);
            self.next_sibling.push(u32::MAX);
            self.prev_sibling.push(u32::MAX);
            self.tag_ids.push(TAG_UNKNOWN);
            self.flags.push(0);
            self.generations.push(0);
            self.spans.push(NIL_SPAN);
            self.attr_start.push(0);
            self.attr_count.push(0);
        }
        self.first_child[i] = u32::MAX;
        self.last_child[i] = u32::MAX;
        self.next_sibling[i] = u32::MAX;
        self.prev_sibling[i] = u32::MAX;
        self.tag_ids[i] = tag;
        self.flags[i] = flags;
        self.spans[i] = span;
        self.attr_start[i] = 0;
        self.attr_count[i] = 0;
        self.link_child(parent, index);
        Some(index)
    }

    #[inline]
    fn head_mut(&mut self, parent: u32) -> &mut u32 {
        if parent == u32::MAX {
            &mut self.roots_head
        } else {
            &mut self.first_child[parent as usize]
        }
    }

    #[inline]
    fn tail_mut(&mut self, parent: u32) -> &mut u32 {
        if parent == u32::MAX {
            &mut self.roots_tail
        } else {
            &mut self.last_child[parent as usize]
        }
    }

    fn link_child(&mut self, parent: u32, child: u32) {
        self.parents[child as usize] = parent;
        let prev = *self.tail_mut(parent);
        if prev == u32::MAX {
            *self.head_mut(parent) = child;
        } else {
            self.next_sibling[prev as usize] = child;
            self.prev_sibling[child as usize] = prev;
        }
        *self.tail_mut(parent) = child;
    }

    fn pool_put(&mut self, bytes: &[u8]) -> Span {
        let bytes = if bytes.len() > u16::MAX as usize {
            &bytes[..core_utils::floor_char_boundary_bytes(bytes, u16::MAX as usize)]
        } else {
            bytes
        };
        if self.pool.len() + bytes.len() > POOL_BYTES {
            self.truncated = true;
            return NIL_SPAN;
        }
        let off = self.pool.len() as u32;
        self.pool.extend_from_slice(bytes);
        Span {
            off,
            len: bytes.len() as u32,
        }
    }

    pub(crate) fn open_element(
        &mut self,
        parent: u32,
        tag: u16,
        attrs: &[(u16, &[u8])],
    ) -> Option<u32> {
        let node = self.push_slot(parent, tag, node_flags::ELEMENT, NIL_SPAN)?;
        let start = self.attrs.len() as u32;
        let mut count: u8 = 0;
        let mut hidden = false;
        let mut id_hash: Option<u64> = None;
        for (name, value) in attrs {
            if self.attrs.len() >= MAX_ATTRS {
                self.truncated = true;
                break;
            }
            let span = self.pool_put(value);
            if *name == attrs::TYPE {
                hidden = is_hidden_type(value);
            } else if *name == attrs::ID {
                id_hash = Some(core_utils::xxh3::hash(value));
            }
            self.attrs.push(Attr {
                name: *name,
                value: span,
            });
            count += 1;
        }
        let i = node as usize;
        self.attr_start[i] = start;
        self.attr_count[i] = count;
        if hidden {
            self.flags[i] |= node_flags::HIDDEN;
        }
        if let Some(h) = id_hash {
            let id = NodeId {
                index: node,
                generation: self.generations[i],
            };
            if self.id_index.len() < ID_INDEX_CAP {
                self.id_index.push((h, id));
            }
        }
        if tag == tags::SCRIPT {
            self.scripts.push(node);
        } else if tag == tags::FORM {
            self.forms.push(node);
        } else if matches!(
            tag,
            tags::INPUT | tags::SELECT | tags::TEXTAREA | tags::BUTTON
        ) {
            self.inputs.push(node);
        } else if tag == tags::TITLE && self.title_node.is_none() {
            self.title_node = Some(node);
        } else if tag == tags::META && self.meta_desc_node.is_none() {
            let named = attrs
                .iter()
                .any(|(n, v)| *n == attrs::NAME && v.eq_ci(b"description"));
            if named {
                self.meta_desc_node = Some(node);
            }
        }
        Some(node)
    }

    pub(crate) fn push_text(&mut self, parent: u32, text: &[u8]) {
        if parent == u32::MAX {
            return;
        }
        let cut = text.len().min(TEXT_NODE_BYTES);
        let cut = core_utils::floor_char_boundary_bytes(text, cut);
        let span = self.pool_put(&text[..cut]);
        let last = self.last_child[parent as usize];
        if last != u32::MAX {
            let il = last as usize;
            let adjacent = self.flags[il] & node_flags::TEXT != 0
                && self.spans[il].off as usize + self.spans[il].len as usize == span.off as usize;
            if adjacent {
                let merged = self.spans[il].len as usize + span.len as usize;
                if merged <= u16::MAX as usize {
                    self.spans[il].len = merged as u32;
                    return;
                }
            }
        }
        self.push_slot(parent, TAG_UNKNOWN, node_flags::TEXT, span);
    }

    fn unlink(&mut self, node: u32) {
        let i = node as usize;
        let prev = self.prev_sibling[i];
        let next = self.next_sibling[i];
        let parent = self.parents[i];
        if prev != u32::MAX {
            self.next_sibling[prev as usize] = next;
        } else {
            *self.head_mut(parent) = next;
        }
        if next != u32::MAX {
            self.prev_sibling[next as usize] = prev;
        } else {
            *self.tail_mut(parent) = prev;
        }
        self.prev_sibling[i] = u32::MAX;
        self.next_sibling[i] = u32::MAX;
    }

    fn free_node(&mut self, node: u32) {
        let i = node as usize;
        self.generations[i] = self.generations[i].wrapping_add(1);
        self.flags[i] = 0;
        self.tag_ids[i] = TAG_UNKNOWN;
        self.spans[i] = NIL_SPAN;
        self.attr_start[i] = 0;
        self.attr_count[i] = 0;
        self.parents[i] = u32::MAX;
        self.first_child[i] = u32::MAX;
        self.last_child[i] = u32::MAX;
        self.next_sibling[i] = u32::MAX;
        self.prev_sibling[i] = u32::MAX;
        self.free.push(node);
    }

    pub fn remove_node(&mut self, id: NodeId) -> bool {
        if !self.is_valid(id) {
            return false;
        }
        self.unlink(id.index);
        let mut node = id.index;
        loop {
            let child = self.first_child[node as usize];
            if child != u32::MAX {
                node = child;
                continue;
            }
            loop {
                let parent = self.parents[node as usize];
                let next = self.next_sibling[node as usize];
                self.free_node(node);
                if node == id.index {
                    break;
                }
                if next != u32::MAX {
                    node = next;
                    break;
                }
                node = parent;
            }
            if node == id.index {
                break;
            }
        }
        let flags = &self.flags;
        for index in [&mut self.scripts, &mut self.forms, &mut self.inputs] {
            index.retain(|&node| flags[node as usize] != 0);
        }
        if self
            .title_node
            .is_some_and(|node| flags[node as usize] == 0)
        {
            self.title_node = None;
        }
        if self
            .meta_desc_node
            .is_some_and(|node| flags[node as usize] == 0)
        {
            self.meta_desc_node = None;
        }
        self.compute_orders();
        true
    }

    #[inline]
    pub fn is_valid(&self, id: NodeId) -> bool {
        let i = id.index as usize;
        i < self.parents.len() && self.flags[i] != 0 && self.generations[i] == id.generation
    }

    pub fn node_id(&self, index: u32) -> NodeId {
        NodeId {
            index,
            generation: self.generations[index as usize],
        }
    }

    #[inline]
    pub fn tag_id(&self, index: u32) -> u16 {
        self.tag_ids[index as usize]
    }

    #[inline]
    pub fn flags(&self, index: u32) -> u8 {
        self.flags[index as usize]
    }

    pub fn parent(&self, index: u32) -> Option<u32> {
        let p = self.parents[index as usize];
        (p != u32::MAX).then_some(p)
    }

    pub fn len(&self) -> usize {
        self.parents.len() - self.free.len()
    }

    #[inline]
    pub fn max_index(&self) -> usize {
        self.parents.len()
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn tag_name(&self, tag: u16) -> Option<&str> {
        static_tag_name(tag).or_else(|| {
            self.tag_names
                .get((tag - DYNAMIC_TAG_BASE) as usize)
                .map(|s| s.as_str())
        })
    }

    pub fn text(&self, index: u32) -> Option<&str> {
        let i = index as usize;
        if self.flags[i] & node_flags::TEXT == 0 {
            return None;
        }
        Some(self.spans[i].get(&self.pool))
    }

    pub fn attr(&self, index: u32, name: u16) -> Option<&str> {
        let i = index as usize;
        let start = self.attr_start[i] as usize;
        let count = self.attr_count[i] as usize;
        for a in &self.attrs[start..start + count] {
            if a.name == name {
                return Some(a.value.get(&self.pool));
            }
        }
        None
    }

    pub fn attrs_of(&self, index: u32) -> impl Iterator<Item = (&str, &str)> {
        let i = index as usize;
        let start = self.attr_start[i] as usize;
        let count = self.attr_count[i] as usize;
        let pool = &self.pool;
        let attr_table = &self.attr_name_table;
        self.attrs[start..start + count]
            .iter()
            .filter_map(move |a| {
                let name = reverse_attr_name(a.name, attr_table)?;
                let value = a.value.get(pool);
                Some((name, value))
            })
    }

    pub fn children(&self, index: u32) -> ChildIter<'_> {
        if index == u32::MAX {
            return ChildIter {
                tree: self,
                next: self.roots_head,
            };
        }
        ChildIter {
            tree: self,
            next: self.first_child[index as usize],
        }
    }

    pub fn find_by_id(&self, id: &str) -> Option<u32> {
        let h = core_utils::xxh3::hash(id.as_bytes());
        let pos = self.id_index.binary_search_by(|(ih, _)| ih.cmp(&h)).ok()?;
        for &(ih, node) in &self.id_index[pos..] {
            if ih != h {
                break;
            }
            if self.is_valid(node) && self.attr(node.index, attrs::ID).is_some_and(|v| v == id) {
                return Some(node.index);
            }
        }
        None
    }

    pub fn script_attr(&self, i: usize, name: &str) -> Option<&str> {
        let node = *self.scripts.get(i)?;
        let name_id = ATTR_NAMES.get(name).copied()?;
        self.attr(node, name_id)
    }

    pub fn pool_len(&self) -> usize {
        self.pool.len()
    }

    pub fn attr_count_total(&self) -> usize {
        self.attrs.len()
    }

    pub(crate) fn attach_name_tables(
        &mut self,
        tag_names: Vec<CompactString>,
        attr_name_table: Vec<CompactString>,
    ) {
        self.id_index.sort_unstable_by_key(|(h, _)| *h);
        self.tag_names = tag_names;
        self.attr_name_table = attr_name_table;
        self.compute_orders();
    }

    fn compute_orders(&mut self) {
        let n = self.parents.len();
        self.pre_order.clear();
        self.pre_order.resize(n, u32::MAX);
        self.post_order.clear();
        self.post_order.resize(n, u32::MAX);
        let mut stack: Vec<(u32, bool)> = Vec::with_capacity(64);
        let mut kids: Vec<u32> = Vec::with_capacity(16);
        let mut tick: u32 = 0;
        let mut cur = self.roots_head;
        while cur != u32::MAX {
            kids.push(cur);
            cur = self.next_sibling[cur as usize];
        }
        for root in kids.drain(..).rev() {
            stack.push((root, false));
        }
        while let Some((node, exit)) = stack.pop() {
            let i = node as usize;
            if exit {
                self.post_order[i] = tick;
                tick = tick.wrapping_add(1);
                continue;
            }
            self.pre_order[i] = tick;
            tick = tick.wrapping_add(1);
            stack.push((node, true));
            kids.clear();
            let mut c = self.first_child[i];
            while c != u32::MAX {
                kids.push(c);
                c = self.next_sibling[c as usize];
            }
            for k in kids.drain(..).rev() {
                stack.push((k, false));
            }
        }
    }

    #[inline]
    pub fn is_descendant(&self, node: u32, root: u32) -> bool {
        if node == root {
            return true;
        }
        let ni = node as usize;
        let ri = root as usize;
        if ni >= self.parents.len() || ri >= self.parents.len() {
            return false;
        }
        if self.flags[ni] == 0 || self.flags[ri] == 0 {
            return false;
        }
        if self.pre_order.len() == self.parents.len() {
            return self.pre_order[ri] <= self.pre_order[ni]
                && self.post_order[ni] <= self.post_order[ri];
        }
        let mut cur = self.parents[ni];
        let mut hops = 0;
        while cur != u32::MAX {
            if cur == root {
                return true;
            }
            cur = self.parents[cur as usize];
            hops += 1;
            if hops > 512 {
                return false;
            }
        }
        false
    }

    #[inline]
    pub fn last_child(&self, index: u32) -> u32 {
        if index == u32::MAX {
            return self.roots_tail;
        }
        self.last_child[index as usize]
    }

    #[inline]
    pub fn next_sibling(&self, index: u32) -> u32 {
        if index == u32::MAX {
            return u32::MAX;
        }
        self.next_sibling[index as usize]
    }

    #[inline]
    pub fn prev_sibling(&self, index: u32) -> u32 {
        if index == u32::MAX {
            return u32::MAX;
        }
        self.prev_sibling[index as usize]
    }
}

pub struct ChildIter<'a> {
    tree: &'a DomTree,
    next: u32,
}

impl Iterator for ChildIter<'_> {
    type Item = u32;
    #[inline]
    fn next(&mut self) -> Option<u32> {
        if self.next == u32::MAX {
            return None;
        }
        let cur = self.next;
        self.next = self.tree.next_sibling[cur as usize];
        Some(cur)
    }
}

fn reverse_attr_name(name: u16, dynamic: &[CompactString]) -> Option<&str> {
    if name >= DYNAMIC_ATTR_BASE {
        if name == ATTR_UNKNOWN {
            return None;
        }
        return dynamic
            .get((name - DYNAMIC_ATTR_BASE) as usize)
            .map(|s| s.as_str());
    }
    ATTR_NAME_BY_ID[name as usize]
}

pub(crate) trait Names {
    fn builtin(name: &str) -> Option<u16>;
    const UNKNOWN: u16;
    const BASE: u16;
    const CAP: u16;
}

pub(crate) fn push_name<L: Names>(name: &str) -> (u16, Option<Span>) {
    match L::builtin(name) {
        Some(id) => (id, None),
        None => (L::UNKNOWN, Some(push_str(name))),
    }
}

pub(crate) fn resolve_name<L: Names>(
    id: u16,
    sp: Option<Span>,
    pool: &[u8],
    interner: &mut Interner<L>,
) -> Option<u16> {
    if id != L::UNKNOWN {
        return Some(id);
    }
    sp.map(|sp| interner.intern(sp.get(pool)))
}

pub(crate) struct Interner<L: Names> {
    dynamic: HashMap<CompactString, u16, FxBuild>,
    next: u16,
    _marker: core::marker::PhantomData<fn() -> L>,
}

impl<L: Names> Interner<L> {
    pub(crate) fn new() -> Self {
        Self {
            dynamic: fx_map(),
            next: L::BASE,
            _marker: core::marker::PhantomData,
        }
    }

    #[inline]
    pub(crate) fn intern(&mut self, name: &str) -> u16 {
        if let Some(id) = self.dynamic.get(name) {
            return *id;
        }
        if self.next < L::BASE + L::CAP {
            let id = self.next;
            self.next += 1;
            self.dynamic.insert(CompactString::new(name), id);
            id
        } else {
            L::UNKNOWN
        }
    }

    pub(crate) fn name_table(&mut self) -> Vec<CompactString> {
        let mut v: Vec<(u16, CompactString)> =
            self.dynamic.drain().map(|(k, id)| (id, k)).collect();
        v.sort_unstable_by_key(|(id, _)| *id);
        v.into_iter().map(|(_, k)| k).collect()
    }
}

pub(crate) struct TagNames;

impl Names for TagNames {
    #[inline]
    fn builtin(name: &str) -> Option<u16> {
        TAGS.get(name).copied()
    }
    const UNKNOWN: u16 = TAG_UNKNOWN;
    const BASE: u16 = DYNAMIC_TAG_BASE;
    const CAP: u16 = 500;
}

pub(crate) struct AttrNames;

impl Names for AttrNames {
    #[inline]
    fn builtin(name: &str) -> Option<u16> {
        ATTR_NAMES.get(name).copied()
    }
    const UNKNOWN: u16 = ATTR_UNKNOWN;
    const BASE: u16 = DYNAMIC_ATTR_BASE;
    const CAP: u16 = 200;
}

pub(crate) type TagInterner = Interner<TagNames>;
pub(crate) type AttrNameInterner = Interner<AttrNames>;

pub(crate) struct OpenStack {
    stack: SmallVec<[(u32, bool); 64]>,
}

impl OpenStack {
    pub(crate) fn new() -> Self {
        Self {
            stack: SmallVec::new(),
        }
    }

    #[inline]
    pub(crate) fn top(&self) -> Option<u32> {
        self.stack.last().map(|(n, _)| *n)
    }

    #[inline]
    pub(crate) fn top_has_id_or_class(&self) -> bool {
        self.stack.last().is_some_and(|(_, m)| *m)
    }

    #[inline]
    pub(crate) fn push(&mut self, node: u32, id_or_class: bool) {
        if self.stack.len() < 512 {
            self.stack.push((node, id_or_class));
        }
    }

    pub(crate) fn close(&mut self, tree: &DomTree, tag: u16) {
        let mut matched = None;
        for (from_top, (node, _)) in self.stack.iter().rev().enumerate() {
            if tree.tag_id(*node) == tag {
                matched = Some(from_top);
                break;
            }
        }
        let Some(matched) = matched else { return };
        for _ in 0..=matched {
            self.stack.pop();
        }
    }

    pub(crate) fn imply_close(&mut self, tree: &DomTree, opening: u16) {
        while let Some((node, _)) = self.stack.last() {
            if sibling_closes(opening, tree.tag_id(*node)) {
                self.stack.pop();
            } else {
                break;
            }
        }
    }
}

#[inline]
fn sibling_closes(opening: u16, top: u16) -> bool {
    match (opening, top) {
        (tags::P, tags::P) => true,
        (tags::LI, tags::LI) => true,
        (a, b) if (a == tags::DT || a == tags::DD) && (b == tags::DT || b == tags::DD) => true,
        (a, b) if (a == tags::TH || a == tags::TD) && (b == tags::TH || b == tags::TD) => true,
        (tags::TR, b) if b == tags::TD || b == tags::TH || b == tags::TR => true,
        (a, b) if a == tags::THEAD || a == tags::TBODY || a == tags::TFOOT => {
            b == tags::THEAD
                || b == tags::TBODY
                || b == tags::TFOOT
                || b == tags::TD
                || b == tags::TH
                || b == tags::TR
        }
        (tags::OPTION, tags::OPTION) => true,
        _ => false,
    }
}
