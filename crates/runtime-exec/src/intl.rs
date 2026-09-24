use crate::touch::{self, ApiKey};
use crate::webidl::{REG_INTL, RegCell, RegVec, class_tag, this_id};
use crate::worker::with_prof;
use compact_str::CompactString;
use fixed_decimal::Decimal;
use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorPreferences};
use icu_datetime::fieldsets::builder::{DateFields, FieldSetBuilder};
use icu_datetime::fieldsets::enums::CompositeDateTimeFieldSet;
use icu_datetime::input::{Date, DateTime, Time};
use icu_datetime::options::{Length, TimePrecision, YearStyle};
use icu_datetime::{DateTimeFormatter, DateTimeFormatterPreferences};
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_decimal::{DecimalFormatter, DecimalFormatterPreferences};
use icu_experimental::dimension::currency::formatter::{
    CurrencyFormatter, CurrencyFormatterPreferences,
};
use icu_experimental::dimension::currency::options::CurrencyFormatterOptions;
use icu_experimental::dimension::percent::formatter::{
    PercentFormatter, PercentFormatterPreferences,
};
use icu_experimental::displaynames::DisplayNamesPreferences;
use icu_experimental::displaynames::multi::{
    LanguageDisplayNames, RegionDisplayNames, ScriptDisplayNames,
};
use icu_experimental::relativetime::options::Numeric;
use icu_experimental::relativetime::{
    RelativeTimeFormatter, RelativeTimeFormatterOptions, RelativeTimeFormatterPreferences,
};
use icu_list::options::{ListFormatterOptions, ListLength};
use icu_list::{ListFormatter, ListFormatterPreferences};
use icu_locale_core::Locale;
use icu_locale_core::preferences::extensions::unicode::keywords::CollationCaseFirst;
use icu_locale_core::preferences::extensions::unicode::keywords::CurrencyType;
use icu_locale_core::subtags::{Language, Region, Script};
use icu_plurals::{PluralCategory, PluralRules, PluralRulesPreferences};
use icu_segmenter::{GraphemeClusterSegmenter, WordSegmenter};
use rquickjs::function::This;
use rquickjs::{Ctx, Function, IntoJs, Object, Persistent, Value};
use smallvec::SmallVec;
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;
use core_utils::{FxBuild, fx_map};
use writeable::{Part, PartsWrite, Writeable};

const REG_CAP: usize = 2048;

static BLOB_ZST: &[u8] = include_bytes!("intl_data.postcard.zst");

type FallbackBlob =
    icu_provider_adapters::fallback::LocaleFallbackProvider<icu_provider_blob::BlobDataProvider>;
type IntlProvider = icu_provider::buf::DeserializingBufferProvider<'static, FallbackBlob>;

static BLOB: std::sync::OnceLock<&'static [u8]> = std::sync::OnceLock::new();

fn blob() -> &'static [u8] {
    BLOB.get_or_init(|| {
        let raw = zstd::bulk::decompress(BLOB_ZST, 8 * 1024 * 1024).expect("intl blob zst");
        Box::leak(raw.into_boxed_slice())
    })
}

static PROVIDER: std::sync::OnceLock<IntlProvider> = std::sync::OnceLock::new();

fn provider() -> &'static IntlProvider {
    PROVIDER.get_or_init(|| {
        use icu_locale_fallback::LocaleFallbacker;
        use icu_provider::buf::AsDeserializingBufferProvider;
        use icu_provider_adapters::fallback::LocaleFallbackProvider;
        use icu_provider_blob::BlobDataProvider;
        let blob = BlobDataProvider::try_new_from_static_blob(blob()).expect("intl blob postcard");
        let deser = blob.as_deserializing();
        let fallbacker = LocaleFallbacker::try_new_unstable(&deser).expect("intl fallbacker");
        let fb: &'static FallbackBlob =
            Box::leak(Box::new(LocaleFallbackProvider::new(blob, fallbacker)));
        fb.as_deserializing()
    })
}

#[inline]
fn locale_of(tag: &str) -> Option<Locale> {
    Locale::try_from_str(tag).ok()
}

#[inline]
fn und() -> Locale {
    Locale::try_from_str("und").expect("und is well-formed")
}

#[inline]
fn canonical_tag(tag: &str) -> Option<String> {
    Locale::try_from_str(tag).ok().map(|l| l.to_string())
}

const K_DTF: u8 = 0;
const K_NF: u8 = 1;
const K_COLL: u8 = 2;
const K_LF: u8 = 3;
const K_RTF: u8 = 4;
const K_SEG: u8 = 5;
const K_PLURAL: u8 = 6;
const K_LOCALE: u8 = 7;
const K_DN: u8 = 8;

#[derive(Default)]
struct FmtState {
    kind: u8,
    locale: CompactString,
    language: CompactString,
    script: CompactString,
    region: CompactString,
    tz: CompactString,
    zone: Option<core_utils::tz::TzIdx>,
    calendar: CompactString,
    numbering: CompactString,
    currency: CompactString,
    year: u8,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    weekday: u8,
    era: u8,
    tz_name: u8,
    hour_cycle: u8,
    hour12: bool,
    style: u8,
    sub: u8,
    currency_display: u8,
    sign_display: u8,
    min_int: u8,
    min_frac: u8,
    max_frac: u8,
    grouping: bool,
    coll: Option<Rc<Collator>>,
    dtf: Option<Rc<DateTimeFormatter<CompositeDateTimeFieldSet>>>,
    dec: Option<Rc<DecimalFormatter>>,
    cur: Option<Rc<CurrencyFormatter<DecimalFormatter>>>,
    pct: Option<Rc<PercentFormatter<DecimalFormatter>>>,
    lf: Option<Rc<ListFormatter>>,
    pr: Option<Rc<PluralRules>>,
    sensitivity: u8,
    ignore_punct: bool,
    coll_numeric: bool,
    case_first: u8,
    wseg: Option<Rc<WordSegmenter>>,
    gseg: Option<Rc<GraphemeClusterSegmenter>>,
    dn: Option<Rc<DnCache>>,
    rtf: Option<Box<[Option<Rc<RelativeTimeFormatter>>; 8]>>,
}

enum DnCache {
    Lang(LanguageDisplayNames),
    Region(RegionDisplayNames),
    Script(ScriptDisplayNames),
}

enum SegOut {
    Words(SmallVec<[(usize, usize, bool); 32]>),
    Graphemes(SmallVec<[(usize, usize); 64]>),
}

impl FmtState {
    fn blank(kind: u8) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }

    fn dtf(locale: CompactString) -> Self {
        Self {
            calendar: CompactString::const_new("gregory"),
            numbering: CompactString::const_new("latn"),
            locale,
            ..Self::blank(K_DTF)
        }
    }

    fn nf(locale: CompactString) -> Self {
        Self {
            numbering: CompactString::const_new("latn"),
            min_int: 1,
            max_frac: 3,
            grouping: true,
            locale,
            ..Self::blank(K_NF)
        }
    }

    fn coll(locale: CompactString) -> Self {
        Self {
            sensitivity: 3,
            locale,
            ..Self::blank(K_COLL)
        }
    }

    fn plain(kind: u8, locale: CompactString) -> Self {
        Self {
            locale,
            ..Self::blank(kind)
        }
    }

    fn locale_state(language: CompactString, script: CompactString, region: CompactString) -> Self {
        Self {
            language,
            script,
            region,
            ..Self::blank(K_LOCALE)
        }
    }
}

type LocaleDtfFmt = DateTimeFormatter<CompositeDateTimeFieldSet>;

type LocaleDtfCell = Option<(
    CompactString,
    u8,
    Option<core_utils::tz::TzIdx>,
    Option<Rc<LocaleDtfFmt>>,
)>;

pub(crate) fn clear_thunks() {
    LOCALE_DTF.with(|c| *c.borrow_mut() = None);
    REALM.with(|m| {
        let mut mm = m.borrow_mut();
        mm.map.clear();
        mm.len = 0;
        mm.head = usize::MAX;
    });
}

pub(crate) fn clear_registry() {
    REG.with(|m| m.borrow_mut().clear());
    TIME_FN.with(|t| *t.borrow_mut() = None);
    LOCALE_EXP.with(|t| *t.borrow_mut() = None);
}

thread_local! {
    static REG: RegCell<FmtState> = const { RefCell::new(RegVec::with_tag(0)) };
    static LOCALE_DTF: RefCell<LocaleDtfCell> = const { RefCell::new(None) };
}

const REALM_CAP: usize = 256;

thread_local! {
    static REALM: RefCell<RealmStore> = RefCell::new(RealmStore::new());
    static WSEG: RefCell<Option<Rc<WordSegmenter>>> = const { RefCell::new(None) };
    static GSEG: RefCell<Option<Rc<GraphemeClusterSegmenter>>> = const { RefCell::new(None) };
    static TIME_FN: RefCell<Option<Persistent<Function<'static>>>> = const { RefCell::new(None) };
}

fn sig(st: &FmtState) -> u64 {
    let pod = [
        st.kind, st.year, st.month, st.day, st.hour, st.minute, st.second, st.weekday, st.era,
        st.tz_name, st.hour_cycle, st.hour12 as u8, st.style, st.sub, st.currency_display,
        st.sign_display, st.min_int, st.min_frac, st.max_frac, st.grouping as u8, st.sensitivity,
        st.ignore_punct as u8, st.coll_numeric as u8, st.case_first,
    ];
    let mut h = core_utils::xxh3::hash(&pod);
    h = core_utils::xxh3::hash_seeded(h, st.locale.as_bytes());
    h = core_utils::xxh3::hash_seeded(h, st.calendar.as_bytes());
    h = core_utils::xxh3::hash_seeded(h, st.numbering.as_bytes());
    core_utils::xxh3::hash_seeded(h, st.currency.as_bytes())
}

fn realm_get<T: Any + 'static>(key: u64) -> Option<Rc<T>> {
    REALM.with(|m| m.borrow().map.get(&key)?.clone().downcast::<T>().ok())
}

fn realm_or_build<T: Any + 'static>(
    key: u64,
    build: impl FnOnce() -> Option<T>,
) -> Option<Rc<T>> {
    if let Some(r) = realm_get::<T>(key) {
        return Some(r);
    }
    let v = Rc::new(build()?);
    REALM.with(|m| m.borrow_mut().insert(key, v.clone()));
    Some(v)
}

struct RealmStore {
    map: HashMap<u64, Rc<dyn Any>, FxBuild>,
    ring: [u64; REALM_CAP],
    head: usize,
    len: usize,
}

impl RealmStore {
    fn new() -> Self {
        Self {
            map: fx_map(),
            ring: [0; REALM_CAP],
            head: usize::MAX,
            len: 0,
        }
    }

    #[inline]
    fn insert(&mut self, key: u64, v: Rc<dyn Any>) {
        self.head = self.head.wrapping_add(1) & (REALM_CAP - 1);
        if self.len < REALM_CAP {
            self.len += 1;
        } else {
            self.map.remove(&self.ring[self.head]);
        }
        self.ring[self.head] = key;
        self.map.insert(key, v);
    }
}


macro_rules! lazy_state {
    ($st:expr, $field:ident, $build:ident) => {
        if $st.$field.is_none() {
            let built = realm_or_build(sig($st), || $build($st));
            $st.$field = built;
        }
    };
}

fn reg_alloc(st: FmtState) -> Option<u64> {
    REG.with(|m| {
        let mut mm = m.borrow_mut();
        if mm.live() >= REG_CAP {
            return None;
        }
        Some(mm.alloc(|_| st))
    })
}

