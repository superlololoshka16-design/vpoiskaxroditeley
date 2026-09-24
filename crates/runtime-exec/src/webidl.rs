use crate::stackfmt::set_fn_name;
use crate::touch::{self, ApiKey};
use crate::worker::{NodeHandle, with_doc};
use bytes::Bytes;
use compact_str::CompactString;
use core_utils::{BytesExt as _, FxBuild, fx_map};
use rquickjs::function::{Rest, This};
use rquickjs::{Class, Ctx, Function, IntoJs, JsLifetime, Object, Persistent, Value};
use smallvec::{SmallVec, smallvec};
use std::cell::RefCell;
use std::collections::HashMap;

pub(crate) const OVERLAY_BASE: u32 = 0x8000_0000;

macro_rules! ifaces {
    ($(($ident:ident, $name:literal, $parent:tt)),* $(,)?) => {
        #[repr(u16)]
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub(crate) enum InterfaceId {
            $($ident),*
        }

        const IFACE_META: &[(&'static str, Option<InterfaceId>)] = &[
            $(($name, ifaces!(@parent $parent))),*
        ];

        #[inline]
        const fn parent_of(id: InterfaceId) -> Option<InterfaceId> {
            IFACE_META[id as u16 as usize].1
        }

        #[inline]
        pub(crate) const fn iface_name(id: InterfaceId) -> &'static str {
            IFACE_META[id as u16 as usize].0
        }

        const IFACE_ALL: &[InterfaceId] = &[$(InterfaceId::$ident),*];

        impl InterfaceId {
            #[inline]
            pub(crate) fn from_u16(v: u16) -> Option<Self> {
                IFACE_ALL.get(v as usize).copied()
            }
        }
    };
    (@parent None) => { None };
    (@parent $p:ident) => { Some(InterfaceId::$p) };
}

ifaces! {
    (EventTarget, "EventTarget", None),
    (Node, "Node", EventTarget),
    (CharacterData, "CharacterData", Node),
    (Text, "Text", CharacterData),
    (Comment, "Comment", CharacterData),
    (CdataSection, "CDATASection", Text),
    (DocumentType, "DocumentType", Node),
    (Attr, "Attr", Node),
    (Element, "Element", Node),
    (HTMLElement, "HTMLElement", Element),
    (HtmlUnknownElement, "HTMLUnknownElement", HTMLElement),
    (HtmlDivElement, "HTMLDivElement", HTMLElement),
    (HtmlSpanElement, "HTMLSpanElement", HTMLElement),
    (HtmlAnchorElement, "HTMLAnchorElement", HTMLElement),
    (HtmlImageElement, "HTMLImageElement", HTMLElement),
    (HtmlInputElement, "HTMLInputElement", HTMLElement),
    (HtmlButtonElement, "HTMLButtonElement", HTMLElement),
    (HtmlFormElement, "HTMLFormElement", HTMLElement),
    (HtmlScriptElement, "HTMLScriptElement", HTMLElement),
    (HtmlCanvasElement, "HTMLCanvasElement", HTMLElement),
    (HtmlBodyElement, "HTMLBodyElement", HTMLElement),
    (HtmlHeadElement, "HTMLHeadElement", HTMLElement),
    (HtmlHtmlElement, "HTMLHtmlElement", HTMLElement),
    (HtmlMetaElement, "HTMLMetaElement", HTMLElement),
    (HtmlLinkElement, "HTMLLinkElement", HTMLElement),
    (HtmlTitleElement, "HTMLTitleElement", HTMLElement),
    (HtmlIFrameElement, "HTMLIFrameElement", HTMLElement),
    (HtmlParagraphElement, "HTMLParagraphElement", HTMLElement),
    (HtmlSelectElement, "HTMLSelectElement", HTMLElement),
    (HtmlTextAreaElement, "HTMLTextAreaElement", HTMLElement),
    (HtmlOptionElement, "HTMLOptionElement", HTMLElement),
    (HtmlUListElement, "HTMLUListElement", HTMLElement),
    (HtmlLIElement, "HTMLLIElement", HTMLElement),
    (HtmlTableElement, "HTMLTableElement", HTMLElement),
    (HtmlTableCellElement, "HTMLTableCellElement", HTMLElement),
    (HtmlTableRowElement, "HTMLTableRowElement", HTMLElement),
    (HtmlLabelElement, "HTMLLabelElement", HTMLElement),
    (HtmlHeadingElement, "HTMLHeadingElement", HTMLElement),
    (HtmlStyleElement, "HTMLStyleElement", HTMLElement),
    (HtmlTemplateElement, "HTMLTemplateElement", HTMLElement),
    (HtmlAreaElement, "HTMLAreaElement", HTMLElement),
    (HtmlAudioElement, "HTMLAudioElement", HtmlMediaElement),
    (HtmlVideoElement, "HTMLVideoElement", HtmlMediaElement),
    (HtmlBRElement, "HTMLBRElement", HTMLElement),
    (HtmlBaseElement, "HTMLBaseElement", HTMLElement),
    (HtmlDataElement, "HTMLDataElement", HTMLElement),
    (HtmlDataListElement, "HTMLDataListElement", HTMLElement),
    (HtmlDetailsElement, "HTMLDetailsElement", HTMLElement),
    (HtmlDialogElement, "HTMLDialogElement", HTMLElement),
    (HtmlDirectoryElement, "HTMLDirectoryElement", HTMLElement),
    (HtmlDListElement, "HTMLDListElement", HTMLElement),
    (HtmlEmbedElement, "HTMLEmbedElement", HTMLElement),
    (HtmlFieldSetElement, "HTMLFieldSetElement", HTMLElement),
    (HtmlFontElement, "HTMLFontElement", HTMLElement),
    (HtmlFrameElement, "HTMLFrameElement", HTMLElement),
    (HtmlFrameSetElement, "HTMLFrameSetElement", HTMLElement),
    (HtmlHRElement, "HTMLHRElement", HTMLElement),
    (HtmlMapElement, "HTMLMapElement", HTMLElement),
    (HtmlMarqueeElement, "HTMLMarqueeElement", HTMLElement),
    (HtmlMediaElement, "HTMLMediaElement", HTMLElement),
    (HtmlMenuElement, "HTMLMenuElement", HTMLElement),
    (HtmlMeterElement, "HTMLMeterElement", HTMLElement),
    (HtmlModElement, "HTMLModElement", HTMLElement),
    (HtmlOListElement, "HTMLOListElement", HTMLElement),
    (HtmlObjectElement, "HTMLObjectElement", HTMLElement),
    (HtmlOptGroupElement, "HTMLOptGroupElement", HTMLElement),
    (HtmlOutputElement, "HTMLOutputElement", HTMLElement),
    (HtmlPictureElement, "HTMLPictureElement", HTMLElement),
    (HtmlPreElement, "HTMLPreElement", HTMLElement),
    (HtmlProgressElement, "HTMLProgressElement", HTMLElement),
    (HtmlQuoteElement, "HTMLQuoteElement", HTMLElement),
    (HtmlSlotElement, "HTMLSlotElement", HTMLElement),
    (HtmlSourceElement, "HTMLSourceElement", HTMLElement),
    (HtmlTableCaptionElement, "HTMLTableCaptionElement", HtmlTableElement),
    (HtmlTableColElement, "HTMLTableColElement", HtmlTableElement),
    (HtmlTableSectionElement, "HTMLTableSectionElement", HtmlTableElement),
    (HtmlTimeElement, "HTMLTimeElement", HTMLElement),
    (HtmlTrackElement, "HTMLTrackElement", HTMLElement),
    (DocumentFragment, "DocumentFragment", Node),
    (ShadowRoot, "ShadowRoot", DocumentFragment),
    (XmlDocument, "XMLDocument", None),
    (SvgElement, "SVGElement", Element),
    (SvgGraphicsElement, "SVGGraphicsElement", SvgElement),
    (SvgSVGElement, "SVGSVGElement", SvgGraphicsElement),
    (SvgTitleElement, "SVGTitleElement", SvgElement),
    (SvgCircleElement, "SVGCircleElement", SvgGraphicsElement),
    (SvgPathElement, "SVGPathElement", SvgGraphicsElement),
    (SvgRectElement, "SVGRectElement", SvgGraphicsElement),
    (SvgLineElement, "SVGLineElement", SvgGraphicsElement),
    (SvgPolygonElement, "SVGPolygonElement", SvgGraphicsElement),
    (SvgPolylineElement, "SVGPolylineElement", SvgGraphicsElement),
    (SvgAnimatedString, "SVGAnimatedString", None),
    (SvgLength, "SVGLength", None),
    (SvgNumber, "SVGNumber", None),
    (WorkerNavigator, "WorkerNavigator", EventTarget),
    (AbstractRange, "AbstractRange", None),
    (StaticRange, "StaticRange", AbstractRange),
    (Range, "Range", AbstractRange),
    (NodeList, "NodeList", None),
    (HtmlCollection, "HTMLCollection", None),
    (DomTokenList, "DOMTokenList", None),
    (DomStringMap, "DOMStringMap", None),
    (DomStringList, "DOMStringList", None),
    (NamedNodeMap, "NamedNodeMap", None),
    (MutationRecord, "MutationRecord", None),
    (NodeIterator, "NodeIterator", None),
    (TreeWalker, "TreeWalker", None),
    (DomParser, "DOMParser", None),
    (DomImplementation, "DOMImplementation", None),
    (XmlSerializer, "XMLSerializer", None),
    (XPathEvaluator, "XPathEvaluator", None),
    (XPathExpression, "XPathExpression", None),
    (XPathResult, "XPathResult", None),
    (XsltProcessor, "XSLTProcessor", None),
    (WebGLRenderingContext, "WebGLRenderingContext", None),
    (WebGL2RenderingContext, "WebGL2RenderingContext", None),
    (WebGLActiveInfo, "WebGLActiveInfo", None),
    (WebGLBuffer, "WebGLBuffer", None),
    (WebGLFramebuffer, "WebGLFramebuffer", None),
    (WebGLProgram, "WebGLProgram", None),
    (WebGLQuery, "WebGLQuery", None),
    (WebGLRenderbuffer, "WebGLRenderbuffer", None),
    (WebGLSampler, "WebGLSampler", None),
    (WebGLShader, "WebGLShader", None),
    (WebGLShaderPrecisionFormat, "WebGLShaderPrecisionFormat", None),
    (WebGLSync, "WebGLSync", None),
    (WebGLTexture, "WebGLTexture", None),
    (WebGLTransformFeedback, "WebGLTransformFeedback", None),
    (WebGLUniformLocation, "WebGLUniformLocation", None),
    (WebGLVertexArrayObject, "WebGLVertexArrayObject", None),
    (CssStyleDeclaration, "CSSStyleDeclaration", None),
    (StyleSheet, "StyleSheet", None),
    (CssStyleSheet, "CSSStyleSheet", StyleSheet),
    (CssRule, "CSSRule", None),
    (CssStyleRule, "CSSStyleRule", CssRule),
    (StyleSheetList, "StyleSheetList", None),
    (MediaList, "MediaList", None),
    (Animation, "Animation", None),
    (CssAnimation, "CSSAnimation", Animation),
    (CssTransition, "CSSTransition", Animation),
    (PerformanceEntry, "PerformanceEntry", None),
    (PerformanceMark, "PerformanceMark", PerformanceEntry),
    (PerformanceMeasure, "PerformanceMeasure", PerformanceEntry),
    (PerformanceResourceTiming, "PerformanceResourceTiming", PerformanceEntry),
    (PerformanceNavigationTiming, "PerformanceNavigationTiming", PerformanceEntry),
    (PerformancePaintTiming, "PerformancePaintTiming", PerformanceEntry),
    (PerformanceLongTaskTiming, "PerformanceLongTaskTiming", PerformanceEntry),
    (TaskAttributionTiming, "TaskAttributionTiming", PerformanceEntry),
    (PerformanceServerTiming, "PerformanceServerTiming", PerformanceEntry),
    (PerformanceNavigation, "PerformanceNavigation", None),
    (PerformanceObserverEntryList, "PerformanceObserverEntryList", None),
    (ResizeObserverEntry, "ResizeObserverEntry", None),
    (IntersectionObserverEntry, "IntersectionObserverEntry", None),
    (Gamepad, "Gamepad", None),
    (GamepadButton, "GamepadButton", None),
    (GamepadHapticActuator, "GamepadHapticActuator", None),
    (Geolocation, "Geolocation", None),
    (GeolocationCoordinates, "GeolocationCoordinates", None),
    (GeolocationPosition, "GeolocationPosition", None),
    (GeolocationPositionError, "GeolocationPositionError", None),
    (Credential, "Credential", None),
    (CredentialsContainer, "CredentialsContainer", None),
    (PasswordCredential, "PasswordCredential", Credential),
    (FederatedCredential, "FederatedCredential", Credential),
    (PublicKeyCredential, "PublicKeyCredential", Credential),
    (Crypto, "Crypto", None),
    (CryptoKey, "CryptoKey", None),
    (SubtleCrypto, "SubtleCrypto", None),
    (Storage, "Storage", None),
    (StorageManager, "StorageManager", None),
    (CustomElementRegistry, "CustomElementRegistry", None),
    (ValidityState, "ValidityState", None),
    (IdleDeadline, "IdleDeadline", None),
    (Plugin, "Plugin", None),
    (PluginArray, "PluginArray", None),
    (MimeType, "MimeType", None),
    (MimeTypeArray, "MimeTypeArray", None),
    (NavigatorUAData, "NavigatorUAData", None),
    (BatteryManager, "BatteryManager", EventTarget),
    (BarProp, "BarProp", None),
    (ScreenOrientation, "ScreenOrientation", None),
    (ScreenDetailed, "ScreenDetailed", None),
    (ScreenDetails, "ScreenDetails", None),
    (History, "History", None),
    (LockManager, "LockManager", None),
    (Lock, "Lock", None),
    (Report, "Report", None),
    (ReportBody, "ReportBody", None),
    (SpeechSynthesis, "SpeechSynthesis", None),
    (MediaCapabilities, "MediaCapabilities", None),
    (MediaDeviceInfo, "MediaDeviceInfo", None),
    (MediaDevices, "MediaDevices", None),
    (MediaError, "MediaError", None),
    (MediaQueryList, "MediaQueryList", None),
    (MediaMetadata, "MediaMetadata", None),
    (MediaRecorder, "MediaRecorder", None),
    (MediaSession, "MediaSession", None),
    (MediaSource, "MediaSource", None),
    (MediaStream, "MediaStream", None),
    (MediaStreamTrack, "MediaStreamTrack", None),
    (TextTrack, "TextTrack", None),
    (TextTrackCue, "TextTrackCue", None),
    (VTCue, "VTTCue", TextTrackCue),
    (TextTrackCueList, "TextTrackCueList", None),
    (TextTrackList, "TextTrackList", None),
    (TimeRanges, "TimeRanges", None),
    (VideoColorSpace, "VideoColorSpace", None),
    (BaseAudioContext, "BaseAudioContext", None),
    (AudioBuffer, "AudioBuffer", None),
    (AudioNode, "AudioNode", EventTarget),
    (AudioParam, "AudioParam", None),
    (AudioBufferSourceNode, "AudioBufferSourceNode", AudioScheduledSourceNode),
    (AudioDestinationNode, "AudioDestinationNode", AudioNode),
    (AudioListener, "AudioListener", None),
    (AudioScheduledSourceNode, "AudioScheduledSourceNode", AudioNode),
    (AudioWorklet, "AudioWorklet", None),
    (AudioWorkletNode, "AudioWorkletNode", AudioNode),
    (ReadableStream, "ReadableStream", None),
    (ReadableStreamDefaultReader, "ReadableStreamDefaultReader", None),
    (ReadableStreamDefaultController, "ReadableStreamDefaultController", None),
    (WritableStream, "WritableStream", None),
    (WritableStreamDefaultWriter, "WritableStreamDefaultWriter", None),
    (WritableStreamDefaultController, "WritableStreamDefaultController", None),
    (TransformStream, "TransformStream", None),
    (TransformStreamDefaultController, "TransformStreamDefaultController", None),
    (ByteLengthQueuingStrategy, "ByteLengthQueuingStrategy", None),
    (CountQueuingStrategy, "CountQueuingStrategy", None),
    (TextEncoderStream, "TextEncoderStream", None),
    (TextDecoderStream, "TextDecoderStream", None),
    (SharedWorker, "SharedWorker", EventTarget),
    (DataTransfer, "DataTransfer", None),
    (DataTransferItem, "DataTransferItem", None),
    (DataTransferItemList, "DataTransferItemList", None),
    (ImageBitmap, "ImageBitmap", None),
    (ImageData, "ImageData", None),
    (TextMetrics, "TextMetrics", None),
    (CanvasGradient, "CanvasGradient", None),
    (CanvasPattern, "CanvasPattern", None),
    (Navigation, "Navigation", EventTarget),
    (NavigationDestination, "NavigationDestination", None),
    (NavigationHistoryEntry, "NavigationHistoryEntry", None),
    (NavigationPreloadManager, "NavigationPreloadManager", None),
    (NavigationTransition, "NavigationTransition", None),
    (ClipboardItem, "ClipboardItem", None),
}

