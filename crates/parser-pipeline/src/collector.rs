use crate::dom::{
    ATTR_UNKNOWN, TAG_UNKNOWN, AttrNameInterner, AttrNames, DomTree, OpenStack, TagInterner,
    TagNames, attrs, resolve_name, tags,
};
use crate::detector::{build_ac, is_challenge};
use crate::scratch::{AttrEv, Ev, ScriptEvKind};
use bytes::Bytes;
use compact_str::CompactString;
use core_utils::{BytesExt, truncate_str};
use serde::Deserialize;
use smallvec::SmallVec;
use std::collections::BTreeMap;
use std::sync::LazyLock;

const SCRIPT_SRC_CAP: usize = 24;
const CHALLENGE_MARKER_CAP: usize = 4;
const ANUBIS_CAP: usize = 8 * 1024;
const SCRIPT_CAP: usize = 256 * 1024;
const NEXT_DATA_CAP: usize = 2 * 1024 * 1024;
const MAX_FORMS: usize = 16;
const MAX_FIELDS: usize = 32;
const MAX_TOKENS: usize = 8;

#[derive(Deserialize)]
struct NextDataRaw<'a> {
    #[serde(default, borrow)]
    page: &'a str,
    #[serde(default, borrow, rename = "buildId")]
    build_id: &'a str,
}

fn parse_next_data(json: &[u8]) -> Result<NextData, core_utils::json::Error> {
    let raw: NextDataRaw<'_> = core_utils::json::from_slice(json)?;
    Ok(NextData {
        page: CompactString::new(raw.page),
        build_id: CompactString::new(raw.build_id),
    })
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub byte_brake: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            byte_brake: crate::BYTE_BRAKE,
        }
    }
}

struct ScriptCapture {
    kind: ScriptEvKind,
    buf: SmallVec<[u8; 2048]>,
    over: bool,
    cap: usize,
    node: Option<u32>,
}

#[inline]
fn is_target_tag(t: u16) -> bool {
    matches!(
        t,
        tags::HTML
            | tags::HEAD
            | tags::BODY
            | tags::TITLE
            | tags::SCRIPT
            | tags::STYLE
            | tags::LINK
            | tags::META
            | tags::NOSCRIPT
            | tags::FORM
            | tags::INPUT
            | tags::SELECT
            | tags::OPTION
            | tags::TEXTAREA
            | tags::BUTTON
            | tags::LABEL
            | tags::A
            | tags::IFRAME
            | tags::TEMPLATE
    )
}

pub(crate) struct Collector {
    title: Option<CompactString>,
    meta_description: Option<CompactString>,
    title_open: bool,
    title_saturated: bool,
    forms: SmallVec<[Form; 4]>,
    open_form: Option<usize>,
    tokens: SmallVec<[CompactString; 8]>,
    script_srcs: SmallVec<[CompactString; 12]>,
    inline_count: usize,
    challenge: Option<Bytes>,
    challenge_node: Option<u32>,
    challenge_markers: SmallVec<[CompactString; 4]>,
    challenge_script_url: Option<CompactString>,
    telemetry_route: Option<crate::telemetry::TelemetryRoute>,
    extract_keys: Vec<CompactString>,
    extracted_ids: Vec<Option<CompactString>>,
    next_data: Option<NextData>,
    anubis: Option<Bytes>,
    anubis_version: Option<CompactString>,
    captcha_family: Option<u8>,
    captcha_sitekey: Option<CompactString>,
    cur_script: Option<ScriptCapture>,
    parse_errors: u32,
    dom: DomTree,
    open: OpenStack,
    skip: Vec<bool>,
    tag_intern: TagInterner,
    attr_intern: AttrNameInterner,
}

impl Collector {
    pub(crate) fn note_scratch_overflow(&mut self) {
        self.parse_errors += 1;
    }

    pub(crate) fn new(extract_keys: Vec<CompactString>) -> Self {
        let extracted_ids = vec![None; extract_keys.len()];
        Self {
            title: None,
            meta_description: None,
            title_open: false,
            title_saturated: false,
            forms: SmallVec::new(),
            open_form: None,
            tokens: SmallVec::new(),
            script_srcs: SmallVec::new(),
            inline_count: 0,
            challenge: None,
            challenge_node: None,
            telemetry_route: None,
            challenge_markers: SmallVec::new(),
            challenge_script_url: None,
            extract_keys,
            extracted_ids,
            next_data: None,
            anubis: None,
            anubis_version: None,
            captcha_family: None,
            captcha_sitekey: None,
            cur_script: None,
            parse_errors: 0,
            dom: DomTree::new(),
            open: OpenStack::new(),
            skip: Vec::with_capacity(64),
            tag_intern: TagInterner::new(),
            attr_intern: AttrNameInterner::new(),
        }
    }