#[inline(always)]
fn reg_of_kind<R>(id: u64, kind: u8, f: impl FnOnce(&FmtState) -> R) -> Option<R> {
    REG.with(|m| {
        m.borrow()
            .get_ro(id, |st| (st.kind == kind).then(|| f(st)))
            .flatten()
    })
}

fn checked<'js, R>(
    c: &Ctx<'js>,
    this: &Value<'js>,
    kind: u8,
    f: impl FnOnce(&FmtState) -> R,
) -> rquickjs::Result<R> {
    let id = this_id(c, this, REG_INTL)?;
    reg_of_kind(id, kind, f).ok_or_else(|| throw_incompat(c))
}

#[inline(always)]
fn reg_of_kind_mut<R>(id: u64, kind: u8, f: impl FnOnce(&mut FmtState) -> R) -> Option<R> {
    REG.with(|m| {
        m.borrow_mut()
            .with(id, |st| (st.kind == kind).then(|| f(st)))
            .flatten()
    })
}

fn checked_mut<'js, R>(
    c: &Ctx<'js>,
    this: &Value<'js>,
    kind: u8,
    f: impl FnOnce(&mut FmtState) -> R,
) -> rquickjs::Result<R> {
    let id = this_id(c, this, REG_INTL)?;
    reg_of_kind_mut(id, kind, f).ok_or_else(|| throw_incompat(c))
}

fn make_instance<'js>(
    c: &Ctx<'js>,
    proto: &Object<'js>,
    tag: &'static str,
    st: FmtState,
) -> rquickjs::Result<Value<'js>> {
    let Some(id) = reg_alloc(st) else {
        return Err(rquickjs::Exception::throw_message(
            c,
            "intl registry saturated",
        ));
    };
    crate::webidl::registry_instance(c, tag, crate::webidl::REG_INTL, id, proto)
}

fn arg<'js>(c: &Ctx<'js>, v: rquickjs::function::Opt<Value<'js>>) -> Value<'js> {
    v.0.unwrap_or_else(|| Value::new_undefined(c.clone()))
}

fn cstr_of<'js>(v: &Value<'js>) -> Option<rquickjs::CString<'js>> {
    v.as_string()?.clone().to_cstring().ok()
}

fn export_prop<'js, V: Into<Value<'js>>>(
    o: &Object<'js>,
    name: &str,
    v: V,
) -> rquickjs::Result<()> {
    o.prop(
        name,
        rquickjs::object::Property::from(v.into())
            .writable()
            .configurable()
            .enumerable(),
    )
}

macro_rules! intl_class {
    ($ctx:expr, $intl:expr, $name:literal, |$c:ident, $locale:ident, $opts:ident| $body:expr, supported) => {
        intl_class_inner!($ctx, $intl, $name, |$c, $locale, $opts| $body, true)
    };
    ($ctx:expr, $intl:expr, $name:literal, |$c:ident, $locale:ident, $opts:ident| $body:expr) => {
        intl_class_inner!($ctx, $intl, $name, |$c, $locale, $opts| $body, false)
    };
}

macro_rules! intl_class_inner {
    ($ctx:expr, $intl:expr, $name:literal, |$c:ident, $locale:ident, $opts:ident| $body:expr, $supported:expr) => {{
        let proto = Object::new($ctx.clone())?;
        class_tag($ctx, &proto, concat!("Intl.", $name))?;
        let ctor_proto = proto.clone();
        let ctor = Function::new(
            $ctx.clone(),
            move |$c: Ctx<'js>,
                  locales: rquickjs::function::Opt<Value<'js>>,
                  opts: rquickjs::function::Opt<Value<'js>>|
                  -> rquickjs::Result<Value<'js>> {
                touch::touch_log_record(ApiKey::INTL);
                let locales = arg(&$c, locales);
                let raw_opts = arg(&$c, opts);
                let $locale = resolved_locale(&$c, &locales)?;
                let $opts = opts_object_of(&$c, &raw_opts)?;
                let st = $body;
                make_instance(&$c, &ctor_proto, concat!("Intl.", $name), st)
            },
        )?
        .with_constructor(true);
        crate::stackfmt::set_fn_name($ctx, &ctor, $name)?;
        link_ctor($ctx, &ctor, &proto)?;
        if $supported {
            attach_supported($ctx, &ctor)?;
        }
        export_prop($intl, $name, ctor)?;
        proto
    }};
}

fn opts_object_of<'js>(c: &Ctx<'js>, opts: &Value<'js>) -> rquickjs::Result<Object<'js>> {
    if let Some(o) = opts.as_object()
        && !opts.is_undefined()
        && !opts.is_null()
    {
        Ok(o.clone())
    } else {
        Object::new(c.clone())
    }
}

fn supported_locales_of_fn<'js>(
    c: Ctx<'js>,
    locales: rquickjs::function::Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let locales = arg(&c, locales);
    supported_locales(&c, &locales)
}

fn attach_supported<'js>(ctx: &Ctx<'js>, ctor: &Function<'js>) -> rquickjs::Result<()> {
    let supported: Function<'js> = Function::new(ctx.clone(), supported_locales_of_fn)?;
    crate::stackfmt::set_fn_name(ctx, &supported, "supportedLocalesOf")?;
    crate::stackfmt::set_fn_len(ctx, &supported, 1)?;
    export_prop(ctor, "supportedLocalesOf", supported)
}

fn install_resolved<'js, F>(
    ctx: &Ctx<'js>,
    proto: &Object<'js>,
    kind: u8,
    build: F,
) -> rquickjs::Result<()>
where
    F: Fn(&FmtState, &Object<'js>) -> rquickjs::Result<()> + 'js,
{
    let resolved = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let o = checked(&c, &this.0, kind, |st| -> rquickjs::Result<Object<'js>> {
                let o = Object::new(c.clone())?;
                o.set("locale", st.locale.as_str())?;
                build(st, &o)?;
                Ok(o)
            })??;
            Ok(o.into_value())
        },
    )?;
    crate::webidl::define_method(ctx, proto, "resolvedOptions", resolved)?;
    Ok(())
}

fn link_ctor<'js>(
    ctx: &Ctx<'js>,
    ctor: &Function<'js>,
    proto: &Object<'js>,
) -> rquickjs::Result<()> {
    crate::stackfmt::set_fn_len(ctx, ctor, 0)?;
    ctor.prop(
        "prototype",
        rquickjs::object::Property::from(proto.clone())
            .writable()
            .configurable(),
    )?;
    proto.prop(
        "constructor",
        rquickjs::object::Property::from(ctor.clone())
            .writable()
            .configurable(),
    )
}

fn throw_incompat<'js>(c: &Ctx<'js>) -> rquickjs::Error {
    rquickjs::Exception::throw_type(c, "Intl method called on incompatible receiver")
}

fn is_windows() -> bool {
    with_prof(|p| p.windows)
}

fn ms_f64_of_value(v: &Value<'_>) -> f64 {
    if let Some(n) = v.as_number() {
        return n;
    }
    if let Some(o) = v.as_object() {
        let ctx = o.ctx().clone();
        let cached = TIME_FN
            .with(|t| t.borrow().clone())
            .and_then(|p| p.restore(&ctx).ok());
        if let Some(f) = cached {
            let mut args = rquickjs::function::Args::new(ctx.clone(), 0);
            if args.this(o.clone()).is_ok() {
                let n: f64 = f.call_arg(args).unwrap_or(f64::NAN);
                if n.is_finite() {
                    return n;
                }
            }
        }
        for key in ["getTime", "valueOf"] {
            let f: Option<Function> = o.get(key).ok().flatten();
            if let Some(f) = f {
                let mut args = rquickjs::function::Args::new(ctx.clone(), 0);
                if args.this(o.clone()).is_ok() {
                    let n: f64 = f.call_arg(args).unwrap_or(f64::NAN);
                    if n.is_finite() {
                        if key == "getTime" {
                            TIME_FN.with(|t| {
                                *t.borrow_mut() = Some(Persistent::save(&ctx, f.clone()));
                            });
                        }
                        return n;
                    }
                }
            }
        }
    }
    f64::NAN
}

fn locale_of_arg(locales: &Value<'_>) -> Option<CompactString> {
    if locales.is_string() {
        let s = locales.as_string()?;
        let cs = s.clone().to_cstring().ok()?;
        return Some(CompactString::new(cs.as_str()));
    }
    if locales.is_array() {
        let arr = locales.as_array()?;
        let first: Option<rquickjs::String<'_>> = arr.get(0).ok().flatten();
        let s = first?;
        let cs = s.to_cstring().ok()?;
        return Some(CompactString::new(cs.as_str()));
    }
    None
}

fn throw_range_invalid<'js>(c: &Ctx<'js>, tag: &str) -> rquickjs::Error {
    let mut msg = CompactString::const_new("Invalid language tag: ");
    msg.push_str(tag);
    rquickjs::Exception::throw_range(c, msg.as_str())
}

static CANON_LOCALE: std::sync::LazyLock<scc::HashMap<CompactString, CompactString>> =
    std::sync::LazyLock::new(scc::HashMap::new);

fn resolved_locale<'js>(c: &Ctx<'js>, locales: &Value<'js>) -> rquickjs::Result<CompactString> {
    match locale_of_arg(locales) {
        Some(tag) => {
            if let Some(hit) = CANON_LOCALE.read_sync(&tag, |_, v| v.clone()) {
                return Ok(hit);
            }
            let can =
                canonical_tag(tag.as_str()).ok_or_else(|| throw_range_invalid(c, tag.as_str()))?;
            let out = CompactString::new(can);
            if CANON_LOCALE.len() < 4096 {
                let _ = CANON_LOCALE.insert_sync(tag, out.clone());
            }
            Ok(out)
        }
        None => Ok(with_prof(|p| p.prof().locale.clone())),
    }
}

