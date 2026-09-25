use crate::catalog::EngineSet;
use core_utils::BytesExt as _;
use core_utils::StrExt as _;
use core_utils::join_origin;

use bytes::Bytes;
use compact_str::CompactString;
use futures_util::StreamExt;
use parser_pipeline::{Flow, PageData, StreamPipeline};
use session_state::Session;
use smallvec::SmallVec;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use wreq::header::{COOKIE, HeaderName, HeaderValue, LOCATION, SET_COOKIE};

const URL_MAX: usize = 8192;
const HEAD_WINDOW: usize = 2048;
const RAW_CAP: u64 = 8 * 1024 * 1024;
const BODY_BRAKE: u64 = parser_pipeline::BYTE_BRAKE;
const REDIRECT_DRAIN: usize = 64 * 1024;

const MAX_HOPS: u8 = crate::catalog::REDIRECT_LIMIT as u8 + 1;

#[derive(Debug, Error)]
pub enum NetError {
    #[error("bad url")]
    Url,
    #[error("blocked url")]
    Blocked,
    #[error("transport: {0}")]
    Transport(#[from] wreq::Error),
    #[error("pipe: {0}")]
    Pipe(#[from] parser_pipeline::PipeError),
    #[error("bad telemetry payload")]
    Payload,
}

pub const VENDOR_NONE: u8 = 0;
pub const VENDOR_CLOUDFLARE: u8 = 1;
pub const VENDOR_DATADOME: u8 = 2;
pub const VENDOR_KASADA: u8 = 3;
pub const VENDOR_PERIMETERX: u8 = 4;
pub const VENDOR_AKAMAI: u8 = 5;
pub const VENDOR_GENERIC: u8 = 6;

pub fn vendor_label(v: u8) -> &'static str {
    match v {
        VENDOR_CLOUDFLARE => "cloudflare",
        VENDOR_DATADOME => "datadome",
        VENDOR_KASADA => "kasada",
        VENDOR_PERIMETERX => "perimeterx",
        VENDOR_AKAMAI => "akamai",
        VENDOR_GENERIC => "generic",
        _ => "none",
    }
}

#[inline]
pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
            {
                return true;
            }
            if let Some(v4) = v6.to_ipv4_mapped().or_else(|| v6.to_ipv4()) {
                return is_forbidden_ip(IpAddr::V4(v4));
            }
            false
        }
    }
}

fn env_allows_private_network() -> bool {
    core_utils::env_present("SILO_ALLOW_PRIVATE_NETWORK")
}