fn iface_for_tag(tag: u16) -> InterfaceId {
    use parser_pipeline::dom::tags as t;
    match tag {
        t::AREA => InterfaceId::HtmlAreaElement,
        t::A => InterfaceId::HtmlAnchorElement,
        t::SPAN => InterfaceId::HtmlSpanElement,
        t::DIV => InterfaceId::HtmlDivElement,
        t::CANVAS => InterfaceId::HtmlCanvasElement,
        t::BUTTON => InterfaceId::HtmlButtonElement,
        t::BODY => InterfaceId::HtmlBodyElement,
        t::TEMPLATE => InterfaceId::HtmlTemplateElement,
        t::FORM => InterfaceId::HtmlFormElement,
        t::H1..=t::H6 => InterfaceId::HtmlHeadingElement,
        t::HEAD => InterfaceId::HtmlHeadElement,
        t::HTML => InterfaceId::HtmlHtmlElement,
        t::IFRAME => InterfaceId::HtmlIFrameElement,
        t::IMG => InterfaceId::HtmlImageElement,
        t::INPUT => InterfaceId::HtmlInputElement,
        t::LI => InterfaceId::HtmlLIElement,
        t::LINK => InterfaceId::HtmlLinkElement,
        t::META => InterfaceId::HtmlMetaElement,
        t::OL => InterfaceId::HtmlOListElement,
        t::UL => InterfaceId::HtmlUListElement,
        t::OPTION => InterfaceId::HtmlOptionElement,
        t::P => InterfaceId::HtmlParagraphElement,
        t::SCRIPT => InterfaceId::HtmlScriptElement,
        t::SELECT => InterfaceId::HtmlSelectElement,
        t::STYLE => InterfaceId::HtmlStyleElement,
        t::SUMMARY => InterfaceId::HTMLElement,
        t::TABLE | t::TBODY | t::TFOOT | t::THEAD => InterfaceId::HtmlTableElement,
        t::TH | t::TD => InterfaceId::HtmlTableCellElement,
        t::TEXTAREA => InterfaceId::HtmlTextAreaElement,
        t::TITLE => InterfaceId::HtmlTitleElement,
        t::TR => InterfaceId::HtmlTableRowElement,
        t::LABEL => InterfaceId::HtmlLabelElement,
        _ => InterfaceId::HTMLElement,
    }
}

static IFACE_BY_TAG: std::sync::LazyLock<[InterfaceId; 256]> = std::sync::LazyLock::new(|| {
    let mut t = [InterfaceId::HTMLElement; 256];
    for tag in 0u16..256 {
        t[tag as usize] = iface_for_tag(tag);
    }
    t
});

fn iface_for_tag_name(name: &str) -> (InterfaceId, u16) {
    match parser_pipeline::dom::TAGS.get(name) {
        Some(&tag) => (iface_for_tag(tag), tag),
        None => (InterfaceId::HtmlUnknownElement, parser_pipeline::dom::TAG_UNKNOWN),
    }
}

const KIND_ELEMENT: u8 = 0;
const KIND_TEXT: u8 = 1;
const KIND_FRAGMENT: u8 = 2;
const KIND_EVENT_TARGET: u8 = 3;

#[derive(Clone)]
pub(crate) struct MutRec {
    pub(crate) kind: u8,
    pub(crate) target: u32,
    pub(crate) added: u32,
    pub(crate) added_len: u32,
    pub(crate) removed: u32,
    pub(crate) removed_len: u32,
    pub(crate) attr_name: CompactString,
    pub(crate) old_value: CompactString,
    pub(crate) prev_sibling: u32,
    pub(crate) next_sibling: u32,
}

impl MutRec {
    #[inline]
    fn empty() -> Self {
        Self {
            kind: 0,
            target: u32::MAX,
            added: 0,
            added_len: 0,
            removed: 0,
            removed_len: 0,
            attr_name: CompactString::const_new(""),
            old_value: CompactString::const_new(""),
            prev_sibling: u32::MAX,
            next_sibling: u32::MAX,
        }
    }
}

struct ObsReg {
    obs: u64,
    cb: Persistent<Value<'static>>,
    target: u32,
    opts: ObsOptions,
    queue: SmallVec<[u32; 8]>,
}

pub(crate) struct ObsOptions {
    bits: u8,
    pub filter: SmallVec<[CompactString; 4]>,
}

impl ObsOptions {
    pub const SUBTREE: u8 = 1 << 0;
    pub const ATTRS: u8 = 1 << 1;
    pub const CHILD_LIST: u8 = 1 << 2;
    pub const ATTR_OLD: u8 = 1 << 3;
    pub const CHAR_DATA: u8 = 1 << 4;
    pub const CHAR_OLD: u8 = 1 << 5;

    #[inline]
    pub fn new(subtree: bool, attrs: bool, child_list: bool, attr_old: bool, char_data: bool, char_old: bool) -> Self {
        let bits = u8::from(subtree) * Self::SUBTREE
            | u8::from(attrs) * Self::ATTRS
            | u8::from(child_list) * Self::CHILD_LIST
            | u8::from(attr_old) * Self::ATTR_OLD
            | u8::from(char_data) * Self::CHAR_DATA
            | u8::from(char_old) * Self::CHAR_OLD;
        Self {
            bits,
            filter: SmallVec::new(),
        }
    }

    #[inline(always)]
    pub fn subtree(&self) -> bool {
        self.bits & Self::SUBTREE != 0
    }

    #[inline(always)]
    pub fn child_list(&self) -> bool {
        self.bits & Self::CHILD_LIST != 0
    }

    #[inline(always)]
    pub fn attrs(&self) -> bool {
        self.bits & Self::ATTRS != 0
    }

    #[inline(always)]
    pub fn char_data(&self) -> bool {
        self.bits & Self::CHAR_DATA != 0
    }

    #[inline(always)]
    pub fn attr_old(&self) -> bool {
        self.bits & Self::ATTR_OLD != 0
    }

    #[inline(always)]
    pub fn char_old(&self) -> bool {
        self.bits & Self::CHAR_OLD != 0
    }

    #[inline]
    pub fn with_filter(mut self, filter: SmallVec<[CompactString; 4]>) -> Self {
        self.filter = filter;
        self
    }
}

#[repr(C, align(32))]
struct NodeSlot {
    parent: u32,
    first: u32,
    last: u32,
    next: u32,
    prev: u32,
    attr_head: u32,
    attr_tail: u32,
    str_idx: u32,
    meta: u32,
}

#[repr(C)]
struct AttrEnt {
    value: CompactString,
    next: u32,
    name_idx: u32,
    name_id: u16,
    tomb: bool,
}

const PLACE_NONE: u32 = u32::MAX;
const PLACE_DETACHED: u32 = u32::MAX - 1;
struct BaseOvr {
    place: u32,
    tail_head: u32,
    tail_last: u32,
    attr_head: u32,
    attr_tail: u32,
    adopted: SmallVec<[u32; 4]>,
}

impl BaseOvr {
    fn new() -> Self {
        Self {
            place: PLACE_NONE,
            tail_head: u32::MAX,
            tail_last: u32::MAX,
            attr_head: u32::MAX,
            attr_tail: u32::MAX,
            adopted: SmallVec::new(),
        }
    }
}

struct MutDom {
    slots: Vec<NodeSlot>,
    strings: Vec<CompactString>,
    attrs: Vec<AttrEnt>,
    attr_free: Vec<u32>,
    base_ovr: HashMap<u32, BaseOvr, FxBuild>,
    external: HashMap<u32, Persistent<Value<'static>>, FxBuild>,
    styles: HashMap<u32, Persistent<Object<'static>>, FxBuild>,
    id_index: HashMap<CompactString, SmallVec<[u32; 2]>, FxBuild>,
    base_id_index: HashMap<CompactString, SmallVec<[u32; 2]>, FxBuild>,
    observers: Vec<ObsReg>,
    recs: Vec<MutRec>,
    rec_ids: Vec<u32>,
    tree_epoch: u64,
    dom_epoch: u64,
}

impl MutDom {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            strings: Vec::new(),
            attrs: Vec::new(),
            attr_free: Vec::new(),
            base_ovr: fx_map(),
            external: fx_map(),
            styles: fx_map(),
            id_index: fx_map(),
            base_id_index: fx_map(),
            observers: Vec::new(),
            recs: Vec::new(),
            rec_ids: Vec::new(),
            tree_epoch: 1,
            dom_epoch: 1,
        }
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.strings.clear();
        self.attrs.clear();
        self.attr_free.clear();
        self.base_ovr.clear();
        self.external.clear();
        self.styles.clear();
        self.id_index.clear();
        self.base_id_index.clear();
        self.observers.clear();
        self.recs.clear();
        self.rec_ids.clear();
        self.tree_epoch = 1;
        self.dom_epoch = 1;
    }

    #[inline]
    fn is_overlay(id: u32) -> bool {
        id >= OVERLAY_BASE && id != u32::MAX
    }

    #[inline]
    fn si(id: u32) -> usize {
        (id - OVERLAY_BASE) as usize
    }

    #[inline]
    fn kind_at(&self, i: usize) -> u8 {
        (self.slots[i].meta & 0x3) as u8
    }

    #[inline]
    fn iface_at(&self, i: usize) -> u16 {
        ((self.slots[i].meta >> 2) & 0x1FF) as u16
    }

    #[inline]
    fn tag_at(&self, i: usize) -> u16 {
        (self.slots[i].meta >> 11) as u16
    }

    #[inline]
    fn str_at(&self, i: usize) -> Option<&CompactString> {
        let s = self.slots[i].str_idx;
        (s != u32::MAX).then(|| &self.strings[s as usize])
    }

    fn alloc(&mut self, kind: u8, iface: InterfaceId, tag: u16, s: Option<CompactString>) -> u32 {
        let str_idx = match s {
            Some(cs) => {
                self.strings.push(cs);
                (self.strings.len() - 1) as u32
            }
            None => u32::MAX,
        };
        let id = OVERLAY_BASE + self.slots.len() as u32;
        self.slots.push(NodeSlot {
            parent: u32::MAX,
            first: u32::MAX,
            last: u32::MAX,
            next: u32::MAX,
            prev: u32::MAX,
            attr_head: u32::MAX,
            attr_tail: u32::MAX,
            str_idx,
            meta: (kind as u32) | ((iface as u16 as u32) << 2) | ((tag as u32) << 11),
        });
        id
    }

    #[inline]
    fn attr_nid(name: &str) -> u16 {
        parser_pipeline::ATTR_NAMES
            .get(name)
            .copied()
            .unwrap_or(parser_pipeline::dom::ATTR_UNKNOWN)
    }

    fn attr_name<'a>(&'a self, e: &'a AttrEnt) -> &'a str {
        if e.name_id != parser_pipeline::dom::ATTR_UNKNOWN {
            parser_pipeline::dom::static_attr_name(e.name_id).unwrap_or("")
        } else if e.name_idx != u32::MAX {
            self.strings[e.name_idx as usize].as_str()
        } else {
            ""
        }
    }

    fn attr_find_at(&self, head: u32, nid: u16, name: &str) -> Option<u32> {
        let mut cur = head;
        while cur != u32::MAX {
            let e = &self.attrs[cur as usize];
            let hit = if nid != parser_pipeline::dom::ATTR_UNKNOWN {
                e.name_id == nid
            } else {
                e.name_id == parser_pipeline::dom::ATTR_UNKNOWN && self.attr_name(e) == name
            };
            if hit {
                return Some(cur);
            }
            cur = e.next;
        }
        None
    }

    fn attr_find(&self, i: usize, nid: u16, name: &str) -> Option<u32> {
        self.attr_find_at(self.slots[i].attr_head, nid, name)
    }

    fn arena_append(&mut self, tail: u32, nid: u16, name: &str, value: &str, tomb: bool) -> u32 {
        let name_idx = if nid == parser_pipeline::dom::ATTR_UNKNOWN {
            self.strings.push(CompactString::new(name));
            (self.strings.len() - 1) as u32
        } else {
            u32::MAX
        };
        let ent = AttrEnt {
            value: CompactString::new(value),
            next: u32::MAX,
            name_idx,
            name_id: nid,
            tomb,
        };
        let idx = match self.attr_free.pop() {
            Some(f) => {
                self.attrs[f as usize] = ent;
                f
            }
            None => {
                self.attrs.push(ent);
                (self.attrs.len() - 1) as u32
            }
        };
        if tail != u32::MAX {
            self.attrs[tail as usize].next = idx;
        }
        idx
    }

    fn attr_chain_tail(&self, head: u32) -> u32 {
        let mut cur = head;
        while cur != u32::MAX {
            let nxt = self.attrs[cur as usize].next;
            if nxt == u32::MAX {
                return cur;
            }
            cur = nxt;
        }
        head
    }

    fn arena_unlink(&mut self, head: u32, idx: u32) -> u32 {
        let mut cur = head;
        let mut prev = u32::MAX;
        while cur != u32::MAX {
            let nxt = self.attrs[cur as usize].next;
            if cur == idx {
                self.attr_free.push(idx);
                if prev == u32::MAX {
                    return nxt;
                }
                self.attrs[prev as usize].next = nxt;
                return head;
            }
            prev = cur;
            cur = nxt;
        }
        head
    }

    #[inline]
    fn bovr(&self, node: u32) -> Option<&BaseOvr> {
        self.base_ovr.get(&node)
    }

    #[inline]
    fn bovr_mut(&mut self, node: u32) -> &mut BaseOvr {
        self.base_ovr.entry(node).or_insert_with(BaseOvr::new)
    }

    #[inline]
    fn bovr_get_mut(&mut self, node: u32) -> Option<&mut BaseOvr> {
        self.base_ovr.get_mut(&node)
    }

    #[inline]
    fn place_of(&self, node: u32) -> u32 {
        self.base_ovr.get(&node).map(|o| o.place).unwrap_or(PLACE_NONE)
    }

    fn attr_value<'a>(&'a self, i: usize, name: &str) -> Option<&'a str> {
        let nid = Self::attr_nid(name);
        self.attr_find(i, nid, name)
            .map(|idx| self.attrs[idx as usize].value.as_str())
    }

    fn attr_set(&mut self, i: usize, name: &str, value: &str) -> Option<CompactString> {
        let nid = Self::attr_nid(name);
        let head = self.slots[i].attr_head;
        if let Some(idx) = self.attr_find_at(head, nid, name) {
            let e = &mut self.attrs[idx as usize];
            return Some(std::mem::replace(&mut e.value, CompactString::new(value)));
        }
        let tail = if self.slots[i].attr_tail != u32::MAX {
            self.slots[i].attr_tail
        } else {
            self.attr_chain_tail(head)
        };
        let idx = self.arena_append(tail, nid, name, value, false);
        if head == u32::MAX {
            self.slots[i].attr_head = idx;
        }
        self.slots[i].attr_tail = idx;
        None
    }

    fn attr_remove(&mut self, i: usize, name: &str) {
        let nid = Self::attr_nid(name);
        let head = self.slots[i].attr_head;
        if let Some(idx) = self.attr_find_at(head, nid, name) {
            self.slots[i].attr_head = self.arena_unlink(head, idx);
            self.slots[i].attr_tail = self.attr_chain_tail(self.slots[i].attr_head);
        }
    }

    fn base_attr_set(&mut self, node: u32, name: &str, value: &str) -> Option<CompactString> {
        let nid = Self::attr_nid(name);
        let (head, tail) = {
            let o = self.bovr_mut(node);
            (o.attr_head, o.attr_tail)
        };
        if let Some(idx) = self.attr_find_at(head, nid, name) {
            if !self.attrs[idx as usize].tomb {
                let e = &mut self.attrs[idx as usize];
                return Some(std::mem::replace(&mut e.value, CompactString::new(value)));
            }
            let h = self.arena_unlink(head, idx);
            let t = self.attr_chain_tail(h);
            let nidx = self.arena_append(t, nid, name, value, false);
            let o = self.bovr_get_mut(node).unwrap();
            o.attr_head = h;
            o.attr_tail = nidx;
            return None;
        }
        let t = if tail != u32::MAX {
            tail
        } else {
            self.attr_chain_tail(head)
        };
        let nidx = self.arena_append(t, nid, name, value, false);
        let o = self.bovr_get_mut(node).unwrap();
        if head == u32::MAX {
            o.attr_head = nidx;
        }
        o.attr_tail = nidx;
        None
    }

    fn base_attr_remove(&mut self, node: u32, name: &str) -> Option<CompactString> {
        let nid = Self::attr_nid(name);
        let (head, tail) = {
            let o = self.bovr_mut(node);
            (o.attr_head, o.attr_tail)
        };
        match self.attr_find_at(head, nid, name) {
            Some(idx) => {
                if self.attrs[idx as usize].tomb {
                    return None;
                }
                let e = &mut self.attrs[idx as usize];
                e.tomb = true;
                Some(std::mem::replace(&mut e.value, CompactString::new("")))
            }
            None => {
                let t = if tail != u32::MAX {
                    tail
                } else {
                    self.attr_chain_tail(head)
                };
                let nidx = self.arena_append(t, nid, name, "", true);
                let o = self.bovr_get_mut(node).unwrap();
                if head == u32::MAX {
                    o.attr_head = nidx;
                }
                o.attr_tail = nidx;
                None
            }
        }
    }

    fn attr_list(&self, i: usize) -> SmallVec<[(CompactString, CompactString); 4]> {
        let mut out: SmallVec<[(CompactString, CompactString); 4]> = SmallVec::new();
        let mut cur = self.slots[i].attr_head;
        while cur != u32::MAX {
            let e = &self.attrs[cur as usize];
            out.push((CompactString::new(self.attr_name(e)), e.value.clone()));
            cur = e.next;
        }
        out
    }

    fn link(&mut self, parent: u32, child: u32) {
        let ci = Self::si(child);
        if Self::is_overlay(parent) {
            let pi = Self::si(parent);
            let prev = self.slots[pi].last;
            self.slots[pi].last = child;
            if prev == u32::MAX {
                self.slots[pi].first = child;
            }
            self.slots[ci].parent = parent;
            if prev != u32::MAX {
                self.slots[ci].prev = prev;
                self.slots[Self::si(prev)].next = child;
            }
            return;
        }
        self.slots[ci].parent = parent;
        let prev = {
            let o = self.bovr_mut(parent);
            let prev = o.tail_last;
            if prev == u32::MAX {
                o.tail_head = child;
                o.tail_last = child;
            } else {
                o.tail_last = child;
            }
            prev
        };
        if prev != u32::MAX {
            self.slots[Self::si(prev)].next = child;
            self.slots[ci].prev = prev;
        }
    }

    fn unlink(&mut self, child: u32) {
        let ci = Self::si(child);
        let (parent, prev, next) = (
            self.slots[ci].parent,
            self.slots[ci].prev,
            self.slots[ci].next,
        );
        if parent == u32::MAX {
            return;
        }
        if Self::is_overlay(parent) {
            let pi = Self::si(parent);
            if self.slots[pi].first == child {
                self.slots[pi].first = next;
            }
            if self.slots[pi].last == child {
                self.slots[pi].last = prev;
            }
        } else if let Some(o) = self.bovr_get_mut(parent) {
            if o.tail_head == child {
                o.tail_head = next;
            }
            if o.tail_last == child {
                o.tail_last = prev;
            }
        }
        if prev != u32::MAX {
            self.slots[Self::si(prev)].next = next;
        }
        if next != u32::MAX {
            self.slots[Self::si(next)].prev = prev;
        }
        self.slots[ci].prev = u32::MAX;
        self.slots[ci].next = u32::MAX;
        self.slots[ci].parent = u32::MAX;
    }

    fn note_child_list(
        &mut self,
        target: u32,
        added: &[u32],
        removed: &[u32],
        prev_hint: u32,
        next_hint: u32,
    ) {
        self.tree_epoch = self.tree_epoch.wrapping_add(1);
        let prev = if let Some(&first) = added.first()
            && Self::is_overlay(first)
        {
            self.slots[Self::si(first)].prev
        } else {
            prev_hint
        };
        let next = if let Some(&last) = added.last()
            && Self::is_overlay(last)
        {
            self.slots[Self::si(last)].next
        } else {
            next_hint
        };
        let added_start = self.rec_ids.len() as u32;
        self.rec_ids.extend_from_slice(added);
        let removed_start = self.rec_ids.len() as u32;
        self.rec_ids.extend_from_slice(removed);
        let rec = MutRec {
            kind: 0,
            target,
            added: added_start,
            added_len: added.len() as u32,
            removed: removed_start,
            removed_len: removed.len() as u32,
            attr_name: CompactString::const_new(""),
            old_value: CompactString::const_new(""),
            prev_sibling: prev,
            next_sibling: next,
        };
        self.fanout(rec);
    }

    fn note_attr(&mut self, target: u32, name: &str, old: Option<&str>) {
        self.dom_epoch = self.dom_epoch.wrapping_add(1);
        let rec = MutRec {
            kind: 1,
            target,
            added: 0,
            added_len: 0,
            removed: 0,
            removed_len: 0,
            attr_name: CompactString::new(name),
            old_value: old.map(CompactString::new).unwrap_or_default(),
            prev_sibling: u32::MAX,
            next_sibling: u32::MAX,
        };
        self.fanout(rec);
    }

    fn note_character(&mut self, target: u32, old: &str) {
        self.dom_epoch = self.dom_epoch.wrapping_add(1);
        let rec = MutRec {
            kind: 2,
            target,
            added: 0,
            added_len: 0,
            removed: 0,
            removed_len: 0,
            attr_name: CompactString::const_new(""),
            old_value: CompactString::new(old),
            prev_sibling: u32::MAX,
            next_sibling: u32::MAX,
        };
        self.fanout(rec);
    }

    fn parent_step(&self, node: u32) -> Option<u32> {
        if Self::is_overlay(node) {
            let p = self.slots[Self::si(node)].parent;
            return (p != u32::MAX).then_some(p);
        }
        match self.place_of(node) {
            PLACE_NONE => with_doc(|d| d.and_then(|p| p.dom.parent(node))),
            PLACE_DETACHED => None,
            p => Some(p),
        }
    }

    fn walk_up_hit(&self, node: u32, root: u32) -> bool {
        let mut cur = node;
        for _ in 0..=512 {
            if cur == root {
                return true;
            }
            let Some(p) = self.parent_step(cur) else {
                return false;
            };
            cur = p;
        }
        false
    }

    fn is_in_subtree(&self, node: u32, root: u32) -> bool {
        if Self::is_overlay(node)
            || Self::is_overlay(root)
            || self.is_gone(node)
        {
            return self.walk_up_hit(node, root);
        }
        with_doc(|d| d.is_some_and(|p| p.dom.is_descendant(node, root)))
    }

    #[inline]
    fn is_gone(&self, n: u32) -> bool {
        self.place_of(n) != PLACE_NONE
    }

    fn live_base_child(&self, parent: u32, last: bool) -> Option<u32> {
        with_doc(|doc| {
            let p = doc?;
            if last {
                let mut cur = p.dom.last_child(parent);
                while cur != u32::MAX {
                    if !self.is_gone(cur) {
                        return Some(cur);
                    }
                    cur = p.dom.prev_sibling(cur);
                }
                return None;
            }
            for c in p.dom.children(parent) {
                if !self.is_gone(c) {
                    return Some(c);
                }
            }
            None
        })
    }

    fn live_base_sibling(&self, node: u32, forward: bool) -> Option<u32> {
        with_doc(|doc| {
            let p = doc?;
            let mut cur = if forward {
                p.dom.next_sibling(node)
            } else {
                p.dom.prev_sibling(node)
            };
            while cur != u32::MAX {
                if !self.is_gone(cur) {
                    return Some(cur);
                }
                cur = if forward {
                    p.dom.next_sibling(cur)
                } else {
                    p.dom.prev_sibling(cur)
                };
            }
            None
        })
    }

    fn matches(&self, reg: &ObsReg, rec: &MutRec) -> bool {
        let hit = rec.target == reg.target
            || (reg.opts.subtree() && self.is_in_subtree(rec.target, reg.target));
        if !hit {
            return false;
        }
        match rec.kind {
            0 => reg.opts.child_list(),
            1 => {
                reg.opts.attrs()
                    && (reg.opts.filter.is_empty() || reg.opts.filter.contains(&rec.attr_name))
            }
            _ => reg.opts.char_data(),
        }
    }
    fn fanout(&mut self, rec: MutRec) {
        if self.observers.is_empty() {
            return;
        }
        let idx = self.recs.len() as u32;
        self.recs.push(rec);
        let mut hits: SmallVec<[u32; 16]> = SmallVec::new();
        {
            let rec = &self.recs[idx as usize];
            for (i, reg) in self.observers.iter().enumerate() {
                if self.matches(reg, rec) {
                    hits.push(i as u32);
                }
            }
        }
        for i in hits {
            self.observers[i as usize].queue.push(idx);
        }
    }

    fn ids_of(&self, r: &MutRec) -> &[u32] {
        let s = r.added as usize;
        let e = s + r.added_len as usize;
        &self.rec_ids[s..e]
    }

    fn removed_ids_of(&self, r: &MutRec) -> &[u32] {
        let s = r.removed as usize;
        let e = s + r.removed_len as usize;
        &self.rec_ids[s..e]
    }
}