fn opt_code(table: &[(&'static str, u8)], v: &str) -> Option<u8> {
    table.iter().find(|(s, _)| *s == v).map(|(_, c)| *c)
}

fn opt_name(table: &[(&'static str, u8)], code: u8) -> &'static str {
    table
        .iter()
        .find(|(_, c)| *c == code)
        .map(|(s, _)| *s)
        .unwrap_or(table[0].0)
}

const STYLE_TABLE: &[(&str, u8)] = &[("long", 0), ("short", 1), ("narrow", 2)];
const DIGIT2: &[(&'static str, u8)] = &[("2-digit", 1), ("numeric", 2)];
const CURRENCY_DISPLAY: &[(&str, u8)] =
    &[("symbol", 0), ("name", 1), ("code", 2), ("narrowSymbol", 3)];
const SENSITIVITY: &[(&str, u8)] = &[("variant", 3), ("base", 0), ("accent", 1), ("case", 2)];
const CASE_FIRST: &[(&str, u8)] = &[("auto", 0), ("lower", 1), ("upper", 2)];
const SIGN_DISPLAY: &[(&str, u8)] = &[
    ("auto", 0),
    ("always", 1),
    ("never", 2),
    ("exceptZero", 3),
    ("negative", 4),
];

fn style_name(v: u8) -> &'static str {
    opt_name(STYLE_TABLE, v)
}

fn month_name(v: u8) -> &'static str {
    match v {
        1 => "2-digit",
        2 => "numeric",
        3 => "narrow",
        4 => "short",
        _ => "long",
    }
}

fn style_of(v: &str) -> Option<u8> {
    opt_code(STYLE_TABLE, v)
}

const DTF_RESOLVED: &[(&str, fn(&FmtState) -> u8, fn(u8) -> &'static str)] = &[
    (
        "weekday",
        |st: &FmtState| st.weekday,
        |v: u8| style_name(v - 1),
    ),
    ("era", |st: &FmtState| st.era, |v: u8| style_name(v - 1)),
    ("year", |st: &FmtState| st.year, |v: u8| opt_name(DIGIT2, v)),
    ("month", |st: &FmtState| st.month, month_name),
    ("day", |st: &FmtState| st.day, |v: u8| opt_name(DIGIT2, v)),
    ("hour", |st: &FmtState| st.hour, |v: u8| opt_name(DIGIT2, v)),
    (
        "minute",
        |st: &FmtState| st.minute,
        |v: u8| opt_name(DIGIT2, v),
    ),
    (
        "second",
        |st: &FmtState| st.second,
        |v: u8| opt_name(DIGIT2, v),
    ),
    (
        "timeZoneName",
        |st: &FmtState| st.tz_name,
        |v: u8| style_name(v - 1),
    ),
];

fn option_str<'js>(o: &Object<'js>, key: &str) -> Option<CompactString> {
    let v: Value = o.get(key).ok().flatten()?;
    if v.is_undefined() || v.is_null() {
        return None;
    }
    let s = v.into_string()?;
    let cs = s.to_cstring().ok()?;
    Some(CompactString::new(cs.as_str()))
}

fn option_bool(o: &Object<'_>, key: &str) -> Option<bool> {
    let v: Value = o.get(key).ok().flatten()?;
    if v.is_undefined() || v.is_null() {
        return None;
    }
    match v.type_of() {
        rquickjs::Type::Bool => v.as_bool(),
        rquickjs::Type::String => cstr_of(&v).as_ref().map(|cs| !cs.is_empty()),
        rquickjs::Type::Int | rquickjs::Type::Float => {
            Some(v.as_number().unwrap_or(0.0) != 0.0)
        }
        _ => Some(true),
    }
}

fn supported_locales<'js>(c: &Ctx<'js>, locales: &Value<'js>) -> rquickjs::Result<Value<'js>> {
    let arr = rquickjs::Array::new(c.clone())?;
    if locales.is_string() {
        let sc = cstr_of(locales);
        let s: &str = sc.as_ref().map(|cs| cs.as_str()).unwrap_or("");
        let can = canonical_tag(s).ok_or_else(|| throw_range_invalid(c, s))?;
        arr.set(0, can.as_str())?;
        return Ok(arr.into_value());
    }
    if let Some(list) = locales.as_array() {
        for (i, item) in list.iter::<String>().enumerate() {
            let s =
                item.map_err(|_| rquickjs::Exception::throw_type(c, "Cannot convert to string"))?;
            let can =
                canonical_tag(s.as_str()).ok_or_else(|| throw_range_invalid(c, s.as_str()))?;
            arr.set(i, can.as_str())?;
        }
    }
    Ok(arr.into_value())
}

fn icu_locale_with(st: &FmtState, hc: Option<&str>) -> Option<Locale> {
    let mut tag = st.locale.as_str().to_owned();
    let mut keys: SmallVec<[&str; 4]> = SmallVec::new();
    if st.calendar != "gregory" && !st.calendar.is_empty() {
        keys.push("ca");
    }
    if hc.is_some() {
        keys.push("hc");
    }
    if st.numbering != "latn" && !st.numbering.is_empty() {
        keys.push("nu");
    }
    if !keys.is_empty() {
        let _ = write!(tag, "-u");
        for k in keys {
            let _ = write!(tag, "-{k}");
            match k {
                "ca" => {
                    let _ = write!(tag, "-{}", st.calendar.as_str());
                }
                "hc" => {
                    let _ = write!(tag, "-{}", hc.unwrap_or("h23"));
                }
                _ => {
                    let _ = write!(tag, "-{}", st.numbering.as_str());
                }
            }
        }
    }
    locale_of(&tag)
}

fn icu_locale_of(st: &FmtState) -> Locale {
    icu_locale_with(st, None).unwrap_or_else(und)
}

fn dec_of(x: f64) -> Option<Decimal> {
    Decimal::try_from_f64(x, fixed_decimal::FloatPrecision::RoundTrip).ok()
}

fn decimal_prefs(st: &FmtState) -> DecimalFormatterPreferences {
    DecimalFormatterPreferences::from(&icu_locale_of(st))
}

fn decimal_of(x: f64, st: &FmtState) -> Option<Decimal> {
    if !x.is_finite() {
        return None;
    }
    let (default_min_frac, default_max_frac) = if st.style == 1 { (2u8, 2u8) } else { (0, 0) };
    let min_frac = if st.min_frac == 0 && default_min_frac > 0 {
        default_min_frac
    } else {
        st.min_frac
    };
    let max_frac = if st.max_frac == 0 && default_max_frac > 0 {
        default_max_frac
    } else {
        st.max_frac
    };
    let mut dec = dec_of(x)?;
    if st.style == 2 {
        dec.multiply_pow10(2);
    }
    let max_f = (max_frac.min(20)) as i16;
    let min_f = (min_frac.min(20)) as i16;
    if max_f < 17 && *dec.absolute.magnitude_range().start() < -max_f {
        dec = dec.rounded(-max_f);
    }
    if min_f > 0 {
        dec = dec.expanded(-min_f);
    }
    if st.min_int > 1 {
        dec.absolute.pad_start(st.min_int as i16);
    }
    use fixed_decimal::SignDisplay as DecSign;
    dec = dec.with_sign_display(match st.sign_display {
        1 => DecSign::Always,
        2 => DecSign::Never,
        3 => DecSign::ExceptZero,
        4 => DecSign::Negative,
        _ => DecSign::Auto,
    });
    Some(dec)
}

fn build_dec(st: &FmtState) -> Option<DecimalFormatter> {
    let mut opts = DecimalFormatterOptions::default();
    opts.grouping_strategy = Some(match st.grouping {
        true => GroupingStrategy::Auto,
        false => GroupingStrategy::Never,
    });
    DecimalFormatter::try_new_unstable(provider(), decimal_prefs(st), opts).ok()
}

fn format_number(x: f64, st: &mut FmtState) -> Result<CompactString, ()> {
    if x.is_nan() {
        return Ok(CompactString::const_new("NaN"));
    }
    if x.is_infinite() {
        return Ok(CompactString::const_new("∞"));
    }
    let Some(dec) = decimal_of(x, st) else {
        return Ok(CompactString::const_new("NaN"));
    };
    match st.style {
        1 => currency_string(dec, st).ok_or(()),
        2 => percent_string(dec, st).ok_or(()),
        _ => {
            lazy_state!(st, dec, build_dec);
            let f = st.dec.as_deref().ok_or(())?;
            let mut out = CompactString::new("");
            let _ = f.format(&dec).write_to(&mut out);
            Ok(out)
        }
    }
}

fn build_cur(st: &FmtState) -> Option<CurrencyFormatter<DecimalFormatter>> {
    let p = provider();
    let prefs = CurrencyFormatterPreferences::from(&icu_locale_of(st));
    let cur =
        CurrencyType::try_from_str(&core_utils::ascii_lower_compact(st.currency.as_str())).ok()?;
    let opts = CurrencyFormatterOptions::default();
    match st.currency_display {
        2 | 1 => CurrencyFormatter::try_new_code_unstable(p, prefs, cur, opts),
        3 => CurrencyFormatter::try_new_symbol_narrow_unstable(p, prefs, cur, opts),
        _ => CurrencyFormatter::try_new_symbol_unstable(p, prefs, cur, opts),
    }
    .ok()
}

fn currency_string(dec: Decimal, st: &mut FmtState) -> Option<CompactString> {
    lazy_state!(st, cur, build_cur);
    let f = st.cur.as_deref()?;
    let mut out = CompactString::new("");
    let _ = f.format_fixed_decimal(&dec).write_to(&mut out);
    Some(out)
}

fn build_pct(st: &FmtState) -> Option<PercentFormatter<DecimalFormatter>> {
    let prefs = PercentFormatterPreferences::from(&icu_locale_of(st));
    PercentFormatter::try_new_unstable(provider(), prefs, Default::default()).ok()
}

fn percent_string(dec: Decimal, st: &mut FmtState) -> Option<CompactString> {
    lazy_state!(st, pct, build_pct);
    let f = st.pct.as_deref()?;
    let mut out = CompactString::new("");
    let _ = f.format(&dec).write_to(&mut out);
    Some(out)
}

#[inline]
fn ord_i32(o: std::cmp::Ordering) -> i32 {
    match o {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn build_coll(st: &FmtState) -> Option<Collator> {
    use icu_collator::options::{AlternateHandling, CaseLevel, Strength};
    use icu_locale_core::preferences::extensions::unicode::keywords::CollationNumericOrdering;
    let p = provider();
    let mut prefs = CollatorPreferences::from(&icu_locale_of(st));
    prefs.case_first = Some(match st.case_first {
        1 => CollationCaseFirst::Lower,
        2 => CollationCaseFirst::Upper,
        _ => CollationCaseFirst::False,
    });
    prefs.numeric_ordering = Some(if st.coll_numeric {
        CollationNumericOrdering::True
    } else {
        CollationNumericOrdering::False
    });
    let mut opts = CollatorOptions::default();
    match st.sensitivity {
        0 => opts.strength = Some(Strength::Primary),
        1 => opts.strength = Some(Strength::Secondary),
        2 => {
            opts.strength = Some(Strength::Primary);
            opts.case_level = Some(CaseLevel::On);
        }
        _ => opts.strength = Some(Strength::Tertiary),
    }
    if st.ignore_punct {
        opts.alternate_handling = Some(AlternateHandling::Shifted);
    }
    Collator::try_new_unstable(p, prefs, opts).ok()
}

fn coll_compare(a: &str, b: &str, st: &mut FmtState) -> i32 {
    lazy_state!(st, coll, build_coll);
    match st.coll.as_deref() {
        Some(c) => ord_i32(c.as_borrowed().compare(a, b)),
        None => ord_i32(a.as_bytes().cmp(b.as_bytes())),
    }
}

fn build_pr(st: &FmtState) -> Option<PluralRules> {
    let prefs = PluralRulesPreferences::from(&icu_locale_of(st));
    if st.sub == 1 {
        PluralRules::try_new_ordinal_unstable(provider(), prefs).ok()
    } else {
        PluralRules::try_new_cardinal_unstable(provider(), prefs).ok()
    }
}

fn plural_category(st: &mut FmtState, x: f64) -> &'static str {
    lazy_state!(st, pr, build_pr);
    let Some(rules) = st.pr.as_deref() else {
        return "other";
    };
    let Some(dec) = dec_of(x) else {
        return "other";
    };
    match rules.category_for(&dec) {
        PluralCategory::Zero => "zero",
        PluralCategory::One => "one",
        PluralCategory::Two => "two",
        PluralCategory::Few => "few",
        PluralCategory::Many => "many",
        PluralCategory::Other => "other",
    }
}

fn build_lf(st: &FmtState) -> Option<ListFormatter> {
    let prefs = ListFormatterPreferences::from(&icu_locale_of(st));
    let mut opts = ListFormatterOptions::default();
    opts.length = Some(match st.style {
        0 => ListLength::Wide,
        1 => ListLength::Short,
        _ => ListLength::Narrow,
    });
    if st.sub == 1 {
        ListFormatter::try_new_or_unstable(provider(), prefs, opts).ok()
    } else {
        ListFormatter::try_new_and_unstable(provider(), prefs, opts).ok()
    }
}

fn list_format(items: &[CompactString], st: &mut FmtState) -> CompactString {
    lazy_state!(st, lf, build_lf);
    let mut out = CompactString::new("");
    let Some(f) = st.lf.as_deref() else {
        for (i, s) in items.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(s.as_str());
        }
        return out;
    };
    let strs: SmallVec<[&str; 8]> = items.iter().map(|s| s.as_str()).collect();
    let _ = f.format(strs.into_iter()).write_to(&mut out);
    out
}

const RTF_UNITS: &[&str] = &[
    "second", "minute", "hour", "day", "week", "month", "quarter", "year",
];

type RtfCtor = fn(
    &IntlProvider,
    RelativeTimeFormatterPreferences,
    RelativeTimeFormatterOptions,
) -> Result<RelativeTimeFormatter, icu_provider::DataError>;

const RTF_CTORS: [[RtfCtor; 3]; 8] = [
    [
        RelativeTimeFormatter::try_new_long_second_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_short_second_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_narrow_second_unstable as RtfCtor,
    ],
    [
        RelativeTimeFormatter::try_new_long_minute_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_short_minute_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_narrow_minute_unstable as RtfCtor,
    ],
    [
        RelativeTimeFormatter::try_new_long_hour_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_short_hour_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_narrow_hour_unstable as RtfCtor,
    ],
    [
        RelativeTimeFormatter::try_new_long_day_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_short_day_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_narrow_day_unstable as RtfCtor,
    ],
    [
        RelativeTimeFormatter::try_new_long_week_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_short_week_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_narrow_week_unstable as RtfCtor,
    ],
    [
        RelativeTimeFormatter::try_new_long_month_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_short_month_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_narrow_month_unstable as RtfCtor,
    ],
    [
        RelativeTimeFormatter::try_new_long_quarter_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_short_quarter_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_narrow_quarter_unstable as RtfCtor,
    ],
    [
        RelativeTimeFormatter::try_new_long_year_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_short_year_unstable as RtfCtor,
        RelativeTimeFormatter::try_new_narrow_year_unstable as RtfCtor,
    ],
];

fn build_rtf(st: &FmtState, ui: usize) -> Option<RelativeTimeFormatter> {
    let prefs = RelativeTimeFormatterPreferences::from(&icu_locale_of(st));
    let mut opts = RelativeTimeFormatterOptions::default();
    opts.numeric = if st.sub == 0 {
        Numeric::Always
    } else {
        Numeric::Auto
    };
    RTF_CTORS[ui][st.style as usize](provider(), prefs, opts).ok()
}

fn rtf_format(n: f64, st: &mut FmtState, unit: &str) -> CompactString {
    let dec = dec_of(n).unwrap_or_else(|| Decimal::from(0i64));
    let ui = RTF_UNITS
        .iter()
        .position(|x| *x == unit)
        .unwrap_or(RTF_UNITS.len() - 1);
    let key = core_utils::xxh3::hash_seeded(sig(st), &[ui as u8]);
    let built = if st.rtf.as_ref().is_none_or(|a| a[ui].is_none()) {
        realm_or_build(key, || build_rtf(st, ui))
    } else {
        None
    };
    if let Some(b) = built {
        st.rtf.get_or_insert_with(|| Box::new(Default::default()))[ui] = Some(b);
    }
    let f = st.rtf.as_ref().and_then(|a| a[ui].clone());
    let Some(f) = f.as_deref() else {
        return CompactString::new("");
    };
    let mut out = CompactString::new("");
    let _ = f.format(dec).write_to(&mut out);
    out
}

struct PartsSink {
    frames: SmallVec<[(Part, CompactString); 4]>,
    parts: SmallVec<[(&'static str, CompactString); 16]>,
}

impl PartsSink {
    fn new() -> Self {
        let mut frames: SmallVec<[(Part, CompactString); 4]> = SmallVec::new();
        frames.push((
            Part {
                category: "literal",
                value: "literal",
            },
            CompactString::new(""),
        ));
        PartsSink {
            frames,
            parts: SmallVec::new(),
        }
    }
}

impl core::fmt::Write for PartsSink {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.frames.last_mut().expect("base frame").1.push_str(s);
        Ok(())
    }
}

impl PartsWrite for PartsSink {
    type SubPartsWrite = PartsSink;

    fn with_part(
        &mut self,
        part: Part,
        f: impl FnOnce(&mut Self::SubPartsWrite) -> core::fmt::Result,
    ) -> core::fmt::Result {
        if self.frames.len() == 1
            && let Some((_, buf)) = self.frames.last_mut()
            && !buf.is_empty()
        {
            let lit = std::mem::take(buf);
            self.parts.push(("literal", lit));
        }
        self.frames.push((part, CompactString::new("")));
        f(self)?;
        let (part, seg) = self.frames.pop().expect("part frame");
        if seg.is_empty() {
            return Ok(());
        }
        if self.frames.len() == 1 {
            self.parts.push((part.value, seg));
        } else if let Some((_, parent)) = self.frames.last_mut() {
            parent.push_str(&seg);
        }
        Ok(())
    }
}

type DtfParts = SmallVec<[(&'static str, CompactString); 16]>;

fn dtf_fieldset(st: &FmtState) -> Option<CompositeDateTimeFieldSet> {
    let has_date = st.month > 0 || st.day > 0 || st.year > 0 || st.era > 0 || st.weekday > 0;
    let has_time = st.hour > 0 || st.minute > 0 || st.second > 0;
    let named_month = matches!(st.month, 3..=5);
    let length = if st.month == 5 || (st.weekday == 1 && st.month >= 4) {
        Length::Long
    } else if named_month || st.weekday > 0 {
        Length::Medium
    } else {
        Length::Short
    };
    let mut b = FieldSetBuilder::new();
    b.length = Some(length);
    if st.era > 0 {
        b.year_style = Some(YearStyle::WithEra);
    } else if st.year == 2 {
        b.year_style = Some(YearStyle::Full);
    }
    if st.hour > 0 {
        b.time_precision = Some(if st.second > 0 {
            TimePrecision::Second
        } else if st.minute > 0 {
            TimePrecision::Minute
        } else {
            TimePrecision::Hour
        });
    }
    if !has_date && has_time {
        return Some(CompositeDateTimeFieldSet::Time(b.build_time().ok()?));
    }
    let date_fields = match (st.year > 0, st.month > 0, st.day > 0, st.weekday > 0) {
        (false, false, false, true) => DateFields::E,
        (false, true, false, false) => DateFields::M,
        (true, false, false, false) => DateFields::Y,
        (true, true, false, false) => DateFields::YM,
        (false, true, true, false) => DateFields::MD,
        (true, true, true, false) => DateFields::YMD,
        (false, true, true, true) => DateFields::MDE,
        (true, true, true, true) => DateFields::YMDE,
        (false, false, true, false) => DateFields::D,
        (false, false, true, true) => DateFields::DE,
        _ => DateFields::YMD,
    };
    b.date_fields = Some(date_fields);
    if has_time {
        b.build_composite_datetime().ok()
    } else {
        Some(CompositeDateTimeFieldSet::Date(b.build_date().ok()?))
    }
}

fn build_dtf(st: &FmtState) -> Option<LocaleDtfFmt> {
    let hc = if st.hour_cycle == 0 {
        None
    } else {
        HOUR_CYCLES.get(st.hour_cycle as usize - 1).copied()
    };
    let loc = icu_locale_with(st, hc).unwrap_or_else(|| icu_locale_of(st));
    let prefs = DateTimeFormatterPreferences::from(&loc);
    let fieldset = dtf_fieldset(st)?;
    DateTimeFormatter::try_new_unstable(provider(), prefs, fieldset).ok()
}

#[inline]
fn tz_suffix(
    zone: Option<core_utils::tz::TzIdx>,
    tz_name: u8,
    unix_ms: i64,
) -> CompactString {
    let mut s = CompactString::with_capacity(48);
    s.push_str("GMT");
    let off = core_utils::tz::tz_offset_zone(zone, unix_ms);
    let a = off.unsigned_abs();
    let h = (a / 60) as i64;
    let m = (a % 60) as i64;
    s.push(if off < 0 { '-' } else { '+' });
    core_utils::math::push_int_padded_into(&mut s, h, 2);
    core_utils::math::push_int_padded_into(&mut s, m, 2);
    core_utils::tz::zone_label_zone(zone, unix_ms, tz_name != 1, &mut s);
    s
}

#[inline]
fn iso_fallback(cv: &core_utils::tz::Civil) -> CompactString {
    let mut s = CompactString::with_capacity(24);
    core_utils::math::push_int_padded_into(&mut s, cv.year as i64, 4);
    s.push('-');
    core_utils::math::push_int_padded_into(&mut s, cv.month as i64, 2);
    s.push('-');
    core_utils::math::push_int_padded_into(&mut s, cv.day as i64, 2);
    s.push('T');
    core_utils::math::push_int_padded_into(&mut s, cv.hour as i64, 2);
    s.push(':');
    core_utils::math::push_int_padded_into(&mut s, cv.minute as i64, 2);
    s.push(':');
    core_utils::math::push_int_padded_into(&mut s, cv.second as i64, 2);
    s
}

fn dt_of(cv: &core_utils::tz::Civil) -> Option<DateTime<icu_calendar::Iso>> {
    let date = Date::try_new_iso(cv.year, cv.month, cv.day).ok();
    let time = Time::try_new(cv.hour, cv.minute, cv.second, 0).ok();
    Some(DateTime {
        date: date?,
        time: time?,
    })
}

fn fmt_date_with(
    fmt: Option<&LocaleDtfFmt>,
    zone: Option<core_utils::tz::TzIdx>,
    tz_name: u8,
    unix_ms: i64,
) -> DtfParts {
    let off = core_utils::tz::tz_offset_zone(zone, unix_ms);
    let cv = core_utils::tz::civil_of(unix_ms, off);
    let mut parts: DtfParts = SmallVec::new();
    if let (Some(fmt), Some(dt)) = (fmt, dt_of(&cv)) {
        let mut sink = PartsSink::new();
        let _ = fmt.format(&dt).write_to_parts(&mut sink);
        if let Some((_, base)) = sink.frames.first_mut()
            && !base.is_empty()
        {
            let lit = std::mem::take(base);
            sink.parts.push(("literal", lit));
        }
        parts = sink.parts;
    }
    if parts.is_empty() {
        parts.push(("literal", iso_fallback(&cv)));
    }
    if tz_name > 0 {
        parts.push(("timeZoneName", tz_suffix(zone, tz_name, unix_ms)));
    }
    parts
}

fn fmt_date_str_with(
    fmt: Option<&LocaleDtfFmt>,
    zone: Option<core_utils::tz::TzIdx>,
    tz_name: u8,
    unix_ms: i64,
) -> CompactString {
    let off = core_utils::tz::tz_offset_zone(zone, unix_ms);
    let cv = core_utils::tz::civil_of(unix_ms, off);
    let mut out = CompactString::with_capacity(32);
    let mut wrote = false;
    if let (Some(fmt), Some(dt)) = (fmt, dt_of(&cv)) {
        let _ = fmt.format(&dt).write_to(&mut out);
        wrote = !out.is_empty();
    }
    if !wrote {
        out = iso_fallback(&cv);
    }
    if tz_name > 0 {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(tz_suffix(zone, tz_name, unix_ms).as_str());
    }
    out
}

fn fmt_date(st: &mut FmtState, unix_ms: i64) -> DtfParts {
    lazy_state!(st, dtf, build_dtf);
    let fmt = st.dtf.as_deref();
    fmt_date_with(fmt, st.zone, st.tz_name, unix_ms)
}

fn fmt_date_str(st: &mut FmtState, unix_ms: i64) -> CompactString {
    lazy_state!(st, dtf, build_dtf);
    let fmt = st.dtf.as_deref();
    fmt_date_str_with(fmt, st.zone, st.tz_name, unix_ms)
}

fn segment_words(s: &str, st: &mut FmtState) -> SmallVec<[(usize, usize, bool); 32]> {
    if st.wseg.is_none() {
        st.wseg = WSEG.with(|w| {
            let mut w = w.borrow_mut();
            if w.is_none() {
                *w = WordSegmenter::try_new_auto_unstable(provider(), Default::default())
                    .ok()
                    .map(Rc::new);
            }
            w.clone()
        });
    }
    let mut out: SmallVec<[(usize, usize, bool); 32]> = SmallVec::new();
    let Some(seg) = st.wseg.as_deref() else {
        return out;
    };
    let mut starts: SmallVec<[(usize, bool); 32]> = SmallVec::new();
    for (start, wt) in seg.as_borrowed().segment_str(s).iter_with_word_type() {
        starts.push((start, wt.is_word_like()));
    }
    for ((start, wl), (end, _)) in starts.iter().zip(starts.iter().skip(1)) {
        out.push((*start, *end, *wl));
    }
    out
}

fn segment_graphemes(s: &str, st: &mut FmtState) -> SmallVec<[(usize, usize); 64]> {
    if st.gseg.is_none() {
        st.gseg = GSEG.with(|g| {
            let mut g = g.borrow_mut();
            if g.is_none() {
                *g = GraphemeClusterSegmenter::try_new_unstable(provider())
                    .ok()
                    .map(Rc::new);
            }
            g.clone()
        });
    }
    let mut out: SmallVec<[(usize, usize); 64]> = SmallVec::new();
    let Some(seg) = st.gseg.as_deref() else {
        return out;
    };
    let starts: SmallVec<[usize; 64]> = seg.as_borrowed().segment_str(s).collect();
    for (start, end) in starts.iter().zip(starts.iter().skip(1)) {
        out.push((*start, *end));
    }
    out
}

const DT_FIELD_NAMES: &[(&str, &str)] = &[
    ("era", "era"),
    ("year", "year"),
    ("quarter", "quarter"),
    ("month", "month"),
    ("week", "week"),
    ("day", "day"),
    ("weekday", "weekday"),
    ("dayperiod", "day period"),
    ("hour", "hour"),
    ("minute", "minute"),
    ("second", "second"),
    ("zone", "time zone"),
];

fn build_dn(st: &FmtState) -> Option<DnCache> {
    let p = provider();
    let prefs = DisplayNamesPreferences::from(&icu_locale_of(st));
    match st.sub {
        1 => RegionDisplayNames::try_new_unstable(p, prefs, Default::default())
            .ok()
            .map(DnCache::Region),
        2 => ScriptDisplayNames::try_new_unstable(p, prefs, Default::default())
            .ok()
            .map(DnCache::Script),
        _ => LanguageDisplayNames::try_new_unstable(p, prefs, Default::default())
            .ok()
            .map(DnCache::Lang),
    }
}

fn display_name_of(st: &mut FmtState, code: &str) -> Option<std::borrow::Cow<'static, str>> {
    match st.sub {
        3 => CALENDARS
            .iter()
            .find(|c| **c == code)
            .map(|c| std::borrow::Cow::Borrowed(*c)),
        4 => DT_FIELD_NAMES
            .iter()
            .find(|(k, _)| *k == code)
            .map(|(_, v)| std::borrow::Cow::Borrowed(*v)),
        _ => {
            lazy_state!(st, dn, build_dn);
            match st.dn.as_deref()? {
                DnCache::Region(names) => {
                    let region = Region::try_from_str(code).ok()?;
                    names.of(region).map(|s| s.to_owned().into())
                }
                DnCache::Script(names) => {
                    let script = Script::try_from_str(code).ok()?;
                    names.of(script).map(|s| s.to_owned().into())
                }
                DnCache::Lang(names) => {
                    let lang = Language::try_from_str(code).ok()?;
                    names.of(lang).map(|s| s.to_owned().into())
                }
            }
        }
    }
}

const EN_DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const EN_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn date_part_of(
    unix_ms: i64,
    zone: Option<core_utils::tz::TzIdx>,
    windows: bool,
    which: u8,
) -> CompactString {
    let off = core_utils::tz::tz_offset_zone(zone, unix_ms);
    let c = core_utils::tz::civil_of(unix_ms, off);
    let mut out = CompactString::with_capacity(48);
    if which != 2 {
        out.push_str(EN_DAYS[c.dow as usize]);
        out.push(' ');
        out.push_str(EN_MONTHS[(c.month - 1) as usize]);
        out.push(' ');
        core_utils::math::push_int_into(&mut out, c.day as i64);
        out.push(' ');
        core_utils::math::push_int_into(&mut out, c.year as i64);
    }
    if which != 1 {
        if which != 2 {
            out.push(' ');
        }
        core_utils::math::push_int_padded_into(&mut out, c.hour as i64, 2);
        out.push(':');
        core_utils::math::push_int_padded_into(&mut out, c.minute as i64, 2);
        out.push(':');
        core_utils::math::push_int_padded_into(&mut out, c.second as i64, 2);
        out.push_str(" GMT");
        let a = off.unsigned_abs();
        out.push(if off < 0 { '-' } else { '+' });
        core_utils::math::push_int_padded_into(&mut out, (a / 60) as i64, 2);
        core_utils::math::push_int_padded_into(&mut out, (a % 60) as i64, 2);
        out.push_str(" (");
        core_utils::tz::zone_label_zone(zone, unix_ms, windows, &mut out);
        out.push(')');
    }
    out
}

fn this_ms_f<'js>(_c: &Ctx<'js>, this: &Value<'js>) -> rquickjs::Result<f64> {
    Ok(ms_f64_of_value(this))
}

fn dtf_out<'js>(c: &Ctx<'js>, this: &Value<'js>, date: &Value<'js>) -> rquickjs::Result<DtfParts> {
    let n = this_ms_f(c, date)?;
    if !n.is_finite() {
        return Err(rquickjs::Exception::throw_range(c, "Invalid time value"));
    }
    checked_mut(c, this, K_DTF, |st| fmt_date(st, n as i64))
}

fn dtf_str<'js>(c: &Ctx<'js>, this: &Value<'js>, date: &Value<'js>) -> rquickjs::Result<CompactString> {
    let n = this_ms_f(c, date)?;
    if !n.is_finite() {
        return Err(rquickjs::Exception::throw_range(c, "Invalid time value"));
    }
    checked_mut(c, this, K_DTF, |st| fmt_date_str(st, n as i64))
}

fn locale_date_string(n: f64, with_time: bool, time_only: bool) -> CompactString {
    let opts_key = (with_time as u8) | ((time_only as u8) << 1);
    LOCALE_DTF.with(|c| {
        let mut cell = c.borrow_mut();
        let hit = with_prof(|p| {
            cell.as_ref().is_some_and(|(l, k, z, _)| {
                l.as_str() == p.prof().locale.as_str() && *k == opts_key && *z == p.tz_zone
            })
        });
        if !hit {
            let (locale, zone) = with_prof(|p| (p.prof().locale.clone(), p.tz_zone));
            let mut st = FmtState::dtf(locale.clone());
            st.zone = zone;
            if !time_only {
                st.year = 2;
                st.month = 2;
                st.day = 2;
            }
            if with_time || time_only {
                st.hour = 2;
                st.minute = 2;
                st.second = 2;
            }
            match build_dtf(&st) {
                Some(f) => *cell = Some((locale, opts_key, zone, Some(Rc::new(f)))),
                None => {
                    *cell = None;
                    return fmt_date_str_with(None, zone, 0, n as i64);
                }
            }
        }
        match cell.as_ref() {
            Some((_, _, zone, fmt)) => fmt_date_str_with(fmt.as_deref(), *zone, 0, n as i64),
            None => {
                let zone = with_prof(|p| p.tz_zone);
                fmt_date_str_with(None, zone, 0, n as i64)
            }
        }
    })
}

fn install_date_time_zone<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let orig_date: Function = ctx.eval("Date")?;
    let tz_off_fn = Function::new(ctx.clone(), |ms: f64| -> f64 {
        let zone = with_prof(|p| p.tz_zone);
        core_utils::tz::tz_offset_zone(zone, ms as i64) as f64
    })?;
    let globals = ctx.globals();
    let date_wrap: Function = ctx.eval(
        "(function (O, OFF) {\
            function Date(a, b, c, d, e, f, g) {\
                var n = arguments.length;\
                if (n === 0) { return new O(); }\
                if (n === 1) { return new O(a); }\
                var y = a;\
                if (y >= 0 && y <= 99) { y += 1900; }\
                var mo = b + 1;\
                if (mo < 1) { mo = 1; }\
                if (mo > 12) { mo = 12; }\
                var dim = new O(Date.UTC(y, mo, 0)).getUTCDate();\
                var da = Math.min(Math.max(c, 1), dim);\
                var guess = Date.UTC(y, mo - 1, da, d || 0, e || 0, f || 0, g || 0);\
                var off1 = OFF(guess);\
                var t1 = guess - off1 * 60000;\
                var off2 = OFF(t1);\
                return new O(off2 === off1 ? t1 : guess - off2 * 60000);\
            }\
            Date.now = O.now;\
            Date.parse = O.parse;\
            Date.UTC = O.UTC;\
            Date.prototype = O.prototype;\
            return Date;\
        })",
    )?;
    let new_date: Function = date_wrap.call((orig_date, tz_off_fn))?;
    globals.set("Date", new_date.clone())?;
    let date_proto: Object = ctx.eval("Date.prototype")?;
    date_proto.prop(
        "constructor",
        rquickjs::object::Property::from(new_date)
            .writable()
            .configurable(),
    )?;


    let build_getter =
        |name: &'static str, pick: fn(&core_utils::tz::Civil) -> f64| -> rquickjs::Result<()> {
            let f = Function::new(
                ctx.clone(),
                move |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                    let n: f64 = this_ms_f(&c, &this.0)?;
                    if !n.is_finite() {
                        return Ok(Value::new_number(c, f64::NAN));
                    }
                    let zone = with_prof(|p| p.tz_zone);
                    let off = core_utils::tz::tz_offset_zone(zone, n as i64);
                    let cv = core_utils::tz::civil_of(n as i64, off);
                    Ok(Value::new_number(c, pick(&cv)))
                },
            )?;
            crate::webidl::define_method(ctx, &date_proto, name, f)?;
            Ok(())
        };

    build_getter("getFullYear", |p| p.year as f64)?;
    build_getter("getMonth", |p| (p.month - 1) as f64)?;
    build_getter("getDate", |p| p.day as f64)?;
    build_getter("getDay", |p| p.dow as f64)?;
    build_getter("getHours", |p| p.hour as f64)?;
    build_getter("getMinutes", |p| p.minute as f64)?;
    build_getter("getSeconds", |p| p.second as f64)?;

    let get_ms_offset = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let n: f64 = this_ms_f(&c, &this.0)?;
            if !n.is_finite() {
                return Ok(Value::new_number(c, f64::NAN));
            }
            let zone = with_prof(|p| p.tz_zone);
            let off = core_utils::tz::tz_offset_zone(zone, n as i64);
            Ok(Value::new_number(c, -(off as f64)))
        },
    )?;
    crate::webidl::define_method(ctx, &date_proto, "getTimezoneOffset", get_ms_offset)?;

    let date_str_method =
        |name: &'static str, which: u8| -> rquickjs::Result<()> {
            let f = Function::new(
                ctx.clone(),
                move |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                    let n: f64 = this_ms_f(&c, &this.0)?;
                    if !n.is_finite() {
                        return "Invalid Date".into_js(&c);
                    }
                    let zone = with_prof(|p| p.tz_zone);
                    date_part_of(n as i64, zone, is_windows(), which).into_js(&c)
                },
            )?;
            crate::webidl::define_method(ctx, &date_proto, name, f)
        };
    date_str_method("toString", 0)?;
    date_str_method("toDateString", 1)?;
    date_str_method("toTimeString", 2)?;

    let locale_str_method =
        |name: &'static str, with_time: bool, time_only: bool| -> rquickjs::Result<()> {
            let f = Function::new(
                ctx.clone(),
                move |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                    let n: f64 = this_ms_f(&c, &this.0)?;
                    if !n.is_finite() {
                        return "Invalid Date".into_js(&c);
                    }
                    let s = locale_date_string(n, with_time, time_only);
                    s.as_str().into_js(&c)
                },
            )?;
            crate::webidl::define_method(ctx, &date_proto, name, f)
        };
    locale_str_method("toLocaleString", true, false)?;
    locale_str_method("toLocaleDateString", false, false)?;
    locale_str_method("toLocaleTimeString", false, true)?;
    Ok(())
}