fn num_radix(p: &str) -> Option<u64> {
    if p.is_empty() {
        return Some(0);
    }
    if let Some(hex) = p.strip_prefix("0x").or_else(|| p.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else if p.len() > 1 && p.starts_with('0') {
        u64::from_str_radix(&p[1..], 8).ok()
    } else if p.bytes().all(|b| b.is_ascii_digit()) {
        p.parse().ok()
    } else {
        None
    }
}

fn ipv4_candidate(host: &str) -> Option<std::net::Ipv4Addr> {
    let mut parts = host.split('.').peekable();
    let mut acc: u128 = 0;
    let mut rem = 4usize;
    let mut n = 0u8;
    while let Some(p) = parts.next() {
        n += 1;
        if n > 4 {
            return None;
        }
        let v = num_radix(p)?;
        let width = if parts.peek().is_none() { rem } else { 1 };
        if u128::from(v) >= 1u128 << (8 * width) {
            return None;
        }
        acc = (acc << (8 * width)) + u128::from(v);
        rem -= width;
    }
    u32::try_from(acc).ok().map(std::net::Ipv4Addr::from)
}

pub(crate) fn normalize_host_whatwg(host: &str) -> Option<CompactString> {
    let stripped: SmallVec<[u8; 64]> = host
        .as_bytes()
        .iter()
        .copied()
        .filter(|b| !matches!(b, b'\t' | b'\n' | b'\r'))
        .collect();
    let raw = core_utils::utf8::basic::from_utf8(stripped.as_slice()).ok()?;
    let decoded_cow = core_utils::percent_decode_cow(raw);
    let decoded_str = core_utils::utf8::basic::from_utf8(decoded_cow.as_ref()).ok()?;
    let mut norm = CompactString::with_capacity(decoded_str.len());
    for c in decoded_str.chars() {
        let c = match c {
            '\u{3002}' => '.',

            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            other => other,
        };
        norm.push(c.to_ascii_lowercase());
    }
    Some(norm)
}

pub fn guard_url(url: &str) -> Result<(), NetError> {
    if url.is_empty() || url.len() > URL_MAX {
        return Err(NetError::Url);
    }
    if env_allows_private_network() {
        return Ok(());
    }
    let auth = core_utils::url::split_authority(url);
    if auth.scheme.is_empty() {
        return Err(NetError::Url);
    }
    if !core_utils::url::scheme_is_http(auth.scheme) {
        return Err(NetError::Blocked);
    }

    let raw_host = auth.host;

    let Some(host_norm) = normalize_host_whatwg(raw_host) else {
        return Err(NetError::Blocked);
    };
    let lower = host_norm.as_str();
    if lower.is_empty() {
        return Err(NetError::Url);
    }
    if lower == "localhost" || lower.ends_with(".localhost") {
        return Err(NetError::Blocked);
    }
    if let Ok(ip) = lower.parse::<IpAddr>() {
        if is_forbidden_ip(ip) {
            return Err(NetError::Blocked);
        }
        return Ok(());
    }
    if let Some(v4) = ipv4_candidate(lower)
        && is_forbidden_ip(IpAddr::V4(v4))
    {
        return Err(NetError::Blocked);
    }
    if !lower.contains('.') && !lower.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(NetError::Blocked);
    }
    Ok(())
}

pub const HDR_CF_MITIGATED_CHALLENGE: u32 = 1 << 0;
pub const HDR_CF_RAY: u32 = 1 << 1;
pub const HDR_DD_B: u32 = 1 << 2;
pub const HDR_KPSDK: u32 = 1 << 3;
pub const HDR_PX: u32 = 1 << 4;
pub const HDR_AKAMAI: u32 = 1 << 5;

fn vendor_header_flags(status: u16, resp: &wreq::Response) -> u32 {
    let h = resp.headers();
    let mut flags = 0u32;
    if let Some(v) = h.get("cf-mitigated")
        && v.as_bytes().contains_ci(b"challenge")
    {
        flags |= HDR_CF_MITIGATED_CHALLENGE;
    }
    if (status == 403 || status == 503) && h.contains_key("cf-ray") {
        flags |= HDR_CF_RAY;
    }
    const PRESENCE_FLAGS: &[(&[&str], u32)] = &[
        (&["x-dd-b", "x-ddtrace-id"], HDR_DD_B),
        (&["x-kpsdk-cd", "x-kpsdk-ct"], HDR_KPSDK),
        (&["x-px-captcha", "x-px"], HDR_PX),
        (&["x-akamai-transformed"], HDR_AKAMAI),
    ];
    for &(names, flag) in PRESENCE_FLAGS {
        if names.iter().any(|name| h.contains_key(*name)) {
            flags |= flag;
        }
    }
    flags
}

pub fn challenge_vendor_of(status: u16, header_flags: u32, body_head: &[u8]) -> u8 {
    if header_flags & (HDR_CF_MITIGATED_CHALLENGE | HDR_CF_RAY) != 0 {
        return VENDOR_CLOUDFLARE;
    }
    if (status == 403 || status == 503)
        && (body_head.find_sub(b"jschl_vc").is_some()
            || body_head.find_sub(b"cf-challenge").is_some()
            || body_head.find_sub(b"Just a moment...").is_some())
    {
        return VENDOR_CLOUDFLARE;
    }
    if header_flags & HDR_DD_B != 0 {
        return VENDOR_DATADOME;
    }
    if status == 403 && body_head.contains_ci(b"datadome") {
        return VENDOR_DATADOME;
    }
    if header_flags & HDR_KPSDK != 0 {
        return VENDOR_KASADA;
    }
    if header_flags & HDR_PX != 0 {
        return VENDOR_PERIMETERX;
    }
    if header_flags & HDR_AKAMAI != 0 && body_head.find_sub(b"/_sec/").is_some() {
        return VENDOR_AKAMAI;
    }
    if status == 429 || status == 403 {
        return VENDOR_GENERIC;
    }
    VENDOR_NONE
}