#[derive(Clone, Copy)]
enum NodeRef {
    Overlay(usize),
    Base(u32),
}

#[inline]
fn node_ref(id: u32) -> NodeRef {
    if MutDom::is_overlay(id) {
        NodeRef::Overlay(MutDom::si(id))
    } else {
        NodeRef::Base(id)
    }
}

type HandleMap = std::collections::HashMap<u32, Persistent<Object<'static>>, FxBuild>;
type ListenerMap = std::collections::HashMap<
    u32,
    SmallVec<[(CompactString, Persistent<Function<'static>>); 4]>,
    FxBuild,
>;

pub(crate) struct DomRuntime {
    dom: MutDom,
    handles: HandleMap,
    listeners: ListenerMap,
    protos: Vec<Option<Persistent<Object<'static>>>>,
    stubvals: Vec<Persistent<Object<'static>>>,
    eq_thunk: Option<Persistent<Function<'static>>>,
    doc_global: Option<Persistent<Object<'static>>>,
    reg_syms: Vec<Option<Persistent<rquickjs::Symbol<'static>>>>,
}

impl DomRuntime {
    fn new() -> Self {
        Self {
            dom: MutDom::new(),
            handles: fx_map(),
            listeners: fx_map(),
            protos: Vec::new(),
            stubvals: Vec::new(),
            eq_thunk: None,
            doc_global: None,
            reg_syms: Vec::new(),
        }
    }
}

thread_local! {
    static RT: RefCell<DomRuntime> = RefCell::new(DomRuntime::new());
    static ITER_MK: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
    static DEF_ACC: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
    static TAG_SYM: RefCell<Option<Persistent<rquickjs::Symbol<'static>>>> = const { RefCell::new(None) };
    static MK_EVENT: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
    static EMPTY_ARR: RefCell<Option<Persistent<Value<'static>>>> = const { RefCell::new(None) };
}

#[inline]
fn with_rt<R>(f: impl FnOnce(&mut DomRuntime) -> R) -> R {
    RT.with(|c| f(&mut c.borrow_mut()))
}

fn cached_persistent_rt<'js, R, T>(
    ctx: &Ctx<'js>,
    load: impl FnOnce() -> Option<Persistent<R>>,
    store: impl FnOnce(Persistent<R>),
    build: impl FnOnce(&Ctx<'js>) -> rquickjs::Result<T>,
) -> rquickjs::Result<T>
where
    R: JsLifetime<'static, Changed<'js> = T> + Clone,
    T: JsLifetime<'js, Changed<'static> = R> + Clone,
{
    if let Some(p) = load()
        && let Ok(v) = p.restore(ctx)
    {
        return Ok(v);
    }
    let v = build(ctx)?;
    store(Persistent::save(ctx, v.clone()));
    Ok(v)
}

pub(crate) fn cached_persistent<'js, R, T>(
    ctx: &Ctx<'js>,
    slot: &'static std::thread::LocalKey<RefCell<Option<Persistent<R>>>>,
    build: impl FnOnce(&Ctx<'js>) -> rquickjs::Result<T>,
) -> rquickjs::Result<T>
where
    R: JsLifetime<'static, Changed<'js> = T> + Clone,
    T: JsLifetime<'js, Changed<'static> = R> + Clone,
{
    cached_persistent_rt(
        ctx,
        || slot.with(|c| c.borrow().as_ref().cloned()),
        |p| slot.with(|c| *c.borrow_mut() = Some(p)),
        build,
    )
}

pub(crate) fn restore_persistent<'js, R, T>(ctx: &Ctx<'js>, p: Option<Persistent<R>>) -> Option<T>
where
    R: JsLifetime<'static, Changed<'js> = T>,
    T: JsLifetime<'js, Changed<'static> = R>,
{
    p?.restore(ctx).ok()
}

pub(crate) fn restore_slot<'js, R, T>(
    ctx: &Ctx<'js>,
    slot: &'static std::thread::LocalKey<RefCell<Option<Persistent<R>>>>,
) -> Option<T>
where
    R: JsLifetime<'static, Changed<'js> = T> + Clone,
    T: JsLifetime<'js, Changed<'static> = R>,
{
    restore_persistent(ctx, slot.with(|c| c.borrow().as_ref().cloned()))
}

pub(crate) const REG_INTL: usize = 0;
pub(crate) const REG_MO: usize = 1;
pub(crate) const REG_RO: usize = 2;
pub(crate) const REG_IO: usize = 3;

pub(crate) fn symbol_tostring_tag<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<rquickjs::Symbol<'js>> {
    cached_persistent(ctx, &TAG_SYM, |c| {
        let globals = c.globals();
        let sym_ctor: Object = globals.get("Symbol")?;
        sym_ctor.get("toStringTag")
    })
}

pub(crate) fn registry_symbol<'js>(
    ctx: &Ctx<'js>,
    slot: usize,
) -> rquickjs::Result<rquickjs::Symbol<'js>> {
    cached_persistent_rt(
        ctx,
        || with_rt(|rt| rt.reg_syms.get(slot).cloned().flatten()),
        |p| {
            with_rt(|rt| {
                if rt.reg_syms.len() <= slot {
                    rt.reg_syms.resize(slot + 1, None);
                }
                rt.reg_syms[slot] = Some(p);
            });
        },
        |c| rquickjs::Symbol::new(c.clone()),
    )
}

pub(crate) fn class_tag<'js>(
    ctx: &Ctx<'js>,
    obj: &Object<'js>,
    tag: &'static str,
) -> rquickjs::Result<()> {
    let sym = symbol_tostring_tag(ctx)?;
    obj.prop(sym, rquickjs::object::Property::from(tag).configurable())?;
    Ok(())
}

pub(crate) fn clear_handles() {
    with_rt(|rt| rt.handles.clear());
}

pub(crate) fn reset_request() {
    with_rt(|rt| {
        rt.dom.clear();
        rt.handles.clear();
        rt.listeners.clear();
    });
}

pub(crate) fn clear() {
    with_rt(|rt| {
        rt.protos.clear();
        rt.stubvals.clear();
        rt.eq_thunk = None;
        rt.doc_global = None;
        rt.reg_syms.clear();
    });
    RT.with(|c| *c.borrow_mut() = DomRuntime::new());
    ITER_MK.with(|c| *c.borrow_mut() = None);
    DEF_ACC.with(|c| *c.borrow_mut() = None);
    TAG_SYM.with(|c| *c.borrow_mut() = None);
    MK_EVENT.with(|c| *c.borrow_mut() = None);
    EMPTY_ARR.with(|c| *c.borrow_mut() = None);
}

fn eq_thunk_fn<'js>(ctx: &Ctx<'js>) -> Option<Function<'js>> {
    restore_persistent(ctx, with_rt(|rt| rt.eq_thunk.clone()))
}

pub(crate) fn eq_thunk_same<'js>(ctx: &Ctx<'js>, a: &Value<'js>, b: &Value<'js>) -> bool {
    match eq_thunk_fn(ctx) {
        Some(f) => f.call::<_, bool>((a.clone(), b.clone())).unwrap_or(false),
        None => false,
    }
}

const ITER_SRC: &str = "(function (a) { var i = 0; var out = {}; out[Symbol.iterator] = function () { return { next: function () { return i < a.length ? { value: a[i++], done: false } : { value: void 0, done: true }; } }; }; return out; })";

pub(crate) fn iterable_of_array<'js>(
    ctx: &Ctx<'js>,
    arr: &rquickjs::Array<'js>,
) -> rquickjs::Result<Value<'js>> {
    let mk = cached_persistent(ctx, &ITER_MK, |c| c.eval(ITER_SRC))?;
    mk.call((arr.clone(),))
}

pub(crate) fn store_stub<'js>(ctx: &Ctx<'js>, val: Object<'js>) -> usize {
    with_rt(|rt| {
        rt.stubvals.push(Persistent::save(ctx, val));
        rt.stubvals.len() - 1
    })
}

pub(crate) fn stub_value<'js>(ctx: &Ctx<'js>, slot: usize) -> Option<Object<'js>> {
    restore_persistent(ctx, with_rt(|rt| rt.stubvals.get(slot).cloned()))
}

pub(crate) fn stub_getter<'js>(
    ctx: &Ctx<'js>,
    slot: usize,
    touch: Option<u32>,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            if let Some(key) = touch {
                touch::touch_log_record(key);
            }
            match stub_value(&c, slot) {
                Some(o) => Ok(o.into_value()),
                None => Ok(Value::new_undefined(c)),
            }
        },
    )
}

const REG_SLOT_BITS: u32 = 20;
const REG_SLOT_MASK: u64 = (1 << REG_SLOT_BITS) - 1;
const REG_GEN_BITS: u32 = 28;
const REG_GEN_MASK: u64 = (1 << REG_GEN_BITS) - 1;
const REG_TAG_SHIFT: u32 = REG_SLOT_BITS + REG_GEN_BITS;

pub(crate) struct RegVec<S> {
    slots: Vec<Option<S>>,
    gens: Vec<u32>,
    free: Vec<u32>,
    live: usize,
    tag: u64,
}

pub(crate) type RegCell<S> = RefCell<RegVec<S>>;

impl<S> RegVec<S> {
    pub(crate) const fn with_tag(tag: u8) -> Self {
        assert!(
            (tag as u32) < (1 << (53 - REG_TAG_SHIFT)),
            "reg tag overflows f64 id precision"
        );
        Self {
            slots: Vec::new(),
            gens: Vec::new(),
            free: Vec::new(),
            live: 0,
            tag: tag as u64,
        }
    }

    #[inline]
    pub(crate) fn live(&self) -> usize {
        self.live
    }

    pub(crate) fn alloc(&mut self, fresh: impl FnOnce(u64) -> S) -> u64 {
        let slot = match self.free.pop() {
            Some(s) => s as usize,
            None => {
                self.slots.push(None);
                self.gens.push(0);
                self.slots.len() - 1
            }
        };
        let g = self.gens[slot].wrapping_add(1) & (REG_GEN_MASK as u32);
        self.gens[slot] = if g == 0 { 1 } else { g };
        self.live += 1;
        let id =
            (self.tag << REG_TAG_SHIFT) | ((self.gens[slot] as u64) << REG_SLOT_BITS) | slot as u64;
        self.slots[slot] = Some(fresh(id));
        id
    }

    #[inline]
    fn resolve(&self, id: u64) -> Option<usize> {
        if id >> REG_TAG_SHIFT != self.tag {
            return None;
        }
        let slot = (id & REG_SLOT_MASK) as usize;
        let g = ((id >> REG_SLOT_BITS) & REG_GEN_MASK) as u32;
        (slot < self.slots.len() && self.gens[slot] == g && self.slots[slot].is_some())
            .then_some(slot)
    }

    pub(crate) fn with<R>(&mut self, id: u64, f: impl FnOnce(&mut S) -> R) -> Option<R> {
        let slot = self.resolve(id)?;
        self.slots[slot].as_mut().map(f)
    }

    pub(crate) fn get_ro<R>(&self, id: u64, f: impl FnOnce(&S) -> R) -> Option<R> {
        let slot = self.resolve(id)?;
        self.slots[slot].as_ref().map(f)
    }