fn dtf_parse_opts(o: &Object<'_>, locale: CompactString) -> FmtState {
    let mut st = FmtState::dtf(locale);
    let mut has_date = false;
    let mut has_time = false;
    if let Some(w) = option_str(o, "weekday") {
        st.weekday = style_of(w.as_str()).unwrap_or(1) + 1;
        has_date = true;
    }
    if let Some(e) = option_str(o, "era") {
        st.era = style_of(e.as_str()).unwrap_or(1) + 1;
        has_date = true;
    }
    if let Some(y) = option_str(o, "year") {
        st.year = opt_code(DIGIT2, y.as_str()).unwrap_or(2);
        has_date = true;
    }
    if let Some(m) = option_str(o, "month") {
        st.month = match m.as_str() {
            "2-digit" => 1,
            "numeric" => 2,
            "narrow" => 3,
            "short" => 4,
            "long" => 5,
            _ => 0,
        };
        has_date = true;
    }
    if let Some(d) = option_str(o, "day") {
        st.day = opt_code(DIGIT2, d.as_str()).unwrap_or(2);
        has_date = true;
    }
    if let Some(h) = option_str(o, "hour") {
        st.hour = opt_code(DIGIT2, h.as_str()).unwrap_or(2);
        has_time = true;
    }
    if let Some(m) = option_str(o, "minute") {
        st.minute = opt_code(DIGIT2, m.as_str()).unwrap_or(2);
        has_time = true;
    }
    if let Some(s) = option_str(o, "second") {
        st.second = opt_code(DIGIT2, s.as_str()).unwrap_or(2);
        has_time = true;
    }
    if let Some(t) = option_str(o, "timeZoneName") {
        st.tz_name = style_of(t.as_str()).unwrap_or(1) + 1;
    }
    if let Some(h12) = option_bool(o, "hour12") {
        st.hour12 = h12;
        st.hour_cycle = if h12 { 2 } else { 3 };
    } else if let Some(hc) = option_str(o, "hourCycle") {
        st.hour_cycle = HOUR_CYCLES
            .iter()
            .position(|h| *h == hc.as_str())
            .map_or(2, |i| i as u8 + 1);
        st.hour12 = st.hour_cycle == 1 || st.hour_cycle == 2;
    }
    if !has_date && !has_time {
        st.year = 2;
        st.month = 2;
        st.day = 2;
    }
    if let Some(cal) = option_str(o, "calendar") {
        st.calendar = cal;
    }
    st
}

