use runtime_exec::WorkerPool;
use session_state::{Family, NetKind, Platform, Profile};
use std::sync::Arc;
mod common;
use common::{chrome_profile, pool, req_with_profile as req};

fn ru_profile() -> std::sync::Arc<Profile> {
    std::sync::Arc::new(Profile {
        ua: Arc::from(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36",
        ),
        sec_ch_ua: Arc::from(
            r#""Google Chrome";v="149", "Chromium";v="149", "Not)A;Brand";v="24""#,
        ),
        accept_language: Arc::from("ru-RU,ru;q=0.9"),
        platform: Platform::Windows,
        locale: "ru-RU".into(),
        tz: "Europe/Moscow".into(),
        screen_w: 1920,
        screen_h: 1080,
        canvas_seed: 0xFA1E,
        asn: 7922,
        net: NetKind::Residential,
        family: Family::Chrome { major: 149 },
        display_hz: 60,
        ..session_state::Profile::shell()
    })
}

async fn eval(pool: &WorkerPool, script: &str, profile: &Profile) -> String {
    let o = pool.exec(req(script, profile, 3000)).await;
    o.token
        .unwrap_or_else(|| panic!("js error: {:?}", o.err))
        .as_str()
        .to_string()
}

#[tokio::test]
async fn dtf_resolved_options_match_profile() {
    let pool = pool();
    let out = eval(
        &pool,
        "var ro = Intl.DateTimeFormat().resolvedOptions();\
         [ro.locale, ro.calendar, ro.numberingSystem, ro.timeZone, ro.year, ro.month, ro.day].join('|');",
        &chrome_profile(),
    )
    .await;
    assert_eq!(
        out,
        "en-US|gregory|latn|America/New_York|numeric|numeric|numeric"
    );
}

#[tokio::test]
async fn dtf_format_is_chrome_shaped_and_deterministic() {
    let pool = pool();
    let out = eval(
        &pool,
        "var d = new Date(2026, 11, 7);\
         var a = new Intl.DateTimeFormat('en-US').format(d);\
         var b = new Intl.DateTimeFormat('en-US').format(d);\
         var ru = new Intl.DateTimeFormat('ru-RU').format(d);\
         var de = new Intl.DateTimeFormat('de-DE').format(d);\
         var long = new Intl.DateTimeFormat('en-US', { weekday: 'long', year: 'numeric', month: 'long', day: 'numeric' }).format(d);\
         [a, a === b, ru, de, long].join('|');",
        &chrome_profile(),
    )
    .await;
    assert_eq!(
        out, "12/7/2026|true|07.12.2026|07.12.2026|Monday, December 7, 2026",
        "fmt: {out}"
    );
}

#[tokio::test]
async fn dtf_rejects_dirty_inputs_with_chrome_errors() {
    let pool = pool();
    let out = eval(
        &pool,
        "var out = [];\
         try { new Intl.DateTimeFormat('en_US'); } catch (e) { out.push('tag:' + e.name + ':' + (e.message.indexOf('Invalid language tag') === 0)); }\
         try { new Intl.DateTimeFormat().format('not-a-date'); } catch (e) { out.push('fmt:' + e.name + ':' + (e.message === 'Invalid time value')); }\
         try { new Intl.DateTimeFormat().format(NaN); } catch (e) { out.push('nan:' + e.name); }\
         try { new Intl.DateTimeFormat().format(); } catch (e) { out.push('none:' + e.name); }\
         var ok = new Intl.DateTimeFormat('xx-YY').resolvedOptions().locale;\
         out.push('unknown-ok:' + ok);\
         out.join('|');",
        &chrome_profile(),
    )
    .await;
    assert_eq!(
        out,
        "tag:RangeError:true|fmt:RangeError:true|nan:RangeError|none:RangeError|unknown-ok:xx-YY",
        "dirty: {out}"
    );
}

#[tokio::test]
async fn date_timezone_is_consistent_with_intl() {
    let pool = pool();
    let out = eval(
        &pool,
        "var winter = new Date(2026, 0, 15, 12).getTimezoneOffset();\
         var summer = new Date(2026, 6, 15, 12).getTimezoneOffset();\
         var s = new Date(2026, 0, 15, 12).toString();\
         var tzName = Intl.DateTimeFormat().resolvedOptions().timeZone;\
         [winter, summer, s.indexOf('GMT-0500') !== -1, s.indexOf('Eastern') !== -1, tzName].join('|');",
        &chrome_profile(),
    )
    .await;
    assert_eq!(out, "300|240|true|true|America/New_York", "tz: {out}");
}

#[tokio::test]
async fn number_format_groups_like_chrome() {
    let pool = pool();
    let out = eval(
        &pool,
        "var us = new Intl.NumberFormat('en-US');\
         var de = new Intl.NumberFormat('de-DE');\
         var cur = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' });\
         var pct = new Intl.NumberFormat('en-US', { style: 'percent' });\
         var ro = new Intl.NumberFormat('en-US').resolvedOptions();\
         [us.format(1234.56), de.format(1234.56), cur.format(1234.56), pct.format(50),\
          us.format(NaN), us.format(-1), ro.roundingMode, ro.useGrouping, ro.maximumFractionDigits].join('|');",
        &chrome_profile(),
    )
    .await;
    assert_eq!(
        out, "1,234.56|1.234,56|$1,234.56|5,000%|NaN|-1|halfExpand|true|3",
        "nf: {out}"
    );
}