    pub(crate) fn remove(&mut self, id: u64) -> Option<S> {
        let slot = self.resolve(id)?;
        let v = self.slots[slot].take()?;
        self.live -= 1;
        self.free.push(slot as u32);
        Some(v)
    }

    pub(crate) fn clear(&mut self) {
        for s in self.slots.iter_mut() {
            *s = None;
        }
        self.free.clear();
        self.free.extend(0..self.slots.len() as u32);
        self.live = 0;
    }
}

pub(crate) fn install_noop_listeners<'js>(
    ctx: &Ctx<'js>,
    obj: &Object<'js>,
) -> rquickjs::Result<()> {
    let add = Function::new(
        ctx.clone(),
        |_c: Ctx<'js>, _k: Value<'js>, _f: Value<'js>| {},
    )?;
    set_fn_name(ctx, &add, "addEventListener")?;
    obj.set("addEventListener", add)?;
    let rm = Function::new(
        ctx.clone(),
        |_c: Ctx<'js>, _k: Value<'js>, _f: Value<'js>| {},
    )?;
    set_fn_name(ctx, &rm, "removeEventListener")?;
    obj.set("removeEventListener", rm)?;
    Ok(())
}

pub(crate) fn ta_bytes<'js>(v: &Value<'js>) -> Option<&'js [u8]> {
    let raw = v
        .as_object()
        .and_then(|o| o.as_typed_array::<u8>())
        .and_then(|t| t.as_raw())?;
    Some(unsafe { std::slice::from_raw_parts(raw.ptr.as_ptr(), raw.len) })
}

pub(crate) fn value_to_str(v: &Value<'_>) -> Option<compact_str::CompactString> {
    let cs = v.as_string().cloned()?.to_cstring().ok()?;
    Some(compact_str::CompactString::new(cs.as_str()))
}

pub(crate) fn ab_bytes<'js>(v: &Value<'js>) -> Option<&'js [u8]> {
    let raw = v
        .as_object()
        .and_then(|o| o.as_array_buffer())
        .and_then(|b| b.as_raw())?;
    Some(unsafe { std::slice::from_raw_parts(raw.ptr.as_ptr(), raw.len) })
}

pub(crate) unsafe fn ta_bytes_mut<'js>(v: &Value<'js>) -> Option<&'js mut [u8]> {
    let raw = v
        .as_object()
        .and_then(|o| o.as_typed_array::<u8>())
        .and_then(|t| t.as_raw())?;
    Some(unsafe { std::slice::from_raw_parts_mut(raw.ptr.as_ptr(), raw.len) })
}

pub(crate) unsafe fn ab_bytes_mut<'js>(v: &Value<'js>) -> Option<&'js mut [u8]> {
    let raw = v
        .as_object()
        .and_then(|o| o.as_array_buffer())
        .and_then(|b| b.as_raw())?;
    Some(unsafe { std::slice::from_raw_parts_mut(raw.ptr.as_ptr(), raw.len) })
}

pub(crate) fn value_to_bytes(v: &Value<'_>) -> Option<bytes::Bytes> {
    if let Some(s) = v.as_string() {
        return s.to_string().ok().map(|x| Bytes::from(x.into_bytes()));
    }
    if let Some(b) = ab_bytes(v) {
        return Some(Bytes::copy_from_slice(b));
    }
    if let Some(b) = ta_bytes(v) {
        return Some(Bytes::copy_from_slice(b));
    }
    let o = v.as_object()?;
    if let Ok(Some(b)) = o.get::<_, Option<String>>("body") {
        return Some(Bytes::from(b.into_bytes()));
    }
    None
}

pub(crate) fn install_host_ctor<'js>(
    ctx: &Ctx<'js>,
    ctor: &Function<'js>,
    proto: &Object<'js>,
    name: &'static str,
    len: u32,
    expose_global: bool,
) -> rquickjs::Result<()> {
    set_fn_name(ctx, ctor, name)?;
    ctor.prop("length", rquickjs::object::Property::from(len as f64))?;
    ctor.prop("prototype", rquickjs::object::Property::from(proto.clone()))?;
    proto.prop(
        "constructor",
        rquickjs::object::Property::from(ctor.clone())
            .writable()
            .configurable(),
    )?;
    let tag = symbol_tostring_tag(ctx)?;
    proto.prop(tag, rquickjs::object::Property::from(name).configurable())?;
    if expose_global {
        let globals = ctx.globals();
        globals.prop(
            name,
            rquickjs::object::Property::from(ctor.clone())
                .writable()
                .configurable(),
        )?;
    }
    Ok(())
}

pub(crate) fn is_overlay_id(id: u32) -> bool {
    MutDom::is_overlay(id)
}

pub(crate) fn mut_epoch() -> u64 {
    with_rt(|rt| rt.dom.tree_epoch.wrapping_add(rt.dom.dom_epoch))
}

pub(crate) fn view_epoch(dgen: u64) -> u64 {
    (dgen << 20) ^ mut_epoch()
}

pub(crate) fn illegal_ctor_fn<'js>(
    ctx: &Ctx<'js>,
    name: &'static str,
) -> rquickjs::Result<Function<'js>> {
    let f = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            Err(rquickjs::Exception::throw_message(
                &c,
                "TypeError: Illegal constructor",
            ))
        },
    )?
    .with_constructor(true);
    set_fn_name(ctx, &f, name)?;
    Ok(f)
}

fn illegal_ctor<'js>(ctx: &Ctx<'js>, iface: InterfaceId) -> rquickjs::Result<Function<'js>> {
    illegal_ctor_fn(ctx, iface_name(iface))
}

fn define_data<'js>(
    obj: &Object<'js>,
    key: &str,
    val: Value<'js>,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) -> rquickjs::Result<()> {
    let mut p = rquickjs::object::Property::from(val);
    if writable {
        p = p.writable();
    }
    if enumerable {
        p = p.enumerable();
    }
    if configurable {
        p = p.configurable();
    }
    obj.prop(key, p)
}

pub(crate) fn define_method<'js>(
    ctx: &Ctx<'js>,
    proto: &Object<'js>,
    key: &'static str,
    f: Function<'js>,
) -> rquickjs::Result<()> {
    set_fn_name(ctx, &f, key)?;
    proto.prop(
        key,
        rquickjs::object::Property::from(f)
            .writable()
            .enumerable()
            .configurable(),
    )
}

pub(crate) fn named_accessor<'js>(
    ctx: &Ctx<'js>,
    proto: &Object<'js>,
    key: &str,
    g: Function<'js>,
    s: Option<Function<'js>>,
) -> rquickjs::Result<()> {
    let mut gname = CompactString::with_capacity(key.len() + 4);
    gname.push_str("get ");
    gname.push_str(key);
    set_fn_name(ctx, &g, gname.as_str())?;
    if let Some(s) = &s {
        let mut sname = CompactString::with_capacity(key.len() + 4);
        sname.push_str("set ");
        sname.push_str(key);
        set_fn_name(ctx, s, sname.as_str())?;
    }
    let thunk = cached_persistent(ctx, &DEF_ACC, |c| {
        c.eval("(function (o, n, g, s) { var d = { get: g, enumerable: true, configurable: true }; if (s) { d.set = s; } Object.defineProperty(o, n, d); })")
    })?;
    let _: Value<'js> = thunk.call((proto.clone(), key, g, s))?;
    Ok(())
}

fn make_ctor<'js>(
    ctx: &Ctx<'js>,
    iface: InterfaceId,
    proto: &Object<'js>,
) -> rquickjs::Result<Function<'js>> {
    let ctor = illegal_ctor(ctx, iface)?;
    install_host_ctor(ctx, &ctor, proto, iface_name(iface), 0, true)?;
    Ok(ctor)
}

fn ensure_prototype<'js>(ctx: &Ctx<'js>, iface: InterfaceId) -> rquickjs::Result<Object<'js>> {
    if let Some(c) = with_rt(|rt| rt.protos.get(iface as u16 as usize).cloned().flatten()) {
        return c.restore(ctx);
    }
    let proto = Object::new(ctx.clone())?;
    if let Some(p) = parent_of(iface) {
        let parent = ensure_prototype(ctx, p)?;
        proto.set_prototype(Some(&parent))?;
    }
    let ctor = make_ctor(ctx, iface, &proto)?;
    let globals = ctx.globals();
    define_data(
        &globals,
        iface_name(iface),
        ctor.into_value(),
        true,
        false,
        true,
    )?;
    with_rt(|rt| {
        if rt.protos.len() <= iface as u16 as usize {
            rt.protos.resize(iface as u16 as usize + 1, None);
        }
        rt.protos[iface as u16 as usize] = Some(Persistent::save(ctx, proto.clone()));
    });
    Ok(proto)
}

pub(crate) fn prototype<'js>(ctx: &Ctx<'js>, iface: InterfaceId) -> rquickjs::Result<Object<'js>> {
    ensure_prototype(ctx, iface)
}



pub(crate) fn node_of_value(v: &Value<'_>) -> Option<u32> {
    let obj = v.as_object()?;
    let c = Class::<NodeHandle>::from_object(obj)?;
    Some(c.borrow().node)
}

fn this_node(this: &Value<'_>) -> Option<u32> {
    node_of_value(this)
}

pub(crate) fn chrome_exec_error(
    ctx: &Ctx<'_>,
    method: &str,
    iface: &str,
    detail: &str,
) -> rquickjs::Error {
    let mut msg = CompactString::with_capacity(56 + method.len() + iface.len() + detail.len());
    msg.push_str("TypeError: Failed to execute '");
    msg.push_str(method);
    msg.push_str("' on '");
    msg.push_str(iface);
    msg.push_str("': ");
    msg.push_str(detail);
    rquickjs::Exception::throw_type(ctx, msg.as_str())
}

pub(crate) fn param_not_type(
    ctx: &Ctx<'_>,
    method: &str,
    iface: &str,
    param: u8,
    ty: &str,
) -> rquickjs::Error {
    let mut d = CompactString::with_capacity(24 + ty.len());
    d.push_str("parameter ");
    core_utils::math::push_int_into(&mut d, param as i64);
    d.push_str(" is not of type '");
    d.push_str(ty);
    d.push_str("'.");
    chrome_exec_error(ctx, method, iface, d.as_str())
}

pub(crate) fn iface_of_node(node: u32) -> InterfaceId {
    if let NodeRef::Overlay(i) = node_ref(node) {
        return with_rt(|rt| {
            InterfaceId::from_u16(rt.dom.iface_at(i)).unwrap_or(InterfaceId::HTMLElement)
        });
    }
    let (tag, flags) = with_doc(|d| {
        d.map(|p| (p.dom.tag_id(node), p.dom.flags(node)))
            .unwrap_or((parser_pipeline::dom::TAG_UNKNOWN, 0))
    });
    if flags & parser_pipeline::dom::node_flags::TEXT != 0 {
        return InterfaceId::Text;
    }
    if flags & parser_pipeline::dom::node_flags::ELEMENT == 0 {
        return InterfaceId::Node;
    }
    if tag < 200 {
        IFACE_BY_TAG[tag as usize]
    } else {
        iface_for_tag(tag)
    }
}
pub(crate) fn to_compact(v: &Value<'_>) -> CompactString {
    value_to_str(v).unwrap_or_default()
}

pub(crate) fn empty_array_value<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    cached_persistent(ctx, &EMPTY_ARR, |c| {
        Ok(rquickjs::Array::new(c.clone())?.into_value())
    })
}

pub(crate) fn read_hidden_id(obj: &Object<'_>, slot: usize) -> Option<u64> {
    let ctx = obj.ctx();
    let sym = registry_symbol(ctx, slot).ok()?;
    obj.get::<_, Option<f64>>(sym)
        .ok()
        .flatten()
        .map(|v| v as u64)
}

pub(crate) fn this_id<'js>(
    c: &Ctx<'js>,
    this: &Value<'js>,
    reg_key: usize,
) -> rquickjs::Result<u64> {
    let Some(obj) = this.as_object() else {
        return throw_illegal(c);
    };
    match read_hidden_id(&obj, reg_key) {
        Some(id) => Ok(id),
        None => throw_illegal(c),
    }
}

pub(crate) fn registry_instance<'js>(
    c: &Ctx<'js>,
    tag: &'static str,
    reg_key: usize,
    id: u64,
    proto: &Object<'js>,
) -> rquickjs::Result<Value<'js>> {
    let o = Object::new_proto(c.clone(), Some(proto))?;
    class_tag(c, &o, tag)?;
    let sym = registry_symbol(c, reg_key)?;
    o.prop(sym, rquickjs::object::Property::from(id as f64).writable())?;
    Ok(o.into_value())
}

pub(crate) fn registry_ctor<'js, S: 'static>(
    ctx: &Ctx<'js>,
    reg: &'static std::thread::LocalKey<RegCell<S>>,
    cap: usize,
    err: &'static str,
    api: u32,
    fresh: impl FnOnce(u64) -> S,
    make: impl FnOnce(&Ctx<'js>, u64) -> rquickjs::Result<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    touch::touch_log_record(api);
    if reg.with(|m| m.borrow().live() >= cap) {
        return Err(rquickjs::Exception::throw_message(ctx, err));
    }
    let id = reg.with(|m| m.borrow_mut().alloc(fresh));
    make(ctx, id)
}

pub(crate) fn call_cb_with_array<'js, F>(
    ctx: &Ctx<'js>,
    cb: &Persistent<Value<'static>>,
    fill: F,
) -> rquickjs::Result<()>
where
    F: FnOnce(&rquickjs::Array<'js>) -> rquickjs::Result<()>,
{
    let f = rquickjs::function::Function::from_value(cb.clone().restore(ctx)?)?;
    let arr = rquickjs::Array::new(ctx.clone())?;
    fill(&arr)?;
    let _: rquickjs::Result<Value<'js>> = f.call((arr, Value::new_undefined(ctx.clone())));
    Ok(())
}

pub(crate) fn create_element<'js>(ctx: &Ctx<'js>, tag: &str) -> rquickjs::Result<Value<'js>> {
    if tag.is_empty() {
        return Err(rquickjs::Exception::throw_message(
            ctx,
            "InvalidCharacterError: The tag name provided is not a valid name.",
        ));
    }
    let mut tag_lower = CompactString::with_capacity(tag.len());
    core_utils::push_ascii_case_into(&mut tag_lower, tag, false);
    if tag_lower.as_str().bytes().any(|b| {
        matches!(b, b' ' | b'<' | b'>' | b'/' | b'"' | b'\'' | b'=' | 0) || (b < 0x20 && b != 0)
    }) {
        return Err(rquickjs::Exception::throw_message(
            ctx,
            "InvalidCharacterError: The tag name provided is not a valid name.",
        ));
    }
    if tag_lower.as_str() == "canvas" {
        return crate::canvas2d::new_canvas(ctx);
    }
    let (iface, tag_id) = if tag_lower.as_str().starts_with("svg") {
        (
            InterfaceId::SvgElement,
            parser_pipeline::dom::TAGS
                .get(tag_lower.as_str())
                .copied()
                .unwrap_or(parser_pipeline::dom::TAG_UNKNOWN),
        )
    } else {
        iface_for_tag_name(tag_lower.as_str())
    };
    let node = with_rt(|rt| {
        let name = (tag_id == parser_pipeline::dom::TAG_UNKNOWN).then(|| CompactString::new(tag_lower.as_str()));
        rt.dom.alloc(KIND_ELEMENT, iface, tag_id, name)
    });
    crate::worker::handle_value_with_iface(ctx, node, iface)
}

fn alloc_text(text: &str) -> u32 {
    with_rt(|rt| {
        rt.dom.alloc(
            KIND_TEXT,
            InterfaceId::Text,
            parser_pipeline::dom::TAG_UNKNOWN,
            Some(CompactString::new(text)),
        )
    })
}

fn alloc_handle<'js>(
    ctx: &Ctx<'js>,
    kind: u8,
    iface: InterfaceId,
    text: Option<&str>,
) -> rquickjs::Result<Value<'js>> {
    let node = with_rt(|rt| {
        rt.dom.alloc(
            kind,
            iface,
            parser_pipeline::dom::TAG_UNKNOWN,
            text.map(CompactString::new),
        )
    });
    crate::worker::handle_value_with_iface(ctx, node, iface)
}

pub(crate) fn create_text_node<'js>(ctx: &Ctx<'js>, text: &str) -> rquickjs::Result<Value<'js>> {
    alloc_handle(ctx, KIND_TEXT, InterfaceId::Text, Some(text))
}

pub(crate) fn create_fragment<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    alloc_handle(ctx, KIND_FRAGMENT, InterfaceId::DocumentFragment, None)
}

pub(crate) fn create_event_target<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    alloc_handle(ctx, KIND_EVENT_TARGET, InterfaceId::EventTarget, None)
}

pub(crate) fn store_handle<'js>(ctx: &Ctx<'js>, node: u32, obj: Object<'js>) {
    with_rt(|rt| {
        rt.handles.insert(node, Persistent::save(ctx, obj));
    });
}

#[inline]
pub(crate) fn lookup_handle_raw(
    node: u32,
) -> (
    Option<Persistent<Value<'static>>>,
    Option<Persistent<Object<'static>>>,
) {
    with_rt(|rt| {
        (
            rt.dom.external.get(&node).cloned(),
            rt.handles.get(&node).cloned(),
        )
    })
}

pub(crate) fn style_of<'js>(ctx: &Ctx<'js>, node: u32) -> rquickjs::Result<Value<'js>> {
    Ok(cached_persistent_rt(
        ctx,
        || with_rt(|rt| rt.dom.styles.get(&node).cloned()),
        |p| {
            with_rt(|rt| {
                rt.dom.styles.insert(node, p);
            });
        },
        |c| Object::new(c.clone()),
    )?
    .into_value())
}

pub(crate) fn listen_add<'js>(
    ctx: &Ctx<'js>,
    node: u32,
    type_: &str,
    fun: Function<'js>,
) -> rquickjs::Result<()> {
    with_rt(|rt| {
        rt.listeners
            .entry(node)
            .or_default()
            .push((CompactString::new(type_), Persistent::save(ctx, fun)));
    });
    Ok(())
}

