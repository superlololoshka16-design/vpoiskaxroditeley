use parser_pipeline::CaptchaFamily::{
    self, CloudflareChallenge, Datadome, Hcaptcha, RecaptchaV2, Turnstile,
};
use parser_pipeline::{StreamPipeline, detect_widget, extract_sitekey_from_url, sitekey_passes};

#[test]
fn sitekey_extraction_garbage_input() {
    assert_eq!(extract_sitekey_from_url(""), None);
    assert_eq!(extract_sitekey_from_url("https://x.com"), None);
    assert_eq!(extract_sitekey_from_url("https://x.com?"), None);
    assert_eq!(extract_sitekey_from_url("https://x.com?a=b"), None);
}

#[test]
fn sitekey_extraction_known_shapes() {
    assert_eq!(
        extract_sitekey_from_url("https://t.site/api?sitekey=0x4AAAAAAA"),
        Some("0x4AAAAAAA")
    );
    assert_eq!(
        extract_sitekey_from_url("https://t.site/api?k=abc123&x=1"),
        Some("abc123")
    );
    assert_eq!(
        extract_sitekey_from_url("https://t.site/api?junk=1&site_key=ZZZ#frag"),
        Some("ZZZ")
    );
    assert_eq!(extract_sitekey_from_url("https://t.site?sitekey="), None);
}

#[test]
fn sitekey_validation_rejects_garbage() {
    assert!(!sitekey_passes(""));
    assert!(!sitekey_passes("has space"));
    assert!(!sitekey_passes("sym!bol"));
    assert!(!sitekey_passes(&"x".repeat(257)));
    assert!(sitekey_passes("0x4AAA-bb_CC"));
}

#[test]
fn detect_widget_empty_input_is_none() {
    assert!(detect_widget(&[], &[]).is_none());
    assert!(detect_widget(&["", "   "], &[""]).is_none());
}

#[test]
fn detect_widget_routes_by_priority() {
    let srcs = [
        "https://challenges.cloudflare.com/turnstile/v0/api.js",
        "https://js.hcaptcha.com/1/api.js",
    ];
    let hit = detect_widget(&srcs, &[]).expect("hcaptcha outranks turnstile");
    assert_eq!(hit.family, Hcaptcha);
}

#[test]
fn detect_widget_marker_beats_plain_src() {
    let markers = ["https://geo.captcha-delivery.com/captcha.js"];
    let srcs = ["https://challenges.cloudflare.com/turnstile/v0/api.js"];
    let hit = detect_widget(&srcs, &markers).expect("marker scanned first per priority");
    assert_eq!(hit.family, Turnstile);
}

#[test]
fn detect_widget_pulls_sitekey_from_url() {
    let srcs = ["https://challenges.cloudflare.com/turnstile/v0/api?sitekey=0xDEAD_BEEF"];
    let hit = detect_widget(&srcs, &[]).expect("turnstile detected");
    assert_eq!(hit.family, Turnstile);
    assert_eq!(hit.sitekey.as_deref(), Some("0xDEAD_BEEF"));
}

#[test]
fn detect_widget_rejects_broken_sitekey() {
    let srcs = ["https://challenges.cloudflare.com/turnstile/v0/api?sitekey=bad key!"];
    let hit = detect_widget(&srcs, &[]).expect("family still detected");
    assert_eq!(hit.family, Turnstile);
    assert!(hit.sitekey.is_none());
}

#[test]
fn family_idx_roundtrip_covers_all() {
    for f in CaptchaFamily::ALL {
        assert_eq!(CaptchaFamily::from_idx(f.idx()), *f);
    }
    assert_eq!(CaptchaFamily::from_idx(200), CaptchaFamily::Unknown);
}

#[test]
fn invisible_families_flagged() {
    assert!(!RecaptchaV2.is_invisible());
    assert!(parser_pipeline::CaptchaFamily::RecaptchaV3.is_invisible());
    assert!(Datadome.is_invisible());
    assert!(CloudflareChallenge.is_invisible());
    assert!(!Turnstile.is_invisible());
}

#[test]
fn pipeline_exposes_family_and_sitekey_from_html() {
    let html = b"<html><body><div class=\"cf-turnstile\" data-sitekey=\"0x4AAA_1234\"></div>\
<script src=\"https://challenges.cloudflare.com/turnstile/v0/api.js\"></script>\
</body></html>";
    let mut pipe = StreamPipeline::new(Default::default());
    pipe.push(html).expect("push");
    let page = pipe.finish().expect("finish");
    assert_eq!(page.captcha_family, Some(Turnstile.idx()));
    assert_eq!(page.captcha_sitekey.as_deref(), Some("0x4AAA_1234"));
}

#[test]
fn pipeline_no_family_on_plain_page() {
    let html = b"<html><body><p>hello</p><script src=\"/app.js\"></script></body></html>";
    let mut pipe = StreamPipeline::new(Default::default());
    pipe.push(html).expect("push");
    let page = pipe.finish().expect("finish");
    assert!(page.captcha_family.is_none());
    assert!(page.captcha_sitekey.is_none());
}

#[test]
fn pipeline_survives_truncated_stream() {
    let mut html = Vec::with_capacity(4096);
    html.extend_from_slice(b"<html><body><div class=\"cf-turnstile\" data-sitekey=\"0x4AAA_1234\"></div><script src=\"https://challenges.cloudflare.com/turn");
    let mut pipe = StreamPipeline::new(Default::default());
    pipe.push(&html).expect("push");
    let page = pipe.finish().expect("finish");
    assert_eq!(page.captcha_sitekey.as_deref(), Some("0x4AAA_1234"));
}