fn install_datetime_format<'js>(ctx: &Ctx<'js>, intl: &Object<'js>) -> rquickjs::Result<()> {
    let proto = intl_class!(
        ctx,
        intl,
        "DateTimeFormat",
        |c, locale, opts_obj| {
            let mut st = dtf_parse_opts(&opts_obj, locale);
            st.tz = option_str(&opts_obj, "timeZone")
                .unwrap_or_else(|| with_prof(|p| p.prof().tz.clone()));
            st.zone = core_utils::tz::zone_of(st.tz.as_str());
            if st.zone.is_none() && st.tz.as_str() != "UTC" {
                return Err(rquickjs::Exception::throw_range(&c, "Invalid timeZone"));
            }
            if let Some(nu) = option_str(&opts_obj, "numberingSystem") {
                st.numbering = nu;
            }
            st
        },
        supported
    );

    install_resolved(ctx, &proto, K_DTF, |st, o| {
        o.set("calendar", st.calendar.as_str())?;
        o.set("numberingSystem", st.numbering.as_str())?;
        o.set("timeZone", st.tz.as_str())?;
        for &(key, get, name) in DTF_RESOLVED {
            let v = get(st);
            if v > 0 {
                o.set(key, name(v))?;
            }
        }
        if st.hour_cycle != 0 {
            let hcs = HOUR_CYCLES
                .get(st.hour_cycle as usize - 1)
                .copied()
                .unwrap_or("h23");
            o.set("hourCycle", hcs)?;
            o.set("hour12", st.hour12)?;
        }
        Ok(())
    })?;

    let format = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         date: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            let date = arg(&c, date);
            dtf_str(&c, &this.0, &date)?.as_str().into_js(&c)
        },
    )?;
    crate::webidl::define_method(ctx, &proto, "format", format)?;

    let format_to_parts = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         date: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            let date = arg(&c, date);
            let parts = dtf_out(&c, &this.0, &date)?;
            let arr = rquickjs::Array::new(c.clone())?;
            for (i, (ty, v)) in parts.iter().enumerate() {
                let o = Object::new(c.clone())?;
                o.set("type", *ty)?;
                o.set("value", v.as_str())?;
                arr.set(i, o)?;
            }
            Ok(arr.into_value())
        },
    )?;
    crate::webidl::define_method(ctx, &proto, "formatToParts", format_to_parts)?;

    Ok(())
}