pub(crate) fn listen_remove<'js>(
    ctx: &Ctx<'js>,
    node: u32,
    type_: &str,
    f: &Value<'js>,
) -> rquickjs::Result<()> {
    let Some(eq) = eq_thunk_fn(ctx) else {
        return Ok(());
    };
    let matched: SmallVec<[(usize, Function<'js>); 4]> = with_rt(|rt| {
        rt.listeners
            .get(&node)
            .map(|list| {
                list.iter()
                    .enumerate()
                    .filter(|(_, (t, _))| t.as_str() == type_)
                    .filter_map(|(i, (_, p))| p.clone().restore(ctx).ok().map(|f| (i, f)))
                    .collect()
            })
            .unwrap_or_default()
    });
    if matched.is_empty() {
        return Ok(());
    }
    let mut dead: SmallVec<[usize; 4]> = SmallVec::new();
    for (i, stored) in matched {
        if eq.call::<_, bool>((stored, f.clone())).unwrap_or(false) {
            dead.push(i);
        }
    }
    if dead.is_empty() {
        return Ok(());
    }
    with_rt(|rt| {
        if let Some(list) = rt.listeners.get_mut(&node) {
            for i in dead.into_iter().rev() {
                if i < list.len() {
                    list.remove(i);
                }
            }
        }
    });
    Ok(())
}

pub(crate) fn listen_dispatch<'js>(
    ctx: &Ctx<'js>,
    node: u32,
    ev: &Value<'js>,
) -> rquickjs::Result<bool> {
    let Some(type_) = ev
        .as_object()
        .and_then(|o| o.get::<_, rquickjs::String<'js>>("type").ok())
        .and_then(|s| s.to_cstring().ok())
    else {
        return Ok(false);
    };
    let saved: SmallVec<[Persistent<Function<'static>>; 4]> = with_rt(|rt| {
        rt.listeners
            .get(&node)
            .map(|list| {
                list.iter()
                    .filter(|(t, _)| t.as_str() == type_.as_str())
                    .map(|(_, p)| p.clone())
                    .collect()
            })
            .unwrap_or_default()
    });
    for p in saved {
        if let Ok(f) = p.restore(ctx) {
            let _: rquickjs::Result<Value<'js>> = f.call((ev.clone(),));
        }
    }
    Ok(true)
}

fn append_child<'js>(
    ctx: Ctx<'js>,
    this: This<Value<'js>>,
    child: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let Some(parent) = this_node(&this.0) else {
        return throw_illegal(&ctx);
    };
    if crate::canvas2d::is_canvas_value(&child) {
        let slot = alloc_element("canvas", InterfaceId::HtmlCanvasElement);
        with_rt(|rt| {
            let d = &mut rt.dom;
            d.external
                .insert(slot, Persistent::save(&ctx, child.clone()));
            d.link(parent, slot);
            d.note_child_list(parent, &[slot], &[], u32::MAX, u32::MAX);
        });
        return Ok(child);
    }
    let Some(child_node) = node_of_value(&child) else {
        return Err(param_not_type(&ctx, "appendChild", "Node", 1, "Node"));
    };
    if MutDom::is_overlay(child_node) {
        let (rp, rn, already) = with_rt(|rt| {
            let d = &rt.dom;
            let i = MutDom::si(child_node);
            (d.slots[i].prev, d.slots[i].next, d.slots[i].parent)
        });
        let mut removed: SmallVec<[u32; 4]> = SmallVec::new();
        if already != u32::MAX {
            with_rt(|rt| rt.dom.unlink(child_node));
            removed.push(child_node);
        }
        with_rt(|rt| {
            let d = &mut rt.dom;
            d.link(parent, child_node);
            d.note_child_list(parent, &[child_node], &removed, rp, rn);
        });
        return Ok(child);
    }
    with_rt(|rt| {
        let d = &mut rt.dom;
        d.bovr_mut(child_node).place = parent;
        if let Some(o) = d.bovr_get_mut(parent) {
            o.adopted.retain(|x| *x != child_node);
            o.adopted.push(child_node);
        }
        d.note_child_list(parent, &[child_node], &[], u32::MAX, u32::MAX);
    });
    Ok(child)
}

fn detach_and_note(d: &mut MutDom, parent: u32, child: u32) {
    let (rp, rn) = detach_node(d, child);
    if let Some(o) = d.bovr_get_mut(parent) {
        o.adopted.retain(|x| *x != child);
    }
    d.note_child_list(parent, &[], &[child], rp, rn);
}

fn detach_node(d: &mut MutDom, n: u32) -> (u32, u32) {
    if MutDom::is_overlay(n) {
        let i = MutDom::si(n);
        let (rp, rn) = (d.slots[i].prev, d.slots[i].next);
        d.unlink(n);
        d.external.remove(&n);
        (rp, rn)
    } else {
        d.bovr_mut(n).place = PLACE_DETACHED;
        (u32::MAX, u32::MAX)
    }
}

fn remove_child<'js>(
    ctx: Ctx<'js>,
    this: This<Value<'js>>,
    child: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let Some(parent) = this_node(&this.0) else {
        return throw_illegal(&ctx);
    };
    let Some(child_node) = node_of_value(&child) else {
        return Err(param_not_type(&ctx, "removeChild", "Node", 1, "Node"));
    };
    let actual_parent = view_parent(child_node);
    if actual_parent != Some(parent) {
        return Err(rquickjs::Exception::throw_message(
            &ctx,
            "NotFoundError: Failed to execute 'removeChild' on 'Node': The node to be removed is not a child of this node.",
        ));
    }
    with_rt(|rt| {
        detach_and_note(&mut rt.dom, parent, child_node);
    });
    Ok(child)
}

fn insert_before<'js>(
    ctx: Ctx<'js>,
    this: This<Value<'js>>,
    child: Value<'js>,
    reference: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let Some(parent) = this_node(&this.0) else {
        return throw_illegal(&ctx);
    };
    let Some(child_node) = node_of_value(&child) else {
        return Err(param_not_type(&ctx, "insertBefore", "Node", 1, "Node"));
    };
    let ref_node = if reference.is_null() || reference.is_undefined() {
        None
    } else {
        match node_of_value(&reference) {
            Some(rn) => Some(rn),
            None => return Err(param_not_type(&ctx, "insertBefore", "Node", 2, "Node")),
        }
    };
    if ref_node.is_some_and(|r| view_parent(r) != Some(parent)) {
        return Err(rquickjs::Exception::throw_message(
            &ctx,
            "NotFoundError: Failed to execute 'insertBefore' on 'Node': The node before which the new node is to be inserted is not a child of this node.",
        ));
    }
    if let Some(r) = ref_node {
        with_rt(|rt| {
            let d = &mut rt.dom;
            d.unlink(child_node);
            if !MutDom::is_overlay(child_node) {
                let old_place = d.bovr(child_node).map(|o| o.place).unwrap_or(0);
                if old_place != parent && old_place != u32::MAX {
                    if let Some(o) = d.bovr_get_mut(old_place) {
                        o.adopted.retain(|x| *x != child_node);
                    }
                }
                d.bovr_mut(child_node).place = parent;
                if let Some(o) = d.bovr_get_mut(parent) {
                    o.adopted.retain(|x| *x != child_node);
                    o.adopted.push(child_node);
                }
            }
            let ri = MutDom::si(r);
            let prev = d.slots[ri].prev;
            let r_parent = d.slots[ri].parent;
            if prev != u32::MAX {
                d.slots[MutDom::si(prev)].next = child_node;
            } else if MutDom::is_overlay(r_parent) {
                d.slots[MutDom::si(r_parent)].first = child_node;
            } else if let Some(o) = d.bovr_get_mut(r_parent) {
                o.tail_head = child_node;
            }
            d.slots[ri].prev = child_node;
            d.slots[MutDom::si(child_node)].next = r;
            d.slots[MutDom::si(child_node)].prev = prev;
            d.slots[MutDom::si(child_node)].parent = r_parent;
            d.note_child_list(parent, &[child_node], &[], prev, r);
        });
    } else {
        return append_child(ctx, this, child);
    }
    Ok(child)
}

fn remove_self(this: &Value<'_>) -> rquickjs::Result<()> {
    let Some(n) = this_node(this) else {
        return Ok(());
    };
    let parent = view_parent(n);
    with_rt(|rt| {
        if let Some(p) = parent {
            detach_and_note(&mut rt.dom, p, n);
        } else {
            detach_node(&mut rt.dom, n);
        }
    });
    Ok(())
}

pub(crate) fn view_parent(node: u32) -> Option<u32> {
    with_rt(|rt| rt.dom.parent_step(node))
}

pub(crate) fn view_children(node: u32) -> SmallVec<[u32; 16]> {
    let mut out: SmallVec<[u32; 16]> = SmallVec::new();
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| {
            let d = &rt.dom;
            let mut cur = d.slots[i].first;
            while cur != u32::MAX {
                out.push(cur);
                cur = d.slots[MutDom::si(cur)].next;
            }
        }),
        NodeRef::Base(node) => with_rt(|rt| {
            let d = &rt.dom;
            if let Some(o) = d.bovr(node) {
                out.extend(o.adopted.iter().copied());
                let mut cur = o.tail_head;
                while cur != u32::MAX {
                    out.push(cur);
                    cur = d.slots[MutDom::si(cur)].next;
                }
            }
            with_doc(|doc| {
                if let Some(p) = doc {
                    for c in p.dom.children(node) {
                        if !d.is_gone(c) {
                            out.push(c);
                        }
                    }
                }
            });
        }),
    }
    out
}

pub(crate) fn view_first(node: u32) -> Option<u32> {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| {
            let v = rt.dom.slots[i].first;
            (v != u32::MAX).then_some(v)
        }),
        NodeRef::Base(node) => with_rt(|rt| {
            let d = &rt.dom;
            if let Some(o) = d.bovr(node) {
                if let Some(&f) = o.adopted.first() {
                    return Some(f);
                }
                if o.tail_head != u32::MAX {
                    return Some(o.tail_head);
                }
            }
            d.live_base_child(node, false)
        }),
    }
}

pub(crate) fn view_last(node: u32) -> Option<u32> {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| {
            let v = rt.dom.slots[i].last;
            (v != u32::MAX).then_some(v)
        }),
        NodeRef::Base(node) => with_rt(|rt| {
            let d = &rt.dom;
            let base_last = d.live_base_child(node, true);
            if base_last.is_some() {
                return base_last;
            }
            if let Some(o) = d.bovr(node) {
                if o.tail_last != u32::MAX {
                    return Some(o.tail_last);
                }
                return o.adopted.last().copied();
            }
            None
        }),
    }
}

pub(crate) fn view_next(node: u32) -> Option<u32> {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| {
            let d = &rt.dom;
            let v = d.slots[i].next;
            if v != u32::MAX {
                return Some(v);
            }
            let parent = d.slots[i].parent;
            if parent == u32::MAX || MutDom::is_overlay(parent) {
                return None;
            }
            d.live_base_child(parent, false)
        }),
        NodeRef::Base(node) => {
            let parent = view_parent(node)?;
            if MutDom::is_overlay(parent) {
                return None;
            }
            with_rt(|rt| {
                let d = &rt.dom;
                let place = d.place_of(node);
                if place != PLACE_NONE && place != PLACE_DETACHED {
                    let o = d.bovr(place);
                    let pos = o
                        .map(|o| o.adopted.iter().position(|&x| x == node))
                        .flatten();
                    if let Some(pos) = pos {
                        if let Some(&nx) = o.and_then(|o| o.adopted.get(pos + 1)) {
                            return Some(nx);
                        }
                        let th = o.map(|o| o.tail_head).unwrap_or(u32::MAX);
                        if th != u32::MAX {
                            return Some(th);
                        }
                        return d.live_base_child(place, false);
                    }
                    return None;
                }
                d.live_base_sibling(node, true)
            })
        }
    }
}

pub(crate) fn view_prev(node: u32) -> Option<u32> {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| {
            let d = &rt.dom;
            let v = d.slots[i].prev;
            if v != u32::MAX {
                return Some(v);
            }
            let parent = d.slots[i].parent;
            if parent == u32::MAX || MutDom::is_overlay(parent) {
                return None;
            }
            d.bovr(parent).and_then(|o| o.adopted.last().copied())
        }),
        NodeRef::Base(node) => {
            let parent = view_parent(node)?;
            if MutDom::is_overlay(parent) {
                return None;
            }
            with_rt(|rt| {
                let d = &rt.dom;
                let place = d.place_of(node);
                if place != PLACE_NONE && place != PLACE_DETACHED {
                    let o = d.bovr(place);
                    let pos = o
                        .map(|o| o.adopted.iter().position(|&x| x == node))
                        .flatten();
                    if let Some(pos) = pos {
                        if pos > 0 {
                            return o.and_then(|o| o.adopted.get(pos - 1).copied());
                        }
                        return None;
                    }
                    return None;
                }
                let base_prev = d.live_base_sibling(node, false);
                if base_prev.is_some() {
                    return base_prev;
                }
                if let Some(o) = d.bovr(parent) {
                    if o.tail_last != u32::MAX {
                        return Some(o.tail_last);
                    }
                    return o.adopted.last().copied();
                }
                None
            })
        }
    }
}

pub(crate) fn view_attr_with<R>(node: u32, name: &str, f: impl Fn(&str) -> R) -> Option<R> {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| rt.dom.attr_value(i, name).map(f)),
        NodeRef::Base(node) => {
            let over: Option<Option<R>> = with_rt(|rt| {
                let d = &rt.dom;
                let nid = MutDom::attr_nid(name);
                let head = d.bovr(node).map(|o| o.attr_head).unwrap_or(u32::MAX);
                match d.attr_find_at(head, nid, name) {
                    Some(idx) => {
                        let e = &d.attrs[idx as usize];
                        if e.tomb {
                            Some(None)
                        } else {
                            Some(Some(f(e.value.as_str())))
                        }
                    }
                    None => None,
                }
            });
            if let Some(v) = over {
                return v;
            }
            with_doc(|d| {
                d.and_then(|p| {
                    let id = parser_pipeline::ATTR_NAMES.get(name).copied()?;
                    p.dom.attr(node, id).map(f)
                })
            })
        }
    }
}

pub(crate) fn view_attr_eq(node: u32, name: &str, expect: &str) -> bool {
    view_attr_with(node, name, |v| v == expect).is_some_and(|hit| hit)
}

pub(crate) fn view_attr_is_some(node: u32, name: &str) -> bool {
    view_attr_with(node, name, |_| true).is_some()
}

pub(crate) fn view_tag_name_ci(node: u32, tag: &str) -> bool {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| {
            let d = &rt.dom;
            if d.kind_at(i) != KIND_ELEMENT {
                return false;
            }
            let t = d.tag_at(i);
            if t != parser_pipeline::dom::TAG_UNKNOWN {
                return parser_pipeline::dom::static_tag_name(t)
                    .is_some_and(|n| n.eq_ignore_ascii_case(tag));
            }
            d.str_at(i).is_some_and(|n| n.eq_ignore_ascii_case(tag))
        }),
        NodeRef::Base(node) => with_doc(|d| {
            d.and_then(|p| {
                let t = p.dom.tag_id(node);
                if t == parser_pipeline::dom::TAG_UNKNOWN {
                    return None;
                }
                p.dom.tag_name(t)
            })
            .is_some_and(|name| name.eq_ignore_ascii_case(tag))
        }),
    }
}

pub(crate) fn view_attr_contains_word(node: u32, name: &str, word: &str) -> bool {
    view_attr_with(node, name, |v| v.as_bytes().contains_word(word.as_bytes()))
        .is_some_and(|hit| hit)
}

pub(crate) fn view_attr(node: u32, name: &str) -> Option<CompactString> {
    view_attr_with(node, name, |v| CompactString::new(v))
}

pub(crate) fn view_attr_f32(node: u32, name: &str) -> Option<f32> {
    view_attr_with(node, name, parse_px)
}

pub(crate) fn parse_px(v: &str) -> f32 {
    v.trim()
        .trim_end_matches("px")
        .parse::<f32>()
        .unwrap_or(0.0)
}

const STATIC_TAG_LIMIT: u16 = 200;

static TAG_UPPER: std::sync::LazyLock<Vec<Option<String>>> =
    std::sync::LazyLock::new(|| {
        let mut v: Vec<Option<String>> = Vec::new();
        for (name, id) in parser_pipeline::dom::TAGS.entries() {
            let i = *id as usize;
            if i >= v.len() {
                v.resize(i + 1, None);
            }
            v[i] = Some(core_utils::ascii_upper_compact(name).to_string());
        }
        v
    });

pub(crate) fn view_tag_name(node: u32) -> Option<CompactString> {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| {
            let d = &rt.dom;
            match d.kind_at(i) {
                KIND_TEXT => Some(CompactString::const_new("#TEXT")),
                KIND_FRAGMENT => Some(CompactString::const_new("#DOCUMENT-FRAGMENT")),
                KIND_EVENT_TARGET => Some(CompactString::const_new("#EVENTTARGET")),
                _ => {
                    let t = d.tag_at(i);
                    if t != parser_pipeline::dom::TAG_UNKNOWN {
                        if t < STATIC_TAG_LIMIT
                            && let Some(up) = TAG_UPPER
                                .get(t as usize)
                                .and_then(|s| s.as_deref())
                        {
                            return Some(CompactString::const_new(up));
                        }
                        return parser_pipeline::dom::static_tag_name(t)
                            .map(core_utils::ascii_upper_compact);
                    }
                    d.str_at(i)
                        .map(|n| core_utils::ascii_upper_compact(n.as_str()))
                }
            }
        }),
        NodeRef::Base(node) => with_doc(|d| {
            d.and_then(|p| {
                let tag = p.dom.tag_id(node);
                if tag == parser_pipeline::dom::TAG_UNKNOWN {
                    return None;
                }
                if tag < STATIC_TAG_LIMIT
                    && let Some(up) = TAG_UPPER.get(tag as usize).and_then(|s| s.as_deref())
                {
                    return Some(CompactString::const_new(up));
                }
                p.dom.tag_name(tag).map(core_utils::ascii_upper_compact)
            })
        }),
    }
}