    fn finalize_script(&mut self) {
        let Some(cap) = self.cur_script.take() else {
            return;
        };
        match cap.kind {
            ScriptEvKind::NextData => {
                if cap.over || cap.buf.is_empty() {
                    self.parse_errors += 1;
                } else {
                    match parse_next_data(&cap.buf) {
                        Ok(nd) => self.next_data = Some(nd),
                        Err(_) => self.parse_errors += 1,
                    }
                }
            }
            ScriptEvKind::Inline => {
                if !cap.over && is_challenge(&cap.buf) {
                    self.challenge = Some(Bytes::from(cap.buf.into_vec()));
                    self.challenge_node = cap.node;
                } else {
                    self.inline_count += 1;
                }
            }
            ScriptEvKind::Anubis => {
                if !cap.over && !cap.buf.is_empty() && self.anubis.is_none() {
                    self.anubis = Some(Bytes::from(cap.buf.into_vec()));
                }
            }
            ScriptEvKind::AnubisVersion => {
                if self.anubis_version.is_none() && !cap.buf.is_empty() {
                    let s = core_utils::utf8::basic::from_utf8(&cap.buf)
                        .map(|s| {
                            s.trim().trim_matches('"')
                        })
                        .ok();
                    if let Some(s) = s {
                        self.anubis_version = Some(CompactString::from(truncate_str(s, 32)));
                    }
                }
            }
        }
    }

    fn finalize_pending(&mut self) {
        self.title_open = false;
        self.finalize_script();
        self.open_form = None;
        let srcs: SmallVec<[&str; 24]> = self.script_srcs.iter().map(|s| s.as_str()).collect();
        if self.telemetry_route.is_none() {
            let inline = self.challenge.as_deref().unwrap_or(&[]);
            self.telemetry_route = crate::telemetry::detect_route(&srcs, inline);
        }
        if self.captcha_family.is_none() {
            let markers: SmallVec<[&str; 8]> = self
                .challenge_markers
                .iter()
                .map(|m| m.as_str())
                .collect();
            if let Some(hit) = crate::detector::detect_widget(&srcs, &markers) {
                self.captcha_family = Some(hit.family.idx());
                if self.captcha_sitekey.is_none() {
                    self.captcha_sitekey = hit.sitekey;
                }
            }
        }
    }

    #[inline]
    fn dom_parent(&self) -> u32 {
        self.open.top().unwrap_or(u32::MAX)
    }