pub struct Fetched {
    pub status: u16,
    pub uri: CompactString,
    pub page: Arc<PageData>,
    pub bytes_in: u64,
    pub elapsed_ms: u64,
    pub challenge_vendor: u8,
}

async fn send_with_retry(req: wreq::RequestBuilder) -> Result<wreq::Response, NetError> {
    let retry = req.try_clone();
    match req.send().await {
        Err(e) if e.is_connection_reset() => match retry {
            Some(r) => r.send().await.map_err(NetError::from),
            None => return Err(e.into()),
        },
        other => other.map_err(NetError::from),
    }
}

async fn send_with_jar(
    session: &mut Session,
    url: &str,
    req: wreq::RequestBuilder,
) -> Result<wreq::Response, NetError> {
    let req = apply_cookie(req, session, url);
    let resp = send_with_retry(req).await?;
    ingest_set_cookies(&resp, session, url);
    Ok(resp)
}

pub async fn push_telemetry(
    engines: &EngineSet,
    slot: usize,
    route: &parser_pipeline::TelemetryRoute,
    endpoint: &str,
    cookie: &str,
    blob: Bytes,
) -> Result<(u16, SmallVec<[CompactString; 4]>), NetError> {
    guard_url(endpoint)?;
    let client = engines.client_for(slot);
    let req = match route.transport {
        parser_pipeline::Transport::FormField => {
            let value = blob.as_ref().b64_string();
            client.post(endpoint).form(&[(route.field.as_str(), value)])
        }
        parser_pipeline::Transport::CustomHeader => {
            let name =
                HeaderName::from_bytes(route.field.as_bytes()).map_err(|_| NetError::Payload)?;
            let value = HeaderValue::from_bytes(blob.as_ref()).map_err(|_| NetError::Payload)?;
            client.post(endpoint).header(name, value)
        }
        parser_pipeline::Transport::CdnPost => client
            .post(endpoint)
            .header(wreq::header::CONTENT_TYPE, "application/octet-stream")
            .body(blob),
    };
    let req = if cookie.is_empty() {
        req
    } else {
        with_cookie(req, cookie)
    };
    let resp = send_with_retry(req).await?;
    Ok((resp.status().as_u16(), set_cookie_lines(&resp)))
}

pub async fn fetch_page(
    engines: &EngineSet,
    slot: usize,
    session: &mut Session,
    url: &str,
) -> Result<Fetched, NetError> {
    fetch_page_sel(engines, slot, session, url, &[]).await
}

enum BodyFeed {
    Utf8(StreamPipeline),
    Transcode(StreamPipeline, encoding_rs::Decoder, String),
}

impl BodyFeed {
    fn push(&mut self, bytes: &[u8]) -> Result<bool, parser_pipeline::PipeError> {
        match self {
            BodyFeed::Utf8(p) => Ok(p.push(bytes)? == Flow::Stop),
            BodyFeed::Transcode(..) => self.transcode(bytes, false),
        }
    }

    fn flush(&mut self) -> Result<bool, parser_pipeline::PipeError> {
        match self {
            BodyFeed::Utf8(_) => Ok(false),
            BodyFeed::Transcode(..) => self.transcode(b"", true),
        }
    }

    fn transcode(&mut self, mut input: &[u8], last: bool) -> Result<bool, parser_pipeline::PipeError> {
        let BodyFeed::Transcode(p, dec, text) = self else {
            return Ok(false);
        };
        loop {
            let (rv, read, _) = dec.decode_to_string(input, text, last);
            input = &input[read..];
            if !text.is_empty() {
                if p.push(text.as_bytes())? == Flow::Stop {
                    return Ok(true);
                }
                text.clear();
            }
            if matches!(rv, encoding_rs::CoderResult::InputEmpty) {
                return Ok(false);
            }
        }
    }