pub(crate) fn view_node_type(node: u32) -> u8 {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| match rt.dom.kind_at(i) {
            KIND_TEXT => 3,
            KIND_FRAGMENT => 11,
            KIND_EVENT_TARGET => 0,
            _ => 1,
        }),
        NodeRef::Base(node) => with_doc(|d| {
            d.map(|p| {
                let f = p.dom.flags(node);
                if f & parser_pipeline::dom::node_flags::TEXT != 0 {
                    3
                } else if f & parser_pipeline::dom::node_flags::ELEMENT != 0 {
                    1
                } else {
                    0
                }
            })
            .unwrap_or(0)
        }),
    }
}

pub(crate) fn view_text(node: u32) -> Option<CompactString> {
    match node_ref(node) {
        NodeRef::Overlay(i) => Some(with_rt(|rt| {
            let d = &rt.dom;
            if d.kind_at(i) == KIND_TEXT {
                d.str_at(i).cloned().unwrap_or_default()
            } else {
                CompactString::const_new("")
            }
        })),
        NodeRef::Base(node) => {
            with_doc(|d| d.and_then(|p| p.dom.text(node).map(CompactString::new)))
        }
    }
}

pub(crate) fn view_tag_id(node: u32) -> u16 {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| rt.dom.tag_at(i)),
        NodeRef::Base(node) => with_doc(|d| {
            d.map(|p| p.dom.tag_id(node))
                .unwrap_or(parser_pipeline::dom::TAG_UNKNOWN)
        }),
    }
}

pub(crate) fn view_flags(node: u32) -> u8 {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| match rt.dom.kind_at(i) {
            KIND_TEXT => parser_pipeline::dom::node_flags::TEXT,
            _ => parser_pipeline::dom::node_flags::ELEMENT,
        }),
        NodeRef::Base(node) => with_doc(|d| d.map(|p| p.dom.flags(node)).unwrap_or(0)),
    }
}

fn index_remove(
    map: &mut HashMap<CompactString, SmallVec<[u32; 2]>, FxBuild>,
    key: &str,
    node: u32,
) {
    if let Some(list) = map.get_mut(key) {
        list.retain(|n| *n != node);
        if list.is_empty() {
            map.remove(key);
        }
    }
}

fn id_index_set(
    map: &mut HashMap<CompactString, SmallVec<[u32; 2]>, FxBuild>,
    node: u32,
    old: Option<&str>,
    value: &str,
    add: bool,
) {
    if let Some(o) = old {
        index_remove(map, o, node);
    }
    if add {
        map.entry(CompactString::new(value)).or_default().push(node);
    }
}

pub(crate) fn set_attr(node: u32, name: &str, value: &str) {
    with_rt(|rt| {
        let d = &mut rt.dom;
        let old: Option<CompactString>;
        match node_ref(node) {
            NodeRef::Overlay(i) => {
                old = d.attr_set(i, name, value);
                if name == "id" {
                    let add = matches!(d.kind_at(i), KIND_ELEMENT | KIND_EVENT_TARGET);
                    id_index_set(&mut d.id_index, node, old.as_deref(), value, add);
                }
            }
            NodeRef::Base(node) => {
                old = match d.base_attr_set(node, name, value) {
                    Some(v) => Some(v),
                    None => with_doc(|doc| {
                        doc.and_then(|p| {
                            let id = parser_pipeline::ATTR_NAMES.get(name).copied()?;
                            p.dom.attr(node, id).map(CompactString::new)
                        })
                    }),
                };
                if name == "id" {
                    id_index_set(&mut d.base_id_index, node, old.as_deref(), value, true);
                }
            }
        }
        d.note_attr(node, name, old.as_deref());
    });
}

pub(crate) fn remove_attr(node: u32, name: &str) {
    with_rt(|rt| {
        let d = &mut rt.dom;
        match node_ref(node) {
            NodeRef::Overlay(i) => {
                if name == "id" {
                    let v = d.attr_value(i, "id").map(CompactString::new);
                    if let Some(v) = v {
                        index_remove(&mut d.id_index, v.as_str(), node);
                    }
                }
                d.attr_remove(i, name);
            }
            NodeRef::Base(node) => {
                let old_ovr = d.base_attr_remove(node, name);
                if name == "id" {
                    if let Some(v) = old_ovr {
                        index_remove(&mut d.base_id_index, v.as_str(), node);
                    }
                }
            }
        }
        d.note_attr(node, name, None);
    });
}

pub(crate) fn view_attrs_list(node: u32) -> SmallVec<[(CompactString, CompactString); 4]> {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| rt.dom.attr_list(i)),
        NodeRef::Base(node) => with_rt(|rt| {
            let d = &rt.dom;
            let head = d.bovr(node).map(|o| o.attr_head).unwrap_or(u32::MAX);
            let base: SmallVec<[(CompactString, CompactString); 4]> = with_doc(|doc| {
                doc.map(|p| {
                    p.dom
                        .attrs_of(node)
                        .map(|(k, v)| (CompactString::new(k), CompactString::new(v)))
                        .collect()
                })
                .unwrap_or_default()
            });
            let mut out: SmallVec<[(CompactString, CompactString); 4]> = SmallVec::new();
            for (k, v) in base {
                let nid = MutDom::attr_nid(k.as_str());
                match d.attr_find_at(head, nid, k.as_str()) {
                    Some(idx) => {
                        let e = &d.attrs[idx as usize];
                        if !e.tomb {
                            out.push((k, e.value.clone()));
                        }
                    }
                    None => out.push((k, v)),
                }
            }
            let mut cur = head;
            while cur != u32::MAX {
                let e = &d.attrs[cur as usize];
                if !e.tomb {
                    let nm = d.attr_name(e);
                    if !out.iter().any(|(ok, _)| ok.as_str() == nm) {
                        out.push((CompactString::new(nm), e.value.clone()));
                    }
                }
                cur = e.next;
            }
            out
        }),
    }
}

fn clear_children(d: &mut MutDom, removed: &[u32]) -> (u32, u32) {
    let mut rp = u32::MAX;
    let mut rn = u32::MAX;
    if let (Some(&first), Some(&last)) = (removed.first(), removed.last())
        && MutDom::is_overlay(first)
        && MutDom::is_overlay(last)
    {
        rp = d.slots[MutDom::si(first)].prev;
        rn = d.slots[MutDom::si(last)].next;
    }
    for &r in removed.iter() {
        if MutDom::is_overlay(r) {
            d.unlink(r);
        }
    }
    (rp, rn)
}

fn set_text_content(node: u32, text: &str) {
    let removed = view_children(node);
    with_rt(|rt| {
        let d = &mut rt.dom;
        if let NodeRef::Overlay(i) = node_ref(node)
            && d.kind_at(i) == KIND_TEXT
        {
            let old = d.str_at(i).cloned().unwrap_or_default();
            d.note_character(node, old.as_str());
            if d.slots[i].str_idx == u32::MAX {
                d.strings.push(CompactString::new(text));
                d.slots[i].str_idx = (d.strings.len() - 1) as u32;
            } else {
                let si = d.slots[i].str_idx as usize;
                let s = &mut d.strings[si];
                s.clear();
                s.push_str(text);
            }
            return;
        }
        let (rp, rn) = clear_children(d, &removed);
        if !text.is_empty() {
            let child = d.alloc(
                KIND_TEXT,
                InterfaceId::Text,
                parser_pipeline::dom::TAG_UNKNOWN,
                Some(CompactString::new(text)),
            );
            d.link(node, child);
            d.note_child_list(node, &[child], &removed, rp, rn);
            return;
        }
        d.note_child_list(node, &[], &removed, rp, rn);
    });
}

fn push_children_rev(node: u32, stack: &mut SmallVec<[u32; 32]>) {
    match node_ref(node) {
        NodeRef::Overlay(i) => with_rt(|rt| {
            let d = &rt.dom;
            let mut cur = d.slots[i].last;
            while cur != u32::MAX {
                stack.push(cur);
                cur = d.slots[MutDom::si(cur)].prev;
            }
        }),
        NodeRef::Base(node) => with_rt(|rt| {
            let d = &rt.dom;
            with_doc(|doc| {
                if let Some(p) = doc {
                    let mut cur = p.dom.last_child(node);
                    while cur != u32::MAX {
                        if !d.is_gone(cur) {
                            stack.push(cur);
                        }
                        cur = p.dom.prev_sibling(cur);
                    }
                }
            });
            if let Some(o) = d.bovr(node) {
                if o.tail_last != u32::MAX {
                    let mut cur = o.tail_last;
                    while cur != u32::MAX {
                        stack.push(cur);
                        cur = d.slots[MutDom::si(cur)].prev;
                    }
                }
                for &a in o.adopted.iter().rev() {
                    stack.push(a);
                }
            }
        }),
    }
}

pub(crate) fn text_content_of(node: u32) -> CompactString {
    let mut out = CompactString::new("");
    with_rt(|rt| {
        with_doc(|doc| {
            let d = &rt.dom;
            let mut stack: SmallVec<[u32; 32]> = smallvec![node];
            let mut kids: SmallVec<[u32; 16]> = SmallVec::new();
            while let Some(n) = stack.pop() {
                let is_text = match node_ref(n) {
                    NodeRef::Overlay(i) => {
                        if d.kind_at(i) == KIND_TEXT {
                            if let Some(t) = d.str_at(i) {
                                out.push_str(t.as_str());
                            }
                            true
                        } else {
                            false
                        }
                    }
                    NodeRef::Base(b) => doc.is_some_and(|p| {
                        p.dom.text(b).is_some_and(|t| {
                            out.push_str(t);
                            true
                        })
                    }),
                };
                if !is_text {
                    kids.clear();
                    children_into_d(d, doc, n, &mut kids);
                    stack.extend(kids.iter().rev().copied());
                }
            }
        })
    });
    out
}

fn attrs_list_raw(
    d: &MutDom,
    doc: Option<&parser_pipeline::PageData>,
    node: u32,
    out: &mut SmallVec<[(CompactString, CompactString); 4]>,
) {
    match node_ref(node) {
        NodeRef::Overlay(i) => out.extend(d.attr_list(i)),
        NodeRef::Base(node) => {
            let head = d.bovr(node).map(|o| o.attr_head).unwrap_or(u32::MAX);
            if let Some(p) = doc {
                for (k, v) in p.dom.attrs_of(node) {
                    let nid = MutDom::attr_nid(k);
                    match d.attr_find_at(head, nid, k) {
                        Some(idx) => {
                            let e = &d.attrs[idx as usize];
                            if !e.tomb {
                                out.push((CompactString::new(k), e.value.clone()));
                            }
                        }
                        None => out.push((CompactString::new(k), CompactString::new(v))),
                    }
                }
            }
            let mut cur = head;
            while cur != u32::MAX {
                let e = &d.attrs[cur as usize];
                if !e.tomb {
                    let nm = d.attr_name(e);
                    if !out.iter().any(|(ok, _)| ok.as_str() == nm) {
                        out.push((CompactString::new(nm), e.value.clone()));
                    }
                }
                cur = e.next;
            }
        }
    }
}

fn tag_name_upper_raw(d: &MutDom, doc: Option<&parser_pipeline::PageData>, node: u32) -> CompactString {
    let t = match node_ref(node) {
        NodeRef::Overlay(i) => d.tag_at(i),
        NodeRef::Base(node) => doc
            .map(|p| p.dom.tag_id(node))
            .unwrap_or(parser_pipeline::dom::TAG_UNKNOWN),
    };
    if t == parser_pipeline::dom::TAG_UNKNOWN {
        return match node_ref(node) {
            NodeRef::Overlay(i) => d
                .str_at(i)
                .map(|n| core_utils::ascii_upper_compact(n.as_str()))
                .unwrap_or_default(),
            NodeRef::Base(_) => CompactString::new(""),
        };
    }
    if t < STATIC_TAG_LIMIT
        && let Some(up) = TAG_UPPER.get(t as usize).and_then(|s| s.as_deref())
    {
        return CompactString::const_new(up);
    }
    let lower = match node_ref(node) {
        NodeRef::Overlay(_) => parser_pipeline::dom::static_tag_name(t),
        NodeRef::Base(node) => doc
            .map(|p| p.dom.tag_id(node))
            .or(Some(t))
            .and_then(parser_pipeline::dom::static_tag_name),
    };
    lower
        .map(core_utils::ascii_upper_compact)
        .unwrap_or_default()
}

fn inner_html_into_raw(
    d: &MutDom,
    doc: Option<&parser_pipeline::PageData>,
    node: u32,
    out: &mut CompactString,
) {
    let mut kids: SmallVec<[u32; 16]> = SmallVec::new();
    children_into_d(d, doc, node, &mut kids);
    for c in kids {
        if node_type_of_d(d, doc, c) == 3 {
            match node_ref(c) {
                NodeRef::Overlay(i) => {
                    if let Some(t) = d.str_at(i) {
                        out.push_str(t.as_str());
                    }
                }
                NodeRef::Base(b) => {
                    if let Some(p) = doc
                        && let Some(t) = p.dom.text(b)
                    {
                        out.push_str(t);
                    }
                }
            }
            continue;
        }
        let tag = tag_name_upper_raw(d, doc, c);
        out.push('<');
        core_utils::push_ascii_case_into(out, tag.as_str(), false);
        let mut attrs: SmallVec<[(CompactString, CompactString); 4]> = SmallVec::new();
        attrs_list_raw(d, doc, c, &mut attrs);
        for (k, v) in attrs {
            out.push(' ');
            out.push_str(k.as_str());
            out.push_str("=\"");
            out.push_str(v.as_str());
            out.push('"');
        }
        out.push('>');
        inner_html_into_raw(d, doc, c, out);
        out.push_str("</");
        out.push_str(tag.as_str());
        out.push('>');
    }
}

fn inner_html_into(node: u32, out: &mut CompactString) {
    with_rt(|rt| {
        with_doc(|doc| inner_html_into_raw(&rt.dom, doc, node, out))
    });
}

fn inner_html_of(node: u32) -> CompactString {
    let mut out = CompactString::new("");
    inner_html_into(node, &mut out);
    out
}
fn alloc_text_raw(d: &mut MutDom, text: &str) -> u32 {
    d.alloc(
        KIND_TEXT,
        InterfaceId::Text,
        parser_pipeline::dom::TAG_UNKNOWN,
        Some(CompactString::new(text)),
    )
}

fn alloc_text_under_raw(d: &mut MutDom, parent: u32, text: &str) -> u32 {
    let id = alloc_text_raw(d, text);
    d.link(parent, id);
    d.note_child_list(parent, &[id], &[], u32::MAX, u32::MAX);
    id
}

fn alloc_element_with_id_raw(
    d: &mut MutDom,
    tag_lower: &str,
    iface: InterfaceId,
    tag_id: u16,
) -> u32 {
    let name =
        (tag_id == parser_pipeline::dom::TAG_UNKNOWN).then(|| CompactString::new(tag_lower));
    d.alloc(KIND_ELEMENT, iface, tag_id, name)
}

fn alloc_element_with_id(tag_lower: &str, iface: InterfaceId, tag_id: u16) -> u32 {
    with_rt(|rt| alloc_element_with_id_raw(&mut rt.dom, tag_lower, iface, tag_id))
}

fn alloc_element(tag_lower: &str, iface: InterfaceId) -> u32 {
    let tag_id = parser_pipeline::dom::TAGS
        .get(tag_lower)
        .copied()
        .unwrap_or(parser_pipeline::dom::TAG_UNKNOWN);
    alloc_element_with_id(tag_lower, iface, tag_id)
}

fn alloc_element_under_raw(
    d: &mut MutDom,
    parent: u32,
    tag: &str,
    attrs: &[(CompactString, CompactString)],
) -> u32 {
    let (iface, tag_id) = iface_for_tag_name(tag);
    let el = alloc_element_with_id_raw(d, tag, iface, tag_id);
    let i = MutDom::si(el);
    for (k, v) in attrs {
        if k.as_str() == "id" {
            d.id_index.entry(v.clone()).or_default().push(el);
        }
        d.attr_set(i, k.as_str(), v.as_str());
    }
    d.link(parent, el);
    d.note_child_list(parent, &[el], &[], u32::MAX, u32::MAX);
    el
}

fn parse_fragment(node: u32, html: &str) {
    let removed = view_children(node);
    with_rt(|rt| {
        let d = &mut rt.dom;
        let (rp, rn) = clear_children(d, &removed);
        d.note_child_list(node, &[], &removed, rp, rn);
        let mut i = 0usize;
        let mut text_start = 0usize;
        let mut parent_stack: SmallVec<[u32; 16]> = smallvec![node];
        let b_html = html.as_bytes();
        while i < html.len() {
            if b_html[i] == b'<' && html.is_char_boundary(i) {
                if i > text_start && html.is_char_boundary(text_start) {
                    let txt = &html[text_start..i];
                    if !txt.is_empty() {
                        let parent = *parent_stack.last().unwrap_or(&node);
                        alloc_text_under_raw(d, parent, txt);
                    }
                }
                let close = html[i..].find('>').map(|c| i + c);
                let Some(close) = close else { break };
                let inner = &html[i + 1..close];
                if inner.starts_with('/') {
                    if parent_stack.len() > 1 {
                        parent_stack.pop();
                    }
                } else {
                    let tag_end = inner.find([' ', '/']).unwrap_or(inner.len());
                    let tag = &inner[..tag_end];
                    if !tag.is_empty()
                        && tag
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                    {
                        let attrs = parse_attrs(&inner[tag_end.min(inner.len())..]);
                        let parent = *parent_stack.last().unwrap_or(&node);
                        let el = alloc_element_under_raw(d, parent, tag, &attrs);
                        let self_closing = inner.ends_with('/')
                            || parser_pipeline::dom::is_void_tag(
                                parser_pipeline::dom::TAGS
                                    .get(tag)
                                    .copied()
                                    .unwrap_or(parser_pipeline::dom::TAG_UNKNOWN),
                            );
                        if !self_closing {
                            parent_stack.push(el);
                        }
                    }
                }
                i = close + 1;
                text_start = i;
            } else {
                i += 1;
            }
        }
        if text_start < html.len() && html.is_char_boundary(text_start) {
            let txt = &html[text_start..];
            let parent = *parent_stack.last().unwrap_or(&node);
            alloc_text_under_raw(d, parent, txt);
        }
    });
}