fn install_number_format<'js>(ctx: &Ctx<'js>, intl: &Object<'js>) -> rquickjs::Result<()> {
    let proto = intl_class!(
        ctx,
        intl,
        "NumberFormat",
        |c, locale, opts_obj| {
            let mut st = FmtState::nf(locale);
            if let Some(style) = option_str(&opts_obj, "style") {
                match style.as_str() {
                    "decimal" => st.style = 0,
                    "percent" => st.style = 2,
                    "currency" => {
                        st.style = 1;
                        let cur = option_str(&opts_obj, "currency").unwrap_or_default();
                        if cur.len() != 3 || !cur.bytes().all(|b| b.is_ascii_alphabetic()) {
                            return Err(rquickjs::Exception::throw_message(
                                &c,
                                "currency is not a well-formed currency code",
                            ));
                        }
                        st.currency = core_utils::ascii_upper_compact(cur.as_str());
                        st.min_frac = 2;
                        st.max_frac = 2;
                        if let Some(cd) = option_str(&opts_obj, "currencyDisplay") {
                            st.currency_display =
                                opt_code(CURRENCY_DISPLAY, cd.as_str()).unwrap_or(0);
                        }
                    }
                    _ => {
                        return Err(rquickjs::Exception::throw_message(
                            &c,
                            "Invalid value for option style",
                        ));
                    }
                }
            }
            if let Some(m) = option_str(&opts_obj, "notation")
                && m.as_str() != "standard"
            {
                return Err(rquickjs::Exception::throw_range(
                    &c,
                    "notation is not supported",
                ));
            }
            if let Some(sd) = option_str(&opts_obj, "signDisplay") {
                st.sign_display = opt_code(SIGN_DISPLAY, sd.as_str()).unwrap_or(0);
            }
            if let Some(g) = option_bool(&opts_obj, "useGrouping") {
                st.grouping = g;
            }
            if let Some(n) = option_str(&opts_obj, "numberingSystem") {
                st.numbering = n;
            }
            if let Some(v) = option_str(&opts_obj, "minimumIntegerDigits") {
                st.min_int = v.parse().unwrap_or(1);
            }
            if let Some(v) = option_str(&opts_obj, "minimumFractionDigits") {
                st.min_frac = v.parse().unwrap_or(0);
            }
            if let Some(v) = option_str(&opts_obj, "maximumFractionDigits") {
                st.max_frac = v.parse().unwrap_or(3);
            }
            st
        },
        supported
    );

    install_resolved(ctx, &proto, K_NF, |st, o| {
        o.set("numberingSystem", st.numbering.as_str())?;
        o.set(
            "style",
            match st.style {
                1 => "currency",
                2 => "percent",
                _ => "decimal",
            },
        )?;
        if st.style == 1 {
            o.set("currency", st.currency.as_str())?;
            o.set(
                "currencyDisplay",
                opt_name(CURRENCY_DISPLAY, st.currency_display),
            )?;
        }
        o.set("minimumIntegerDigits", st.min_int as u32)?;
        o.set("minimumFractionDigits", st.min_frac as u32)?;
        o.set("maximumFractionDigits", st.max_frac as u32)?;
        o.set("useGrouping", st.grouping)?;
        o.set("notation", "standard")?;
        o.set("signDisplay", opt_name(SIGN_DISPLAY, st.sign_display))?;
        o.set("roundingMode", "halfExpand")?;
        Ok(())
    })?;

    let format = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         n: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            let n = arg(&c, n);
            let x = ms_f64_of_value(&n);
            let out = checked_mut(&c, &this.0, K_NF, |st| format_number(x, st))?
                .map_err(|_| rquickjs::Exception::throw_range(&c, "Intl.NumberFormat.format failed"))?;
            out.as_str().into_js(&c)
        },
    )?;
    crate::webidl::define_method(ctx, &proto, "format", format)?;

    Ok(())
}