    pub(crate) fn apply(&mut self, pool: &[u8], attrs: &[AttrEv], ev: Ev) {
        match ev {
            Ev::DomOpen {
                tag,
                tag_dyn,
                attr_start,
                attr_count,
            } => {
                let tag_id = resolve_name::<TagNames>(tag, tag_dyn, pool, &mut self.tag_intern)
                    .unwrap_or(TAG_UNKNOWN);
                let a0 = attr_start as usize;
                let a1 = (a0 + attr_count as usize).min(attrs.len());
                let void = crate::dom::is_void_tag(tag_id);
                let mut marker = is_target_tag(tag_id);
                let mut id_or_class = false;
                let mut refs: SmallVec<[(u16, &[u8]); 16]> = SmallVec::new();
                for a in &attrs[a0..a1] {
                    let name_id =
                        resolve_name::<AttrNames>(a.name, a.name_dyn, pool, &mut self.attr_intern)
                            .unwrap_or(ATTR_UNKNOWN);
                    let dyn_name = a.name_dyn.map(|sp| sp.get(pool));
                    if name_id == attrs::ID || name_id == attrs::CLASS {
                        id_or_class = true;
                        marker = true;
                    } else if !marker && dyn_name.is_some_and(|n| n.as_bytes().starts_with(b"data-")) {
                        marker = true;
                    }
                    if self.captcha_sitekey.is_none() && dyn_name == Some("data-sitekey") {
                        let v = a.value.get(pool);
                        if crate::detector::sitekey_passes(v) {
                            self.captcha_sitekey = Some(CompactString::from(v));
                        }
                    }
                    refs.push((name_id, a.value.get(pool).as_bytes()));
                }
                if !marker {
                    if !void {
                        self.skip.push(true);
                    }
                    return;
                }
                self.open.imply_close(&self.dom, tag_id);
                let parent = self.dom_parent();
                if let Some(node) = self.dom.open_element(parent, tag_id, &refs)
                    && !void
                {
                    self.open.push(node, id_or_class);
                }
                if !void {
                    self.skip.push(false);
                }
            }
            Ev::DomText { span } => {
                let Some(top) = self.open.top() else { return };
                let t = self.dom.tag_id(top);
                if t == tags::SCRIPT {
                    return;
                }
                let text = span.get(pool);
                let always = matches!(t, tags::STYLE | tags::BUTTON | tags::LABEL | tags::A);
                if always {
                    self.dom.push_text(top, text.as_bytes());
                } else if self.open.top_has_id_or_class() {
                    self.dom.push_text(top, truncate_str(text, 128).as_bytes());
                }
            }
            Ev::DomClose { tag, tag_dyn } => {
                let Some(tag_id) =
                    resolve_name::<TagNames>(tag, tag_dyn, pool, &mut self.tag_intern)
                else {
                    return;
                };
                if self.skip.pop() == Some(true) {
                    return;
                }
                self.open.close(&self.dom, tag_id);
            }
            Ev::Script { src, kind } => {
                self.finalize_script();
                if let Some(src) = src {
                    if self.script_srcs.len() < SCRIPT_SRC_CAP {
                        self.script_srcs.push(CompactString::from(src.get(pool)));
                    }
                } else {
                    let cap = match kind {
                        ScriptEvKind::NextData => NEXT_DATA_CAP,
                        ScriptEvKind::Anubis => ANUBIS_CAP,
                        _ => SCRIPT_CAP,
                    };
                    self.cur_script = Some(ScriptCapture {
                        kind,
                        buf: SmallVec::new(),
                        over: false,
                        cap,
                        node: self.dom.last_script_node(),
                    });
                }
            }
            Ev::ScriptText { span, last } => {
                if let Some(cap) = self.cur_script.as_mut() {
                    let text = span.get(pool);
                    if cap.buf.len() + text.len() > cap.cap {
                        cap.over = true;
                    } else {
                        cap.buf.extend_from_slice(text.as_bytes());
                    }
                    if last {
                        self.finalize_script();
                    }
                }
            }
            Ev::TitleOpen => {
                self.title_open = true;
            }
            Ev::TitleText { span, last } => {
                if self.title_open {
                    let text = span.get(pool);
                    if !text.is_empty() && !self.title_saturated {
                        let title = self.title.get_or_insert_with(CompactString::default);
                        let remaining = 256usize.saturating_sub(title.len());
                        let head = truncate_str(text, remaining);
                        title.push_str(head);
                        self.title_saturated = head.len() < text.len() || title.len() == 256;
                    }
                    if last {
                        self.title_open = false;
                    }
                }
            }
            Ev::MetaDescription { span } => {
                if self.meta_description.is_none() {
                    let content = span.get(pool);
                    self.meta_description = Some(CompactString::from(truncate_str(content, 512)));
                }
            }
            Ev::Form { action, method } => {
                self.open_form = None;
                if self.forms.len() >= MAX_FORMS {
                    return;
                }
                let method = if method.len == 0 {
                    CompactString::const_new("get")
                } else {
                    CompactString::from(method.get(pool))
                };
                self.forms.push(Form {
                    action: action.map(|a| CompactString::from(a.get(pool))),
                    method,
                    fields: SmallVec::new(),
                });
                self.open_form = Some(self.forms.len() - 1);
            }
            Ev::Field {
                name,
                value,
                kind,
                hidden,
            } => {
                let name_str = name.get(pool);
                if hidden
                    && looks_like_token(name_str)
                    && let Some(v) = value
                {
                    let v = v.get(pool);
                    if self.tokens.len() < MAX_TOKENS && v.len() <= 512 {
                        self.tokens.push(CompactString::from(v));
                    }
                }
                if let Some(idx) = self.open_form {
                    let form = &mut self.forms[idx];
                    if form.fields.len() < MAX_FIELDS {
                        form.fields.push(FormData {
                            name: CompactString::from(name_str),
                            value: value.map(|v| CompactString::from(v.get(pool))),
                            kind: FieldKind::from_type_attr(kind.get(pool)),
                        });
                    }
                }
            }
            Ev::Extract { key, span } => {
                self.push_extract(key, span.get(pool), 512);
            }
            Ev::ChallengeDetected { marker } => {
                let url = marker.get(pool);
                if self.challenge_markers.len() < CHALLENGE_MARKER_CAP {
                    self.challenge_markers.push(CompactString::from(url));
                }
                if self.challenge_script_url.is_none() {
                    self.challenge_script_url = Some(CompactString::from(url));
                }
            }
        }
    }

    fn push_extract(&mut self, key: u32, text: &str, cap: usize) {
        let Some(slot) = self.extracted_ids.get_mut(key as usize) else {
            return;
        };
        let e = slot.get_or_insert_with(CompactString::default);
        if e.len() >= cap {
            return;
        }
        e.push_str(truncate_str(text, cap - e.len()));
    }