fn parse_attrs(s: &str) -> SmallVec<[(CompactString, CompactString); 4]> {
    let mut out: SmallVec<[(CompactString, CompactString); 4]> = SmallVec::new();
    let b = s.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        while i < b.len() && (b[i] == b' ' || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i] != b'=' && b[i] != b' ' && b[i] != b'>' {
            i += 1;
        }
        if start == i {
            break;
        }
        let name = &s[start..i];
        if i < b.len() && b[i] == b'=' {
            i += 1;
            let value: &str;
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                i += 1;
                let vstart = i;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                value = &s[vstart..i];
                if i < b.len() {
                    i += 1;
                }
            } else {
                let vstart = i;
                while i < b.len() && b[i] != b' ' {
                    i += 1;
                }
                value = &s[vstart..i];
            }
            out.push((CompactString::new(name), CompactString::new(value)));
        } else {
            out.push((CompactString::new(name), CompactString::const_new("")));
        }
    }
    out
}

pub(crate) fn observe_reg<'js>(
    ctx: &Ctx<'js>,
    obs: u64,
    cb: Value<'js>,
    target: u32,
    opts: ObsOptions,
) {
    with_rt(|rt| {
        let d = &mut rt.dom;
        d.observers
            .retain(|r| !(r.obs == obs && r.target == target));
        d.observers.push(ObsReg {
            obs,
            cb: Persistent::save(ctx, cb),
            target,
            opts,
            queue: SmallVec::new(),
        });
    });
}

pub(crate) fn disconnect_obs(obs: u64) {
    with_rt(|rt| {
        let d = &mut rt.dom;
        d.observers.retain(|r| r.obs != obs);
        if d.observers.iter().all(|r| r.queue.is_empty()) {
            d.recs.clear();
            d.rec_ids.clear();
        }
    });
}

fn push_materialized(d: &MutDom, recs: &mut Vec<MutRec>, ids: &mut Vec<u32>, r: MutRec) {
    let a = ids.len() as u32;
    ids.extend_from_slice(d.ids_of(&r));
    let al = ids.len() as u32 - a;
    let rv = ids.len() as u32;
    ids.extend_from_slice(d.removed_ids_of(&r));
    let rl = ids.len() as u32 - rv;
    recs.push(MutRec {
        added: a,
        added_len: al,
        removed: rv,
        removed_len: rl,
        ..r
    });
}

pub(crate) fn take_records_for(obs: u64) -> Option<(Vec<MutRec>, Vec<u32>, bool, bool)> {
    let mut recs_out: Vec<MutRec> = Vec::new();
    let mut ids_out: Vec<u32> = Vec::new();
    let flags = with_rt(|rt| {
        let d = &mut rt.dom;
        let Some(reg) = d.observers.iter_mut().find(|r| r.obs == obs) else {
            return None;
        };
        let attr_old = reg.opts.attr_old();
        let char_old = reg.opts.char_old();
        let queue = std::mem::take(&mut reg.queue);
        for idx in queue.iter().copied() {
            let r = std::mem::replace(
                &mut d.recs[idx as usize],
                MutRec::empty(),
            );
            push_materialized(d, &mut recs_out, &mut ids_out, r);
        }
        Some((attr_old, char_old))
    })?;
    with_rt(|rt| {
        let d = &mut rt.dom;
        if d.observers.iter().all(|r| r.queue.is_empty()) {
            d.recs.clear();
            d.rec_ids.clear();
        }
    });
    Some((recs_out, ids_out, flags.0, flags.1))
}

pub(crate) fn flush_mutations(ctx: &Ctx<'_>) {
    type ObsBatch = (
        u64,
        Persistent<Value<'static>>,
        bool,
        bool,
        Vec<MutRec>,
        Vec<u32>,
    );
    let batch: Vec<ObsBatch> = with_rt(|rt| {
        let d = &mut rt.dom;
        let mut materialized: Vec<(usize, ObsBatch)> = Vec::new();
        let queues: Vec<(usize, SmallVec<[u32; 8]>, u64, bool, bool)> = d
            .observers
            .iter_mut()
            .enumerate()
            .filter(|(_, reg)| !reg.queue.is_empty())
            .map(|(i, reg)| {
                let queue = std::mem::take(&mut reg.queue);
                (i, queue, reg.obs, reg.opts.attr_old(), reg.opts.char_old())
            })
            .collect();
        for (i, queue, obs, attr_old, char_old) in queues {
            let cb = d.observers[i].cb.clone();
            let mut recs: Vec<MutRec> = Vec::with_capacity(queue.len());
            let mut ids: Vec<u32> = Vec::new();
            for idx in queue.iter().copied() {
                let r = std::mem::replace(
                    &mut d.recs[idx as usize],
                    MutRec::empty(),
                );
                push_materialized(d, &mut recs, &mut ids, r);
            }
            materialized.push((i, (obs, cb, attr_old, char_old, recs, ids)));
        }
        materialized.into_iter().map(|(_, b)| b).collect()
    });
    if batch.is_empty() {
        return;
    }
    let tag = symbol_tostring_tag(ctx).ok();
    let empty = empty_array_value(ctx).ok();
    for (_, cb, attr_old, char_old, records, ids) in batch {
        let _ = call_cb_with_array(ctx, &cb, |arr| {
            for (i, r) in records.iter().enumerate() {
                if let Ok(rec) = mutation_record_value(
                    ctx,
                    r,
                    &ids,
                    empty.as_ref(),
                    tag.as_ref(),
                    attr_old,
                    char_old,
                ) {
                    let _ = arr.set(i, rec);
                }
            }
            Ok(())
        });
    }
}

pub(crate) fn mutation_record_value<'js>(
    ctx: &Ctx<'js>,
    r: &MutRec,
    ids: &[u32],
    empty: Option<&Value<'js>>,
    tag: Option<&rquickjs::Symbol<'js>>,
    attr_old: bool,
    char_old: bool,
) -> rquickjs::Result<Value<'js>> {
    let o = Object::new(ctx.clone())?;
    if let Some(tag) = tag {
        let _ = o.prop(
            tag.clone(),
            rquickjs::object::Property::from("MutationRecord").configurable(),
        );
    }
    let added = &ids[r.added as usize..(r.added + r.added_len) as usize];
    let removed = &ids[r.removed as usize..(r.removed + r.removed_len) as usize];
    let _ = o.set("addedNodes", node_array(ctx, added, empty));
    let _ = o.set("removedNodes", node_array(ctx, removed, empty));
    let _ = o.set("previousSibling", node_or_null(ctx, r.prev_sibling)?);
    let _ = o.set("nextSibling", node_or_null(ctx, r.next_sibling)?);
    let _ = o.set("attributeNamespace", Value::new_null(ctx.clone()));
    match r.kind {
        0 => {
            let _ = o.set("type", "childList");
            let _ = o.set("attributeName", Value::new_null(ctx.clone()));
        }
        1 => {
            let _ = o.set("type", "attributes");
            let _ = o.set("attributeName", r.attr_name.as_str());
        }
        _ => {
            let _ = o.set("type", "characterData");
            let _ = o.set("attributeName", Value::new_null(ctx.clone()));
        }
    }
    let old_ok = match r.kind {
        1 => attr_old,
        2 => char_old,
        _ => false,
    } && !r.old_value.is_empty();
    if old_ok {
        let _ = o.set("oldValue", r.old_value.as_str());
    } else {
        let _ = o.set("oldValue", Value::new_null(ctx.clone()));
    }
    let _ = o.set("target", crate::worker::handle_value(ctx, r.target)?);
    Ok(o.into_value())
}

fn handle_array<'js>(ctx: &Ctx<'js>, ids: &[u32]) -> rquickjs::Result<Value<'js>> {
    let arr = rquickjs::Array::new(ctx.clone())?;
    for (i, &k) in ids.iter().enumerate() {
        arr.set(i, crate::worker::handle_value(ctx, k)?)?;
    }
    Ok(arr.into_value())
}

fn handle_array_lenient<'js>(ctx: &Ctx<'js>, ids: &[u32]) -> rquickjs::Result<Value<'js>> {
    let arr = rquickjs::Array::new(ctx.clone())?;
    for (i, &k) in ids.iter().enumerate() {
        if let Ok(v) = crate::worker::handle_value(ctx, k) {
            arr.set(i, v)?;
        }
    }
    Ok(arr.into_value())
}

fn node_or_null<'js>(ctx: &Ctx<'js>, id: u32) -> rquickjs::Result<Value<'js>> {
    if id == u32::MAX {
        Ok(Value::new_null(ctx.clone()))
    } else {
        crate::worker::handle_value(ctx, id)
    }
}

pub(crate) fn opt_node_or_null<'js>(
    ctx: &Ctx<'js>,
    id: Option<u32>,
) -> rquickjs::Result<Value<'js>> {
    node_or_null(ctx, id.unwrap_or(u32::MAX))
}

fn node_array<'js>(ctx: &Ctx<'js>, ids: &[u32], empty: Option<&Value<'js>>) -> Value<'js> {
    if ids.is_empty()
        && let Some(e) = empty
    {
        return e.clone();
    }
    match handle_array_lenient(ctx, ids) {
        Ok(v) => v,
        Err(_) => Value::new_null(ctx.clone()),
    }
}

pub(crate) fn added_nodes_value<'js>(ctx: &Ctx<'js>, ids: &[u32]) -> rquickjs::Result<Value<'js>> {
    handle_array_lenient(ctx, ids)
}

fn is_in_subtree(node: u32, root: u32) -> bool {
    with_rt(|rt| rt.dom.is_in_subtree(node, root))
}

pub(crate) fn dom_rect<'js>(
    ctx: &Ctx<'js>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    tag: &'static str,
) -> rquickjs::Result<Object<'js>> {
    let o = Object::new(ctx.clone())?;
    class_tag(ctx, &o, tag)?;
    o.set("x", x)?;
    o.set("y", y)?;
    o.set("width", w)?;
    o.set("height", h)?;
    o.set("top", y)?;
    o.set("bottom", y + h)?;
    o.set("left", x)?;
    o.set("right", x + w)?;
    Ok(o)
}

pub(crate) fn with_reg<S, R>(
    reg: &'static std::thread::LocalKey<RegCell<S>>,
    id: u64,
    f: impl FnOnce(&mut S) -> R,
) -> Option<R> {
    reg.with(|m| m.borrow_mut().with(id, f))
}

pub(crate) fn throw_illegal<T>(c: &Ctx<'_>) -> rquickjs::Result<T> {
    Err(rquickjs::Exception::throw_message(
        c,
        "TypeError: Illegal invocation",
    ))
}

macro_rules! node_get {
    ($ctx:expr, $proto:expr, $key:literal, |$c:ident, $n:ident| $body:expr) => {
        named_accessor(
            $ctx,
            $proto,
            $key,
            Function::new(
                $ctx.clone(),
                |$c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                    let Some($n) = this_node(&this.0) else {
                        return throw_illegal(&$c);
                    };
                    $body
                },
            )?,
            None,
        )?
    };
}

macro_rules! node_getset {
    ($ctx:expr, $proto:expr, $key:literal, |$c:ident, $n:ident| $g:expr, |$n2:ident, $v:ident| $s:expr) => {
        named_accessor(
            $ctx,
            $proto,
            $key,
            Function::new(
                $ctx.clone(),
                |$c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                    let Some($n) = this_node(&this.0) else {
                        return throw_illegal(&$c);
                    };
                    $g
                },
            )?,
            Some(Function::new(
                $ctx.clone(),
                |this: This<Value<'js>>, $v: Value<'js>| -> rquickjs::Result<()> {
                    let Some($n2) = this_node(&this.0) else {
                        return Ok(());
                    };
                    $s
                },
            )?),
        )?
    };
}

macro_rules! node_method {
    ($ctx:expr, $proto:expr, $key:literal, ($ret:ty; $bad:expr), |$c:ident, $n:ident $(, $a:ident : $t:ty)*| $body:expr) => {{
        let f = Function::new(
            $ctx.clone(),
            |$c: Ctx<'js>, this: This<Value<'js>> $(, $a: $t)*| -> rquickjs::Result<$ret> {
                let Some($n) = this_node(&this.0) else {
                    return $bad;
                };
                $body
            },
        )?;
        define_method($ctx, $proto, $key, f)?;
    }};
}

fn install_event_target<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let et = ensure_prototype(ctx, InterfaceId::EventTarget)?;
    let et_real = Function::new(ctx.clone(), |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
        create_event_target(&c)
    })?
    .with_constructor(true);
    install_host_ctor(ctx, &et_real, &et, "EventTarget", 0, true)?;

    node_method!(ctx, &et, "addEventListener", ((); throw_illegal(&c)), |c, n, type_: String, f: Value<'js>, _rest: Rest<Value<'js>>| {
        let Some(fun) = f.as_function() else {
            return Ok(());
        };
        listen_add(&c, n, type_.as_str(), fun.clone())
    });
    node_method!(ctx, &et, "removeEventListener", ((); throw_illegal(&c)), |c, n, type_: String, f: Value<'js>| {
        listen_remove(&c, n, type_.as_str(), &f)
    });
    node_method!(ctx, &et, "dispatchEvent", (bool; Ok(false)), |c, n, ev: Value<'js>| {
        listen_dispatch(&c, n, &ev)
    });
    Ok(())
}

fn install_node_level<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let node_p = ensure_prototype(ctx, InterfaceId::Node)?;
    let globals = ctx.globals();
    let node_ctor: Function = globals.get("Node")?;
    for (name, val) in [
        ("ELEMENT_NODE", 1f64),
        ("ATTRIBUTE_NODE", 2f64),
        ("TEXT_NODE", 3f64),
        ("CDATA_SECTION_NODE", 4f64),
        ("COMMENT_NODE", 8f64),
        ("DOCUMENT_NODE", 9f64),
        ("DOCUMENT_TYPE_NODE", 10f64),
        ("DOCUMENT_FRAGMENT_NODE", 11f64),
    ] {
        node_ctor.prop(name, rquickjs::object::Property::from(val))?;
    }

    node_get!(ctx, &node_p, "childNodes", |c, n| handle_array(
        &c,
        &view_children(n)
    ));

    node_get!(ctx, &node_p, "parentNode", |c, n| match view_parent(n) {
        Some(p) => crate::worker::handle_value(&c, p),
        None => Ok(Value::new_null(c)),
    });

    node_get!(ctx, &node_p, "nodeType", |c, n| {
        Ok(Value::new_number(c, view_node_type(n) as f64))
    });

    node_get!(ctx, &node_p, "nodeName", |c, n| match view_node_type(n) {
        3 => "#text".into_js(&c),
        11 => "#document-fragment".into_js(&c),
        _ => match view_tag_name(n) {
            Some(t) => t.as_str().into_js(&c),
            None => "#node".into_js(&c),
        },
    });

    node_get!(ctx, &node_p, "firstChild", |c, n| {
        opt_node_or_null(&c, view_first(n))
    });

    node_get!(ctx, &node_p, "lastChild", |c, n| {
        opt_node_or_null(&c, view_last(n))
    });

    node_get!(ctx, &node_p, "nextSibling", |c, n| {
        opt_node_or_null(&c, view_next(n))
    });

    node_get!(ctx, &node_p, "previousSibling", |c, n| {
        opt_node_or_null(&c, view_prev(n))
    });

    node_getset!(
        ctx,
        &node_p,
        "textContent",
        |c, n| text_content_of(n).as_str().into_js(&c),
        |n, v| {
            set_text_content(n, to_compact(&v).as_str());
            Ok(())
        }
    );

    node_get!(ctx, &node_p, "ownerDocument", |c, _nd| {
        let doc = with_rt(|rt| rt.doc_global.clone());
        match doc {
            Some(d) => Ok(d.restore(&c)?.into_value()),
            None => Ok(Value::new_null(c)),
        }
    });

    define_method(
        ctx,
        &node_p,
        "appendChild",
        Function::new(ctx.clone(), append_child)?,
    )?;
    define_method(
        ctx,
        &node_p,
        "insertBefore",
        Function::new(ctx.clone(), insert_before)?,
    )?;
    define_method(
        ctx,
        &node_p,
        "removeChild",
        Function::new(ctx.clone(), remove_child)?,
    )?;

    let remove_f = Function::new(
        ctx.clone(),
        |_c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<()> { remove_self(&this.0) },
    )?;
    define_method(ctx, &node_p, "remove", remove_f)?;

    node_method!(ctx, &node_p, "contains", (bool; Ok(false)), |_c, n, other: Value<'js>| {
        let Some(other_n) = node_of_value(&other) else {
            return Ok(false);
        };
        Ok(other_n == n || is_in_subtree(other_n, n))
    });

    node_method!(ctx, &node_p, "hasChildNodes", (bool; Ok(false)), |_c, n| {
        Ok(view_first(n).is_some())
    });
    Ok(())
}