#[tokio::test]
async fn intl_namespace_surface_is_complete() {
    let pool = pool();
    let out = eval(
        &pool,
        "var names = Object.getOwnPropertyNames(Intl).sort().join(',');\
         var cal = Intl.supportedValuesOf('calendar').length;\
         var bad = '';\
         try { Intl.supportedValuesOf('bogus'); } catch (e) { bad = e.name; }\
         var can = Intl.getCanonicalLocales('en-us')[0];\
         [names, cal, bad, can].join('|');",
        &chrome_profile(),
    )
    .await;
    assert_eq!(
        out,
        "Collator,DateTimeFormat,DisplayNames,ListFormat,Locale,NumberFormat,PluralRules,RelativeTimeFormat,Segmenter,getCanonicalLocales,supportedValuesOf|17|RangeError|en-US",
        "ns: {out}"
    );
}

#[tokio::test]
async fn collator_listformat_plural_behave() {
    let pool = pool();
    let out = eval(
        &pool,
        "var coll = new Intl.Collator();\
         var list = new Intl.ListFormat('en');\
         var ruList = new Intl.ListFormat('ru');\
         var pl = new Intl.PluralRules('ru');\
         var plEn = new Intl.PluralRules('en');\
         [coll.compare('a', 'b'), coll.compare('b', 'a'), coll.compare('a', 'a'),\
          list.format(['a', 'b']), list.format(['a', 'b', 'c']), ruList.format(['a', 'b']),\
          pl.select(1), pl.select(2), pl.select(5), plEn.select(1), plEn.select(2)].join('|');",
        &chrome_profile(),
    )
    .await;
    assert_eq!(
        out, "-1|1|0|a and b|a, b, and c|a и b|one|few|many|one|other",
        "coll: {out}"
    );
}

#[tokio::test]
async fn locale_class_parses_and_maximizes() {
    let pool = pool();
    let out = eval(
        &pool,
        "var l = new Intl.Locale('en-US');\
         var m = l.maximize().baseName;\
         var mn = l.minimize().baseName;\
         var bad = '';\
         try { new Intl.Locale('en_US'); } catch (e) { bad = e.name; }\
         [l.baseName, l.language, l.region, String(l), m, mn, bad].join('|');",
        &chrome_profile(),
    )
    .await;
    assert_eq!(
        out, "en-US|en|US|en-US|en-Latn-US|en|RangeError",
        "locale: {out}"
    );
}

#[tokio::test]
async fn rtf_and_segmenter_work() {
    let pool = pool();
    let out = eval(
        &pool,
        "var rtf = new Intl.RelativeTimeFormat('en');\
         var seg = new Intl.Segmenter('en', { granularity: 'word' });\
         var segG = new Intl.Segmenter('en');\
         var words = [];\
         for (var s of seg.segment('ab cd')) { words.push(s.segment); }\
         var chars = [];\
         for (var s of segG.segment('ab')) { chars.push(s.segment); }\
         var bad = '';\
         try { rtf.format(1, 'bogus'); } catch (e) { bad = e.name; }\
         [rtf.format(3, 'day'), rtf.format(-1, 'day'), words.join('/'), chars.join(''), bad, seg.resolvedOptions().granularity, segG.resolvedOptions().granularity].join('|');",
        &chrome_profile(),
    )
    .await;
    assert_eq!(
        out, "in 3 days|1 day ago|ab/ /cd|ab|RangeError|word|grapheme",
        "rtf: {out}"
    );
}

#[tokio::test]
async fn ru_profile_feeds_ru_locale_and_moscow_tz() {
    let pool = pool();
    let out = eval(
        &pool,
        "var ro = Intl.DateTimeFormat().resolvedOptions();\
         var off = new Date(2026, 6, 15, 12).getTimezoneOffset();\
         [ro.locale, ro.timeZone, off].join('|');",
        &ru_profile(),
    )
    .await;
    assert_eq!(out, "ru-RU|Europe/Moscow|-180", "ru: {out}");
}

#[tokio::test]
async fn dtf_instances_link_to_prototype() {
    let pool = pool();
    let out = eval(
        &pool,
        "var dtf = new Intl.DateTimeFormat();\
         var parts = dtf.formatToParts(new Date(2026, 11, 7));\
         var types = parts.map(function (p) { return p.type; }).join(',');\
         var vals = parts.map(function (p) { return p.value; }).join('');\
         [dtf instanceof Intl.DateTimeFormat, typeof Intl.DateTimeFormat.prototype.format, parts.length > 2, types, vals].join('|');",
        &chrome_profile(),
    )
    .await;
    assert!(out.starts_with("true|function|true|"), "proto: {out}");
    assert!(
        out.contains("month,day,year") || out.contains("month,literal,day,literal,year"),
        "parts: {out}"
    );
    assert!(out.ends_with("12/7/2026"), "parts vals: {out}");
}