fn install_collator<'js>(ctx: &Ctx<'js>, intl: &Object<'js>) -> rquickjs::Result<()> {
    let proto = intl_class!(
        ctx,
        intl,
        "Collator",
        |c, locale, opts_obj| {
            let mut st = FmtState::coll(locale);
            if let Some(s) = option_str(&opts_obj, "sensitivity") {
                st.sensitivity = opt_code(SENSITIVITY, s.as_str()).unwrap_or(3);
            }
            if let Some(v) = option_bool(&opts_obj, "ignorePunctuation") {
                st.ignore_punct = v;
            }
            if let Some(v) = option_bool(&opts_obj, "numeric") {
                st.coll_numeric = v;
            }
            if let Some(v) = option_str(&opts_obj, "caseFirst") {
                st.case_first = opt_code(CASE_FIRST, v.as_str()).unwrap_or(0);
            }
            st
        },
        supported
    );

    let compare = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         a: rquickjs::function::Opt<Value<'js>>,
         b: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            let a = arg(&c, a);
            let b = arg(&c, b);
            let ca = cstr_of(&a);
            let cb = cstr_of(&b);
            let sa: &str = ca.as_ref().map(|cs| cs.as_str()).unwrap_or("");
            let sb: &str = cb.as_ref().map(|cs| cs.as_str()).unwrap_or("");
            let r = checked_mut(&c, &this.0, K_COLL, |st| coll_compare(sa, sb, st))?;
            Ok(Value::new_number(c, r as f64))
        },
    )?;
    crate::webidl::define_method(ctx, &proto, "compare", compare)?;

    install_resolved(ctx, &proto, K_COLL, |st, o| {
        o.set("usage", "sort")?;
        o.set("sensitivity", opt_name(SENSITIVITY, st.sensitivity))?;
        o.set("ignorePunctuation", st.ignore_punct)?;
        o.set("collation", "default")?;
        o.set("numeric", st.coll_numeric)?;
        o.set(
            "caseFirst",
            match opt_name(CASE_FIRST, st.case_first) {
                "auto" => "false",
                other => other,
            },
        )?;
        Ok(())
    })?;

    Ok(())
}

fn install_list_format<'js>(ctx: &Ctx<'js>, intl: &Object<'js>) -> rquickjs::Result<()> {
    let proto = intl_class!(ctx, intl, "ListFormat", |c, locale, opts_obj| {
        let mut st = FmtState::plain(K_LF, locale);
        if let Some(t) = option_str(&opts_obj, "type") {
            st.sub = if t == "disjunction" { 1 } else { 0 };
        }
        if let Some(s) = option_str(&opts_obj, "style") {
            st.style = style_of(s.as_str()).unwrap_or(0);
        }
        st
    });

    let format = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         list: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            let list = arg(&c, list);
            let items: SmallVec<[CompactString; 8]> = list
                .as_array()
                .map(|arr| {
                    arr.iter::<rquickjs::String>()
                        .filter_map(|x| x.ok())
                        .filter_map(|x| x.to_cstring().ok())
                        .map(|cs| CompactString::new(cs.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            let s = checked_mut(&c, &this.0, K_LF, |st| list_format(&items, st))?;
            s.as_str().into_js(&c)
        },
    )?;
    crate::webidl::define_method(ctx, &proto, "format", format)?;

    install_resolved(ctx, &proto, K_LF, |st, o| {
        o.set(
            "type",
            if st.sub == 0 {
                "conjunction"
            } else {
                "disjunction"
            },
        )?;
        o.set("style", style_name(st.style))?;
        Ok(())
    })?;

    Ok(())
}

fn install_plural_rules<'js>(ctx: &Ctx<'js>, intl: &Object<'js>) -> rquickjs::Result<()> {
    let proto = intl_class!(ctx, intl, "PluralRules", |c, locale, opts_obj| {
        let mut st = FmtState::plain(K_PLURAL, locale);
        if let Some(t) = option_str(&opts_obj, "type") {
            st.sub = if t == "ordinal" { 1 } else { 0 };
        }
        st
    });

    let select = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         n: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            let n = arg(&c, n);
            let x = ms_f64_of_value(&n);
            if x.is_nan() {
                return "other".into_js(&c);
            }
            let cat = checked_mut(&c, &this.0, K_PLURAL, |st| plural_category(st, x))?;
            cat.into_js(&c)
        },
    )?;
    crate::webidl::define_method(ctx, &proto, "select", select)?;

    install_resolved(ctx, &proto, K_PLURAL, |st, o| {
        o.set("type", if st.sub == 0 { "cardinal" } else { "ordinal" })?;
        o.set("minimumIntegerDigits", 1u32)?;
        Ok(())
    })?;

    Ok(())
}

fn install_relative_time_format<'js>(ctx: &Ctx<'js>, intl: &Object<'js>) -> rquickjs::Result<()> {
    let proto = intl_class!(ctx, intl, "RelativeTimeFormat", |c, locale, opts_obj| {
        let mut st = FmtState::plain(K_RTF, locale);
        if let Some(v) = option_str(&opts_obj, "numeric") {
            st.sub = if v == "auto" { 1 } else { 0 };
        }
        if let Some(s) = option_str(&opts_obj, "style") {
            st.style = style_of(s.as_str()).unwrap_or(0);
        }
        st
    });

    let format = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         n: rquickjs::function::Opt<Value<'js>>,
         unit: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            let n = arg(&c, n);
            let unit = arg(&c, unit);
            let x = ms_f64_of_value(&n);
            let uc = cstr_of(&unit);
            let u: &str = uc.as_ref().map(|cs| cs.as_str()).unwrap_or("");
            if !RTF_UNITS.contains(&u) {
                let mut msg = CompactString::const_new("invalid unit: ");
                msg.push_str(u);
                return Err(rquickjs::Exception::throw_range(&c, msg.as_str()));
            }
            if x.is_nan() {
                return Err(rquickjs::Exception::throw_range(&c, "Invalid value"));
            }
            let s = checked_mut(&c, &this.0, K_RTF, |st| rtf_format(x, st, u))?;
            s.as_str().into_js(&c)
        },
    )?;
    crate::webidl::define_method(ctx, &proto, "format", format)?;

    install_resolved(ctx, &proto, K_RTF, |st, o| {
        o.set("numeric", if st.sub == 0 { "always" } else { "auto" })?;
        o.set("style", style_name(st.style))?;
        Ok(())
    })?;

    Ok(())
}

fn install_display_names<'js>(ctx: &Ctx<'js>, intl: &Object<'js>) -> rquickjs::Result<()> {
    let proto = intl_class!(ctx, intl, "DisplayNames", |c, locale, opts_obj| {
        let mut st = FmtState::plain(K_DN, locale);
        st.sub = match option_str(&opts_obj, "type").as_deref() {
            Some("language") => 0u8,
            Some("region") => 1,
            Some("script") => 2,
            Some("calendar") => 3,
            Some("dateTimeField") => 4,
            _ => {
                return Err(rquickjs::Exception::throw_type(
                    &c,
                    "Failed to construct 'DisplayNames': member type is required and must be one of \"language\", \"region\", \"script\", \"calendar\", \"dateTimeField\".",
                ));
            }
        };
        st
    });

    let of = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         code: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            let code = code.0.ok_or_else(|| {
                rquickjs::Exception::throw_type(
                    &c,
                    "Failed to execute 'of' on 'DisplayNames': The provided value is not of type 'string'.",
                )
            })?;
            let scc = cstr_of(&code);
            let s: &str = scc.as_ref().map(|cs| cs.as_str()).unwrap_or("");
            let found = checked_mut(&c, &this.0, K_DN, |st| display_name_of(st, s))?;
            match found {
                Some(v) => v.as_ref().into_js(&c),
                None => Ok(Value::new_undefined(c)),
            }
        },
    )?;
    crate::webidl::define_method(ctx, &proto, "of", of)?;

    Ok(())
}

fn locale_tag(st: &FmtState) -> String {
    let mut out = String::with_capacity(16);
    out.push_str(st.language.as_str());
    if !st.script.is_empty() {
        out.push('-');
        out.push_str(st.script.as_str());
    }
    if !st.region.is_empty() {
        out.push('-');
        out.push_str(st.region.as_str());
    }
    out
}

fn state_of_loc(loc: &Locale) -> FmtState {
    let language = CompactString::new(loc.id.language.as_str());
    let script = loc
        .id
        .script
        .map(|s| CompactString::new(s.as_str()))
        .unwrap_or_default();
    let region = loc
        .id
        .region
        .map(|r| CompactString::new(r.as_str()))
        .unwrap_or_default();
    FmtState::locale_state(language, script, region)
}

thread_local! {
    static LOCALE_EXP: RefCell<Option<Rc<icu_locale::LocaleExpander>>> = const { RefCell::new(None) };
}

fn expand_locale(st: &FmtState, maximize: bool) -> FmtState {
    let tag = locale_tag(st);
    let mut loc = locale_of(&tag).unwrap_or_else(und);
    LOCALE_EXP.with(|c| {
        let mut cell = c.borrow_mut();
        if cell.is_none() {
            *cell = icu_locale::LocaleExpander::try_new_extended_unstable(provider())
                .ok()
                .map(Rc::new);
        }
        if let Some(exp) = cell.as_ref() {
            if maximize {
                let _ = exp.maximize(&mut loc.id);
            } else {
                let _ = exp.minimize(&mut loc.id);
            }
        }
    });
    state_of_loc(&loc)
}

fn install_locale_class<'js>(ctx: &Ctx<'js>, intl: &Object<'js>) -> rquickjs::Result<()> {
    let proto = Object::new(ctx.clone())?;
    class_tag(ctx, &proto, "Intl.Locale")?;
    let ctor_proto = proto.clone();
    let ctor = Function::new(
        ctx.clone(),
        move |c: Ctx<'js>, tag: rquickjs::function::Opt<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            touch::touch_log_record(ApiKey::INTL);
            let tag = tag.0.ok_or_else(|| {
                rquickjs::Exception::throw_type(
                    &c,
                    "Failed to construct 'Locale': parameter 1 is not of type 'string'.",
                )
            })?;
            let tcc = cstr_of(&tag);
            let ts: &str = tcc.as_ref().map(|cs| cs.as_str()).unwrap_or("");
            let can = canonical_tag(ts).ok_or_else(|| throw_range_invalid(&c, ts))?;
            let loc = locale_of(can.as_str()).unwrap_or_else(und);
            let st = state_of_loc(&loc);
            make_instance(&c, &ctor_proto, "Intl.Locale", st)
        },
    )?
    .with_constructor(true);
    crate::stackfmt::set_fn_name(ctx, &ctor, "Locale")?;
    crate::stackfmt::set_fn_len(ctx, &ctor, 1)?;
    link_ctor(ctx, &ctor, &proto)?;

    let locale_tag_of = |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
        checked(&c, &this.0, K_LOCALE, locale_tag)?.into_js(&c)
    };
    let to_string = Function::new(ctx.clone(), locale_tag_of)?;
    crate::webidl::define_method(ctx, &proto, "toString", to_string)?;

    let base_name = Function::new(ctx.clone(), locale_tag_of)?;
    crate::webidl::named_accessor(ctx, &proto, "baseName", base_name, None)?;

    let locale_part = |part: fn(&FmtState) -> &str| {
        move |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let v = checked(&c, &this.0, K_LOCALE, |st| {
                let p = part(st);
                (!p.is_empty()).then(|| CompactString::new(p))
            })?;
            match v {
                Some(cs) => cs.as_str().into_js(&c),
                None => Ok(Value::new_undefined(c)),
            }
        }
    };
    let language_get = Function::new(ctx.clone(), locale_part(|st| st.language.as_str()))?;
    crate::webidl::named_accessor(ctx, &proto, "language", language_get, None)?;

    let region_get = Function::new(ctx.clone(), locale_part(|st| st.region.as_str()))?;
    crate::webidl::named_accessor(ctx, &proto, "region", region_get, None)?;

    let script_get = Function::new(ctx.clone(), locale_part(|st| st.script.as_str()))?;
    crate::webidl::named_accessor(ctx, &proto, "script", script_get, None)?;

    let expand_native = |fill: bool| {
        let proto_ref = proto.clone();
        move |c: Ctx<'js>, this: This<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            let st2 = checked(&c, &this.0, K_LOCALE, move |st| expand_locale(st, fill))?;
            make_instance(&c, &proto_ref, "Intl.Locale", st2)
        }
    };
    let maximize = Function::new(ctx.clone(), expand_native(true))?;
    crate::webidl::define_method(ctx, &proto, "maximize", maximize)?;

    let minimize = Function::new(ctx.clone(), expand_native(false))?;
    crate::webidl::define_method(ctx, &proto, "minimize", minimize)?;

    export_prop(intl, "Locale", ctor)?;
    Ok(())
}