    pub(crate) fn into_page(
        mut self,
        bytes_fed: u64,
        utf8_bad_chunks: u32,
        truncated: bool,
    ) -> PageData {
        self.finalize_pending();
        let tag_names = self.tag_intern.name_table();
        let attr_names = self.attr_intern.name_table();
        self.dom.attach_name_tables(tag_names, attr_names);
        let parse_errors = self.parse_errors;
        let extracted = self
            .extract_keys
            .into_iter()
            .zip(self.extracted_ids)
            .filter_map(|(key, v)| v.map(|v| (key, v)))
            .collect();
        PageData {
            title: self.title,
            meta_description: self.meta_description,
            forms: self.forms,
            tokens: self.tokens,
            script_srcs: self.script_srcs,
            inline_count: self.inline_count,
            challenge: self.challenge,
            challenge_node: self.challenge_node,
            challenge_markers: self.challenge_markers,
            challenge_script_url: self.challenge_script_url,
            captcha_family: self.captcha_family,
            captcha_sitekey: self.captcha_sitekey,
            telemetry_route: self.telemetry_route,
            extracted,
            next_data: self.next_data,
            anubis: self.anubis,
            anubis_version: self.anubis_version,
            dom: self.dom,
            bytes_fed,
            utf8_bad_chunks,
            truncated,
            parse_errors,
        }
    }
}

const TOKEN_MARKERS: &[&str] = &[
    "csrf",
    "_token",
    "token",
    "xsrf",
    "authenticity",
    "x-csrf",
    "anticsrf",
    "anti-csrf",
    "requesttoken",
    "request-token",
    "__requestverificationtoken",
    "csrfmiddlewaretoken",
];

static TOKEN_AC: LazyLock<aho_corasick::AhoCorasick> =
    LazyLock::new(|| build_ac(TOKEN_MARKERS.iter().map(|s| s.as_bytes()), true));

fn looks_like_token(name: &str) -> bool {
    TOKEN_AC.is_match(name.as_bytes())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Hidden,
    Password,
    Other,
}

impl FieldKind {
    #[inline(always)]
    pub fn from_type_attr(t: &str) -> Self {
        let b = t.as_bytes();
        if b.eq_ci(b"hidden") {
            Self::Hidden
        } else if b.eq_ci(b"password") {
            Self::Password
        } else if b.eq_ci(b"text") {
            Self::Text
        } else {
            Self::Other
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Hidden => "Hidden",
            Self::Password => "Password",
            Self::Other => "Other",
        }
    }
}

pub struct FormData {
    pub name: CompactString,
    pub value: Option<CompactString>,
    pub kind: FieldKind,
}

pub struct Form {
    pub action: Option<CompactString>,
    pub method: CompactString,
    pub fields: SmallVec<[FormData; 8]>,
}

pub struct NextData {
    pub page: CompactString,
    pub build_id: CompactString,
}

pub struct PageData {
    pub title: Option<CompactString>,
    pub meta_description: Option<CompactString>,
    pub forms: SmallVec<[Form; 4]>,
    pub tokens: SmallVec<[CompactString; 8]>,
    pub script_srcs: SmallVec<[CompactString; 12]>,
    pub inline_count: usize,
    pub challenge: Option<Bytes>,
    pub challenge_node: Option<u32>,
    pub challenge_markers: SmallVec<[CompactString; 4]>,
    pub challenge_script_url: Option<CompactString>,
    pub captcha_family: Option<u8>,
    pub captcha_sitekey: Option<CompactString>,
    pub telemetry_route: Option<crate::telemetry::TelemetryRoute>,
    pub dom: DomTree,
    pub extracted: BTreeMap<CompactString, CompactString>,
    pub next_data: Option<NextData>,
    pub anubis: Option<Bytes>,
    pub anubis_version: Option<CompactString>,
    pub bytes_fed: u64,
    pub utf8_bad_chunks: u32,
    pub truncated: bool,
    pub parse_errors: u32,
}

impl PageData {
    pub fn empty() -> Self {
        Self {
            title: None,
            meta_description: None,
            forms: SmallVec::new(),
            tokens: SmallVec::new(),
            script_srcs: SmallVec::new(),
            inline_count: 0,
            challenge: None,
            challenge_node: None,
            challenge_markers: SmallVec::new(),
            challenge_script_url: None,
            captcha_family: None,
            captcha_sitekey: None,
            telemetry_route: None,
            extracted: BTreeMap::new(),
            next_data: None,
            anubis: None,
            anubis_version: None,
            dom: DomTree::new(),
            bytes_fed: 0,
            utf8_bad_chunks: 0,
            truncated: false,
            parse_errors: 0,
        }
    }
}