fn attr_accessor<'js>(
    ctx: &Ctx<'js>,
    proto: &Object<'js>,
    key: &'static str,
    attr: &'static str,
) -> rquickjs::Result<()> {
    let g = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let Some(n) = this_node(&this.0) else {
                return throw_illegal(&c);
            };
            match view_attr(n, attr) {
                Some(v) => v.into_js(&c),
                None => Ok(Value::new_null(c)),
            }
        },
    )?;
    let s = Function::new(
        ctx.clone(),
        move |this: This<Value<'js>>, v: Value<'js>| -> rquickjs::Result<()> {
            let Some(n) = this_node(&this.0) else {
                return Ok(());
            };
            set_attr(n, attr, to_compact(&v).as_str());
            Ok(())
        },
    )?;
    named_accessor(ctx, proto, key, g, Some(s))
}

fn install_element_level<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let element_p = ensure_prototype(ctx, InterfaceId::Element)?;
    let html_p = ensure_prototype(ctx, InterfaceId::HTMLElement)?;

    node_get!(ctx, &element_p, "tagName", |c, n| match view_tag_name(n) {
        Some(t) => t.as_str().into_js(&c),
        None => Ok(Value::new_undefined(c)),
    });

    for (key, attr) in [
        ("id", "id"),
        ("className", "class"),
        ("title", "title"),
        ("lang", "lang"),
        ("dir", "dir"),
    ] {
        attr_accessor(ctx, &html_p, key, attr)?;
    }

    node_getset!(
        ctx,
        &html_p,
        "hidden",
        |c, n| Ok(Value::new_bool(c, view_attr(n, "hidden").is_some())),
        |n, v| {
            if v.as_bool().unwrap_or(false) {
                set_attr(n, "hidden", "");
            } else {
                remove_attr(n, "hidden");
            }
            Ok(())
        }
    );

    node_get!(ctx, &element_p, "children", |c, n| {
        let kids: SmallVec<[u32; 16]> = view_children(n)
            .into_iter()
            .filter(|&k| view_node_type(k) == 1)
            .collect();
        handle_array(&c, &kids)
    });

    node_get!(ctx, &element_p, "attributes", |c, n| {
        let list = view_attrs_list(n);
        let arr = rquickjs::Array::new(c.clone())?;
        for (i, (k, v)) in list.iter().enumerate() {
            let o = Object::new(c.clone())?;
            o.set("name", k.as_str())?;
            o.set("value", v.as_str())?;
            arr.set(i, o)?;
        }
        Ok(arr.into_value())
    });

    node_method!(ctx, &element_p, "getAttribute", (Value<'js>; throw_illegal(&c)), |c, n, name: String| {
        match view_attr(n, name.as_str()) {
            Some(v) => v.into_js(&c),
            None => Ok(Value::new_null(c)),
        }
    });

    node_method!(ctx, &element_p, "setAttribute", ((); Ok(())), |_c, n, name: rquickjs::String<'js>, value: Value<'js>| {
        let nc = name.to_cstring()?;
        set_attr(n, nc.as_str(), to_compact(&value).as_str());
        Ok(())
    });

    node_method!(ctx, &element_p, "hasAttribute", (bool; Ok(false)), |_c, n, name: String| {
        Ok(view_attr(n, name.as_str()).is_some())
    });

    node_method!(ctx, &element_p, "removeAttribute", ((); Ok(())), |_c, n, name: String| {
        remove_attr(n, name.as_str());
        Ok(())
    });

    node_method!(ctx, &element_p, "querySelector", (Value<'js>; Ok(Value::new_null(c))), |c, n, q: String| {
        match crate::query::select_all_under_view(n, q.as_str()) {
            Some(list) => match list.first() {
                Some(&x) => crate::worker::handle_value(&c, x),
                None => Ok(Value::new_null(c)),
            },
            None => Ok(Value::new_null(c)),
        }
    });

    node_method!(ctx, &element_p, "querySelectorAll", (Value<'js>; Ok(rquickjs::Array::new(c.clone())?.into_value())), |c, n, q: String| {
        let arr = rquickjs::Array::new(c.clone())?;
        if let Some(list) = crate::query::select_all_under_view(n, q.as_str()) {
            for x in list.iter() {
                let v = crate::worker::handle_value(&c, *x)?;
                arr.set(arr.len(), v)?;
            }
        }
        Ok(arr.into_value())
    });

    let gcr = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::GET_BOUNDING_CLIENT_RECT);
            let Some(n) = this_node(&this.0) else {
                return throw_illegal(&c);
            };
            let b = crate::layout::rect_for(n);
            Ok(crate::webidl::dom_rect(&c, b.x, b.y, b.w, b.h, "DOMRect")?.into_value())
        },
    )?;
    define_method(ctx, &element_p, "getBoundingClientRect", gcr)?;

    for (key, dim) in [
        ("offsetWidth", 0u8),
        ("offsetHeight", 1u8),
        ("clientWidth", 0u8),
        ("clientHeight", 1u8),
    ] {
        let g = Function::new(
            ctx.clone(),
            move |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                touch::touch_log_record(ApiKey::GET_BOUNDING_CLIENT_RECT);
                let Some(n) = this_node(&this.0) else {
                    return throw_illegal(&c);
                };
                let b = crate::layout::rect_for(n);
                let v = if dim == 0 { b.w } else { b.h };
                Ok(Value::new_number(c, v))
            },
        )?;
        named_accessor(ctx, &element_p, key, g, None)?;
    }

    node_method!(ctx, &html_p, "click", ((); Ok(())), |c, n| {
        let ev = Object::new(c.clone())?;
        ev.set("type", "click")?;
        ev.set("isTrusted", false)?;
        let _: rquickjs::Result<bool> = listen_dispatch(&c, n, &ev.into_value());
        Ok(())
    });

    for name in ["focus", "blur"] {
        let f = Function::new(ctx.clone(), |_c: Ctx<'js>, _this: This<Value<'js>>| {})?;
        define_method(ctx, &html_p, name, f)?;
    }

    node_get!(ctx, &html_p, "style", |c, n| style_of(&c, n));

    node_getset!(
        ctx,
        &element_p,
        "innerHTML",
        |c, n| inner_html_of(n).as_str().into_js(&c),
        |n, v| {
            parse_fragment(n, to_compact(&v).as_str());
            Ok(())
        }
    );

    node_getset!(
        ctx,
        &html_p,
        "innerText",
        |c, n| text_content_of(n).as_str().into_js(&c),
        |n, v| {
            set_text_content(n, to_compact(&v).as_str());
            Ok(())
        }
    );
    Ok(())
}

fn install_document_level<'js>(
    ctx: &Ctx<'js>,
    doc: &Object<'js>,
    host_doc_proto: &Object<'js>,
) -> rquickjs::Result<()> {
    let node_p = ensure_prototype(ctx, InterfaceId::Node)?;
    host_doc_proto.set_prototype(Some(&node_p))?;
    let html_doc_p = Object::new(ctx.clone())?;
    html_doc_p.set_prototype(Some(host_doc_proto))?;
    let ctor = illegal_ctor_fn(ctx, "HTMLDocument")?;
    install_host_ctor(ctx, &ctor, &html_doc_p, "HTMLDocument", 0, true)?;
    doc.set_prototype(Some(&html_doc_p))?;

    macro_rules! doc_create {
        ($key:literal, |$c:ident $(, $a:ident : $t:ty)*| $body:expr) => {{
            let f = Function::new(ctx.clone(), move |$c: Ctx<'js> $(, $a: $t)*| -> rquickjs::Result<Value<'js>> {
                touch::touch_log_record(ApiKey::CREATE_ELEMENT);
                $body
            })?;
            define_method(ctx, host_doc_proto, $key, f)?;
        }};
    }
    doc_create!(
        "createElement",
        |c, tag: String, _rest: Rest<Value<'js>>| create_element(&c, tag.as_str())
    );
    doc_create!("createTextNode", |c, text: String| create_text_node(
        &c,
        text.as_str()
    ));
    doc_create!("createDocumentFragment", |c| create_fragment(&c));

    let create_ev = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, type_: String| -> rquickjs::Result<Value<'js>> {
            let ctor: Option<Function> = c.globals().get("Event").ok().flatten();
            match ctor {
                Some(f) => {
                    let mk = cached_persistent(&c, &MK_EVENT, |c| {
                        c.eval("(function (C, t) { return new C(t); })")
                    })?;
                    mk.call((f, type_))
                }
                None => {
                    let o = Object::new(c.clone())?;
                    let _ = o.set("type", type_);
                    Ok(o.into_value())
                }
            }
        },
    )?;
    define_method(ctx, host_doc_proto, "createEvent", create_ev)?;

    let get_default_view =
        Function::new(ctx.clone(), |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            let w: Object = c.globals();
            Ok(w.into_value())
        })?;
    named_accessor(ctx, host_doc_proto, "defaultView", get_default_view, None)?;

    let get_active = Function::new(ctx.clone(), |c: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
        match crate::query::find_first_by_tag_view("body") {
            Some(n) => crate::worker::handle_value(&c, n),
            None => Ok(Value::new_null(c)),
        }
    })?;
    named_accessor(ctx, host_doc_proto, "activeElement", get_active, None)?;

    let compat = Function::new(ctx.clone(), || -> &'static str { "CSS1Compat" })?;
    define_method(ctx, host_doc_proto, "compatMode", compat)?;
    Ok(())
}

fn install_console<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let console = Object::new(ctx.clone())?;
    let names = [
        "log",
        "warn",
        "info",
        "error",
        "debug",
        "trace",
        "dir",
        "dirxml",
        "table",
        "group",
        "groupCollapsed",
        "groupEnd",
        "clear",
        "count",
        "countReset",
        "assert",
        "profile",
        "profileEnd",
        "time",
        "timeLog",
        "timeEnd",
        "context",
    ];
    for name in names {
        let f = Function::new(ctx.clone(), |_c: Ctx<'js>, _rest: Rest<Value<'js>>| {})?;
        set_fn_name(ctx, &f, name)?;
        console.prop(
            name,
            rquickjs::object::Property::from(f)
                .writable()
                .enumerable()
                .configurable(),
        )?;
    }
    let memory: Value<'js> = Value::new_undefined(ctx.clone());
    define_data(&console, "memory", memory, true, true, true)?;
    let globals = ctx.globals();
    define_data(&globals, "console", console.into_value(), true, true, true)?;
    Ok(())
}

pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    doc: &Object<'js>,
    host_doc_proto: &Object<'js>,
) -> rquickjs::Result<()> {
    install_event_target(ctx)?;
    for &iface in IFACE_ALL {
        if matches!(
            iface,
            InterfaceId::HtmlCanvasElement | InterfaceId::EventTarget
        ) {
            continue;
        }
        ensure_prototype(ctx, iface)?;
    }

    install_node_level(ctx)?;
    install_element_level(ctx)?;
    let html_p = ensure_prototype(ctx, InterfaceId::HTMLElement)?;
    crate::canvas2d::link_webidl(ctx, &html_p)?;
    install_document_level(ctx, doc, host_doc_proto)?;
    install_console(ctx)?;
    let eq: Function = ctx.eval("(function (a, b) { return a === b; })")?;
    with_rt(|rt| {
        rt.eq_thunk = Some(Persistent::save(ctx, eq));
        rt.doc_global = Some(Persistent::save(ctx, doc.clone()));
    });
    Ok(())
}

pub(crate) fn register_canvas_proto<'js>(ctx: &Ctx<'js>, proto: &Object<'js>) {
    with_rt(|rt| {
        let slot = InterfaceId::HtmlCanvasElement as u16 as usize;
        if rt.protos.len() <= slot {
            rt.protos.resize(slot + 1, None);
        }
        rt.protos[slot] = Some(Persistent::save(ctx, proto.clone()));
    });
}

#[inline]
fn overlay_element(d: &MutDom, i: usize) -> bool {
    d.kind_at(i) == KIND_ELEMENT
        && d.iface_at(i) != InterfaceId::EventTarget as u16
        && d.slots[i].parent != u32::MAX
}

#[inline]
fn overlay_hit(d: &MutDom, i: usize, want: Option<u16>, tag: &str) -> bool {
    overlay_element(d, i)
        && match want {
            Some(w) => {
                d.tag_at(i) == w
                    || (d.tag_at(i) == parser_pipeline::dom::TAG_UNKNOWN
                        && d.str_at(i).is_some_and(|n| n.eq_ignore_ascii_case(tag)))
            }
            None => {
                d.tag_at(i) == parser_pipeline::dom::TAG_UNKNOWN
                    && d.str_at(i).is_some_and(|n| n.eq_ignore_ascii_case(tag))
            }
        }
}

fn overlay_scan(mut hit: impl FnMut(&MutDom, usize) -> bool, mut emit: impl FnMut(usize) -> bool) {
    with_rt(|rt| {
        let d = &rt.dom;
        for i in 0..d.slots.len() {
            if hit(d, i) && !emit(i) {
                return;
            }
        }
    });
}

pub(crate) fn overlay_elements_into(tag: &str, out: &mut SmallVec<[u32; 32]>) {
    if out.len() >= 256 {
        return;
    }
    let want = parser_pipeline::dom::TAGS.get(tag).copied();
    overlay_scan(
        |d, i| overlay_hit(d, i, want, tag),
        |i| {
            out.push(OVERLAY_BASE + i as u32);
            out.len() < 256
        },
    );
}

pub(crate) const ELEM_CAP: usize = 4096;

fn children_into_d(
    dom: &MutDom,
    doc: Option<&parser_pipeline::PageData>,
    node: u32,
    out: &mut SmallVec<[u32; 16]>,
) {
    match node_ref(node) {
        NodeRef::Overlay(i) => {
            let mut cur = dom.slots[i].first;
            while cur != u32::MAX {
                out.push(cur);
                cur = dom.slots[MutDom::si(cur)].next;
            }
        }
        NodeRef::Base(node) => {
            if let Some(o) = dom.bovr(node) {
                out.extend(o.adopted.iter().copied());
                let mut cur = o.tail_head;
                while cur != u32::MAX {
                    out.push(cur);
                    cur = dom.slots[MutDom::si(cur)].next;
                }
            }
            if let Some(p) = doc {
                for c in p.dom.children(node) {
                    if !dom.is_gone(c) {
                        out.push(c);
                    }
                }
            }
        }
    }
}

fn node_type_of_d(dom: &MutDom, doc: Option<&parser_pipeline::PageData>, node: u32) -> u8 {
    match node_ref(node) {
        NodeRef::Overlay(i) => match dom.kind_at(i) {
            KIND_TEXT => 3,
            KIND_FRAGMENT => 11,
            KIND_EVENT_TARGET => 0,
            _ => 1,
        },
        NodeRef::Base(node) => doc
            .map(|p| {
                let f = p.dom.flags(node);
                if f & parser_pipeline::dom::node_flags::TEXT != 0 {
                    3
                } else if f & parser_pipeline::dom::node_flags::ELEMENT != 0 {
                    1
                } else {
                    0
                }
            })
            .unwrap_or(0),
    }
}

fn walk_nodes_into(doc: Option<&parser_pipeline::PageData>, root: u32, out: &mut SmallVec<[u32; 512]>) {
    if out.len() >= ELEM_CAP {
        return;
    }
    with_rt(|rt| {
        let d = &rt.dom;
        let mut stack: SmallVec<[u32; 128]> = SmallVec::new();
        let mut roots: SmallVec<[u32; 16]> = SmallVec::new();
        children_into_d(d, doc, root, &mut roots);
        stack.extend(roots.iter().rev().copied());
        while let Some(n) = stack.pop() {
            if node_type_of_d(d, doc, n) == 1 {
                out.push(n);
                if out.len() >= ELEM_CAP {
                    return;
                }
            }
            let mut kids: SmallVec<[u32; 16]> = SmallVec::new();
            children_into_d(d, doc, n, &mut kids);
            stack.extend(kids.iter().rev().copied());
        }
    });
}

pub(crate) fn walk_subtree_into(root: u32, out: &mut SmallVec<[u32; 512]>) {
    with_doc(|doc| walk_nodes_into(doc, root, out));
}

pub(crate) fn walk_elements_into(out: &mut SmallVec<[u32; 512]>) {
    with_doc(|doc| walk_nodes_into(doc, u32::MAX, out));
    overlay_all_elements_into(out);
}

pub(crate) fn overlay_all_elements_into(out: &mut SmallVec<[u32; 512]>) {
    if out.len() >= 4096 {
        return;
    }
    overlay_scan(
        |d, i| overlay_element(d, i),
        |i| {
            out.push(OVERLAY_BASE + i as u32);
            out.len() < 4096
        },
    );
}

pub(crate) fn overlay_first_tag(tag: &str) -> Option<u32> {
    let want = parser_pipeline::dom::TAGS.get(tag).copied();
    let mut found = None;
    overlay_scan(
        |d, i| overlay_hit(d, i, want, tag),
        |i| {
            found = Some(OVERLAY_BASE + i as u32);
            false
        },
    );
    found
}

pub(crate) fn find_by_id_view(id: &str) -> Option<u32> {
    let key = CompactString::new(id);
    let hit = with_rt(|rt| {
        let d = &rt.dom;
        if let Some(list) = d.id_index.get(&key) {
            for &n in list {
                if d.slots[MutDom::si(n)].parent != u32::MAX {
                    return Some(n);
                }
            }
        }
        if let Some(list) = d.base_id_index.get(&key) {
            for &n in list {
                if d.place_of(n) != PLACE_DETACHED {
                    return Some(n);
                }
            }
        }
        None
    });
    hit.or_else(|| with_doc(|doc| doc.and_then(|p| p.dom.find_by_id(id))))
}

pub(crate) fn hidden_flag_of(node: u32) -> bool {
    match node_ref(node) {
        NodeRef::Overlay(_) => view_attr(node, "hidden").is_some(),
        NodeRef::Base(node) => {
            let base_hidden = with_doc(|d| {
                d.map(|p| p.dom.flags(node) & parser_pipeline::dom::node_flags::HIDDEN != 0)
                    .unwrap_or(false)
            });
            base_hidden || view_attr(node, "hidden").is_some()
        }
    }
}