    fn finish(self) -> Result<PageData, parser_pipeline::PipeError> {
        match self {
            BodyFeed::Utf8(p) => p.finish(),
            BodyFeed::Transcode(p, ..) => p.finish(),
        }
    }
}

fn build_feed(content_type: Option<&str>, head: &[u8], pipeline: StreamPipeline) -> BodyFeed {
    match crate::charset::resolve_encoding(content_type, head) {
        Some(enc) => BodyFeed::Transcode(
            pipeline,
            enc.new_decoder(),
            String::with_capacity(24 * 1024),
        ),
        None => BodyFeed::Utf8(pipeline),
    }
}

pub async fn fetch_page_sel(
    engines: &EngineSet,
    slot: usize,
    session: &mut Session,
    url: &str,
    selectors: &[(String, String)],
) -> Result<Fetched, NetError> {
    let start = Instant::now();
    guard_url(url)?;
    if crate::blocklist::url_blocked(url) {
        tracing::debug!(target: "net", "tracker blocked: {url}");

        return Ok(Fetched {
            status: 204,
            uri: CompactString::new(url),
            page: Arc::new(PageData::empty()),
            bytes_in: 0,
            elapsed_ms: core_utils::ms(start, Instant::now()),
            challenge_vendor: VENDOR_NONE,
        });
    }
    let client = engines.client_for(slot);
    let resp = send_with_jar(session, url, client.get(url)).await?;
    let status = resp.status().as_u16();
    let uri = final_uri(&resp);
    let header_flags = vendor_header_flags(status, &resp);
    let content_type: Option<CompactString> = resp
        .headers()
        .get(wreq::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(CompactString::new);
    let declared_len = resp
        .headers()
        .get(wreq::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let make_pipeline = || {
        if selectors.is_empty() {
            StreamPipeline::new(Default::default())
        } else {
            StreamPipeline::with_selectors(Default::default(), selectors)
        }
    };
    let braked = declared_len.is_some_and(|len| len > BODY_BRAKE);
    let mut truncated = braked;
    if braked {
        tracing::debug!(target: "net", "content-length over brake, body skipped");
    }
    let mut stream = resp.bytes_stream();

    let mut head_buf: SmallVec<[u8; HEAD_WINDOW]> = SmallVec::new();
    let mut head_done = false;
    let mut feed: Option<BodyFeed> = None;

    let mut stopped = false;
    let mut bytes_in: u64 = 0;

    let mut transport_err = false;
    if !braked {
        while !stopped {
            let chunk = match stream.next().await {
                Some(Ok(c)) => c,
                Some(Err(e)) => {
                    tracing::debug!(target: "net", "transport error mid-body: {e}");
                    transport_err = true;
                    break;
                }
                None => break,
            };
            bytes_in += chunk.len() as u64;
            if bytes_in > RAW_CAP {
                truncated = true;
                break;
            }
            let mut rest: &[u8] = &chunk;
            if !head_done {
                let take = rest.len().min(HEAD_WINDOW - head_buf.len());
                head_buf.extend_from_slice(&rest[..take]);
                rest = &rest[take..];
                if head_buf.len() == HEAD_WINDOW {
                    head_done = true;
                    let mut f = build_feed(content_type.as_deref(), head_buf.as_slice(), make_pipeline());
                    stopped = f.push(head_buf.as_slice())?;
                    feed = Some(f);
                }
            }
            if let Some(f) = feed.as_mut()
                && !rest.is_empty()
            {
                stopped = f.push(rest)?;
            }
        }

        if feed.is_none() && !head_buf.is_empty() {
            let mut f = build_feed(content_type.as_deref(), head_buf.as_slice(), make_pipeline());
            stopped = f.push(head_buf.as_slice())?;
            stopped |= f.flush()?;
            feed = Some(f);
        } else if let Some(f) = feed.as_mut() {
            stopped |= f.flush()?;
        }
    }
    let vendor = challenge_vendor_of(status, header_flags, head_buf.as_slice());
    if stopped {
        tracing::debug!(target: "net", "byte brake hit after {bytes_in} bytes");
    }
    let mut page = match feed {
        Some(f) => f.finish()?,

        None => {
            let mut empty = PageData::empty();
            empty.truncated = true;
            empty
        }
    };
    if truncated || transport_err {
        page.truncated = true;
    }
    Ok(Fetched {
        status,
        uri,
        page: Arc::new(page),
        bytes_in,
        elapsed_ms: core_utils::ms(start, Instant::now()),
        challenge_vendor: vendor,
    })
}

pub fn parse_body(body: &[u8]) -> Result<PageData, NetError> {
    let mut pipeline = StreamPipeline::new(Default::default());
    pipeline.push(body)?;
    Ok(pipeline.finish()?)
}

pub fn with_cookie(req: wreq::RequestBuilder, cookie: &str) -> wreq::RequestBuilder {
    match HeaderValue::from_str(cookie) {
        Ok(cv) => req.header(COOKIE, cv),
        Err(_) => req,
    }
}

pub fn set_cookie_lines(resp: &wreq::Response) -> SmallVec<[CompactString; 4]> {
    resp.headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(CompactString::new)
        .collect()
}

fn apply_cookie(
    req: wreq::RequestBuilder,
    session: &mut session_state::Session,
    url: &str,
) -> wreq::RequestBuilder {
    match session.jar.header_for_url_into(url, &mut session.cookie_scratch) {
        Some(s) => with_cookie(req, s),
        None => req,
    }
}

fn ingest_set_cookies(resp: &wreq::Response, session: &mut Session, url: &str) {
    for line in set_cookie_lines(resp) {
        session.jar.ingest_for_url(line.as_str(), url);
    }
    session.touch();
}

fn final_uri(resp: &wreq::Response) -> compact_str::CompactString {
    let resp_uri = resp.uri();
    let mut uri = compact_str::CompactString::with_capacity(128);
    if let Some(scheme) = resp_uri.scheme() {
        uri.push_str(scheme.as_str());
        uri.push_str("://");
    }
    if let Some(authority) = resp_uri.authority() {
        uri.push_str(authority.as_str());
    }
    uri.push_str(resp_uri.path());
    if let Some(query) = resp_uri.query() {
        uri.push('?');
        uri.push_str(query);
    }
    uri
}

pub struct AnubisPass {
    pub status: u16,
    pub hops: u8,
    pub final_uri: CompactString,
    pub auth_cookie: Option<CompactString>,
}

const AUTH_COOKIE_NAME: &str = "techaro.lol-anubis-auth";

async fn drain_up_to(resp: wreq::Response, cap: usize) {
    let mut stream = resp.bytes_stream();
    let mut drained = 0usize;
    while drained < cap {
        match stream.next().await {
            Some(Ok(c)) => drained += c.len(),
            _ => break,
        }
    }
}

pub async fn pass_anubis(
    engines: &EngineSet,
    slot: usize,
    session: &mut Session,
    url: &str,
    referer: &str,
) -> Result<AnubisPass, NetError> {
    let client = engines.direct_for(slot);
    let mut current: CompactString = CompactString::new(url);
    let mut referer = CompactString::new(referer);
    let mut hops = 0u8;
    loop {
        guard_url(current.as_str())?;
        let resp = send_with_jar(
            session,
            current.as_str(),
            client
                .get(current.as_str())
                .header("referer", referer.as_str()),
        )
        .await?;
        let status = resp.status().as_u16();
        let uri = final_uri(&resp);
        if matches!(status, 301 | 302 | 303 | 307 | 308) && hops < MAX_HOPS {
            let next = resp
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(|loc| join_origin(current.as_str(), loc, true));
            referer = current;
            if let Some(next) = next {
                drain_up_to(resp, REDIRECT_DRAIN).await;
                current = next;
                hops += 1;
                continue;
            }
        }

        drain_up_to(resp, BODY_BRAKE as usize).await;
        let host = uri.host();
        return Ok(AnubisPass {
            status,
            hops,
            final_uri: uri,
            auth_cookie: session
                .jar
                .get_for_host(AUTH_COOKIE_NAME, host.as_str())
                .map(CompactString::new),
        });
    }
}