fn seg_push<'js>(
    arr: &rquickjs::Array<'js>,
    c: &Ctx<'js>,
    seg: &str,
    word_like: bool,
    idx: &mut usize,
    i: &mut usize,
) -> rquickjs::Result<()> {
    let o = Object::new(c.clone())?;
    o.set("segment", seg)?;
    o.set("index", *idx)?;
    o.set("isWordLike", word_like)?;
    *idx += seg.chars().map(|ch| ch.len_utf16()).sum::<usize>();
    arr.set(*i, o)?;
    *i += 1;
    Ok(())
}

fn install_segmenter<'js>(ctx: &Ctx<'js>, intl: &Object<'js>) -> rquickjs::Result<()> {
    let proto = intl_class!(ctx, intl, "Segmenter", |c, locale, opts_obj| {
        let mut st = FmtState::plain(K_SEG, locale);
        st.sub = match option_str(&opts_obj, "granularity").as_deref() {
            Some("grapheme") | None => 0u8,
            Some("word") => 1,
            _ => {
                return Err(rquickjs::Exception::throw_range(
                    &c,
                    "granularity is not one of grapheme, word",
                ));
            }
        };
        st
    });

    let segment = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         this: This<Value<'js>>,
         text: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            let text = arg(&c, text);
            let sc = cstr_of(&text);
            let s: &str = sc.as_ref().map(|cs| cs.as_str()).unwrap_or("");
            let arr = rquickjs::Array::new(c.clone())?;
            let mut idx = 0usize;
            let mut i = 0usize;
            let segs = checked_mut(&c, &this.0, K_SEG, |st| {
                if st.sub == 1 {
                    SegOut::Words(segment_words(s, st))
                } else {
                    SegOut::Graphemes(segment_graphemes(s, st))
                }
            })?;
            match segs {
                SegOut::Words(v) => {
                    for (a, b, word_like) in v {
                        seg_push(&arr, &c, &s[a..b], word_like, &mut idx, &mut i)?;
                    }
                }
                SegOut::Graphemes(v) => {
                    for (a, b) in v {
                        let seg = &s[a..b];
                        seg_push(
                            &arr,
                            &c,
                            seg,
                            seg.chars().any(|ch| ch.is_alphanumeric()),
                            &mut idx,
                            &mut i,
                        )?;
                    }
                }
            }
            crate::webidl::iterable_of_array(&c, &arr)
        },
    )?;
    crate::webidl::define_method(ctx, &proto, "segment", segment)?;

    install_resolved(ctx, &proto, K_SEG, |st, o| {
        o.set("granularity", if st.sub == 0 { "grapheme" } else { "word" })?;
        Ok(())
    })?;

    Ok(())
}

const CURRENCY_CODES: &[&str] = &[
    "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT",
    "BGN", "BHD", "BIF", "BMD", "BND", "BOB", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD", "CAD",
    "CDF", "CHF", "CLP", "CNY", "COP", "CRC", "CUP", "CVE", "CZK", "DJF", "DKK", "DOP", "DZD",
    "EGP", "ERN", "ETB", "EUR", "FJD", "FKP", "GBP", "GEL", "GHS", "GIP", "GMD", "GNF", "GTQ",
    "GYD", "HKD", "HNL", "HRK", "HTG", "HUF", "IDR", "ILS", "INR", "IQD", "IRR", "ISK", "JMD",
    "JOD", "JPY", "KES", "KGS", "KHR", "KMF", "KPW", "KRW", "KWD", "KYD", "KZT", "LAK", "LBP",
    "LKR", "LRD", "LSL", "LYD", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT", "MOP", "MRU", "MUR",
    "MVR", "MWK", "MXN", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK", "NPR", "NZD", "OMR", "PAB",
    "PEN", "PGK", "PHP", "PKR", "PLN", "PYG", "QAR", "RON", "RSD", "RUB", "RWF", "SAR", "SBD",
    "SCR", "SDG", "SEK", "SGD", "SHP", "SLL", "SOS", "SRD", "SSP", "STN", "SVC", "SYP", "SZL",
    "THB", "TJS", "TMT", "TND", "TOP", "TRY", "TTD", "TWD", "TZS", "UAH", "UGX", "USD", "UYU",
    "UZS", "VES", "VND", "VUV", "WST", "XAF", "XCD", "XOF", "XPF", "YER", "ZAR", "ZMW", "ZWL",
];

const CALENDARS: &[&str] = &[
    "buddhist",
    "chinese",
    "coptic",
    "dangi",
    "ethioaa",
    "ethiopic",
    "gregory",
    "hebrew",
    "indian",
    "islamic",
    "islamic-civil",
    "islamic-tbla",
    "islamic-umalqura",
    "iso8601",
    "japanese",
    "persian",
    "roc",
];

const COLLATIONS: &[&str] = &[
    "big5han", "compat", "dict", "direct", "ducet", "emoji", "eor", "gb2312", "phonebk", "pinyin",
    "reformed", "search", "searchjl", "standard", "stroke", "trad", "unihan", "zhuyin",
];

const NUMBERINGS: &[&str] = &[
    "adlm", "ahom", "arab", "arabext", "bali", "beng", "bhks", "brah", "cakm", "cham", "deva",
    "diak", "fullwide", "gong", "gonm", "gujr", "guru", "hanidec", "hmng", "hmnp", "java", "kali",
    "khmr", "knda", "lana", "lanatham", "laoo", "latn", "lepc", "limb", "mathbold", "mathdbl",
    "mathmono", "mathsanb", "mathsans", "mlym", "modi", "mong", "mroo", "mtei", "mymr", "mymrshan",
    "mymrtlng", "newa", "nkoo", "olck", "orya", "osma", "rohg", "saur", "segment", "shrd", "sind",
    "sinh", "sund", "takr", "talu", "tamldec", "telu", "thai", "tibt", "tirh", "vaii", "wara",
    "wcho",
];

const HOUR_CYCLES: &[&str] = &["h11", "h12", "h23", "h24"];

const CASE_FIRSTS: &[&str] = &["false", "lower", "upper"];

const UNITS: &[&str] = &[
    "acre",
    "bit",
    "byte",
    "celsius",
    "centiliter",
    "centimeter",
    "day",
    "degree",
    "fahrenheit",
    "fluid-ounce",
    "foot",
    "gallon",
    "gigabit",
    "gigabyte",
    "gram",
    "hectare",
    "hectoliter",
    "hectopascal",
    "hertz",
    "hour",
    "inch",
    "kilobit",
    "kilobyte",
    "kilogram",
    "kilometer",
    "kilowatt",
    "liter",
    "megabit",
    "megabyte",
    "meter",
    "microsecond",
    "mile",
    "mile-scandinavian",
    "milliliter",
    "millimeter",
    "millisecond",
    "milliwatt",
    "minute",
    "month",
    "nanosecond",
    "ounce",
    "percent",
    "petabyte",
    "pound",
    "second",
    "stone",
    "terabit",
    "terabyte",
    "week",
    "yard",
    "year",
];

fn install_supported_values<'js>(ctx: &Ctx<'js>, intl: &Object<'js>) -> rquickjs::Result<()> {
    let f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>, kind: Value<'js>| -> rquickjs::Result<Value<'js>> {
            let kc = cstr_of(&kind);
            let k: &str = kc.as_ref().map(|cs| cs.as_str()).unwrap_or("");
            if k == "timeZone" {
                let arr = rquickjs::Array::new(c.clone())?;
                for (i, name) in core_utils::tz::all_zone_names().enumerate() {
                    arr.set(i, name)?;
                }
                return Ok(arr.into_value());
            }
            let list: &[&str] = match k {
                "calendar" => CALENDARS,
                "collation" => COLLATIONS,
                "numberingSystem" => NUMBERINGS,
                "hourCycle" => HOUR_CYCLES,
                "caseFirst" => CASE_FIRSTS,
                "unit" => UNITS,
                "currency" => CURRENCY_CODES,
                _ => {
                    let mut msg = CompactString::const_new("invalid key: ");
                    msg.push_str(k);
                    return Err(rquickjs::Exception::throw_range(&c, msg.as_str()));
                }
            };
            let arr = rquickjs::Array::new(c.clone())?;
            for (i, v) in list.iter().enumerate() {
                arr.set(i, *v)?;
            }
            Ok(arr.into_value())
        },
    )?;
    crate::stackfmt::set_fn_name(ctx, &f, "supportedValuesOf")?;
    crate::stackfmt::set_fn_len(ctx, &f, 1)?;
    export_prop(intl, "supportedValuesOf", f)?;
    Ok(())
}

fn install_canonical<'js>(ctx: &Ctx<'js>, intl: &Object<'js>) -> rquickjs::Result<()> {
    let f = Function::new(
        ctx.clone(),
        |c: Ctx<'js>,
         locales: rquickjs::function::Opt<Value<'js>>|
         -> rquickjs::Result<Value<'js>> {
            let locales = arg(&c, locales);
            supported_locales(&c, &locales)
        },
    )?;
    crate::stackfmt::set_fn_name(ctx, &f, "getCanonicalLocales")?;
    crate::stackfmt::set_fn_len(ctx, &f, 1)?;
    export_prop(intl, "getCanonicalLocales", f)?;
    Ok(())
}

pub(crate) fn install<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let _ = globals.remove("Intl");
    let intl = Object::new(ctx.clone())?;
    class_tag(ctx, &intl, "Intl")?;

    install_canonical(ctx, &intl)?;
    install_collator(ctx, &intl)?;
    install_datetime_format(ctx, &intl)?;
    install_display_names(ctx, &intl)?;
    install_list_format(ctx, &intl)?;
    install_locale_class(ctx, &intl)?;
    install_number_format(ctx, &intl)?;
    install_plural_rules(ctx, &intl)?;
    install_relative_time_format(ctx, &intl)?;
    install_segmenter(ctx, &intl)?;
    install_supported_values(ctx, &intl)?;

    globals.prop(
        "Intl",
        rquickjs::object::Property::from(intl)
            .writable()
            .configurable(),
    )?;
    install_date_time_zone(ctx)?;
    Ok(())
}

pub(crate) fn teardown_intl<'js>(ctx: &Ctx<'js>) {
    let Ok(intl): rquickjs::Result<Object> = ctx.globals().get("Intl") else {
        return;
    };
    for name in [
        "Collator",
        "DateTimeFormat",
        "DisplayNames",
        "ListFormat",
        "Locale",
        "NumberFormat",
        "PluralRules",
        "RelativeTimeFormat",
        "Segmenter",
    ] {
        let Ok(ctor): rquickjs::Result<Function> = intl.get(name) else {
            continue;
        };
        let Ok(proto): rquickjs::Result<Object> = ctor.get("prototype") else {
            continue;
        };
        for m in ["constructor", "maximize", "minimize", "compare", "format", "formatToParts"] {
            let _ = proto.remove(m);
        }
        let _ = ctor.remove("prototype");
        let _ = intl.remove(name);
    }
    let _ = ctx.globals().remove("Intl");
}

pub(crate) fn warmup() {
    let _ = provider();
}
