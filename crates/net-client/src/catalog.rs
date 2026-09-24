use compact_str::CompactString;
use session_state::{Family, NetKind, Platform, Preset, Profile, ProxyConfig};
use std::sync::Arc;
use std::time::Duration;
use wreq::IntoEmulation as _;
use wreq::header::{ACCEPT_LANGUAGE, HeaderName, USER_AGENT};
use wreq::redirect::Policy;
use wreq_util::{Emulation, Platform as WPlatform, Profile as WProfile};

const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const POOL_MAX_IDLE_PER_HOST: usize = 256;
const POOL_MAX_SIZE: usize = 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const REDIRECT_LIMIT: usize = 5;

struct Row {
    profile: WProfile,
    platform: WPlatform,
    os: Platform,
    family: Family,
    net: NetKind,
    preset: Preset,
    screen: (u32, u32),
    dpr_x1000: u16,
    display_hz: u16,
    pointer: (u8, u8, bool),
}

macro_rules! row {
    ($wp:ident, $pl:ident, $fam:ident, $mj:expr, $net:ident, $pre:ident, $w:expr, $h:expr, $dpr:expr, $hz:expr, $pk:ident, $ps:expr, $bs:expr) => {
        Row {
            profile: WProfile::$wp,
            platform: WPlatform::$pl,
            os: Platform::$pl,
            family: Family::$fam { major: $mj },
            net: NetKind::$net,
            preset: Preset::$pre,
            screen: ($w, $h),
            dpr_x1000: $dpr,
            display_hz: $hz,
            pointer: (session_state::$pk, $ps, $bs),
        }
    };
}

const ROWS: &[Row] = &[
    row!(Chrome149, Windows, Chrome, 149, Residential, I5_6300U, 1920, 1080, 1000, 60, POINTER_MOUSE, 10, false),
    row!(Chrome148, Windows, Chrome, 148, Residential, I5_6200U, 1920, 1080, 1250, 60, POINTER_TOUCHPAD, 10, false),
    row!(Edge148, Windows, Edge, 148, Residential, I5_4200U, 1600, 900, 1000, 60, POINTER_TOUCHPAD, 10, false),
    row!(Firefox151, Windows, Firefox, 151, Residential, I7_6500U, 2560, 1440, 1000, 60, POINTER_MOUSE, 10, false),
    row!(Chrome147, Linux, Chrome, 147, Datacenter, I5_6300U, 1920, 1080, 1000, 60, POINTER_MOUSE, 10, false),
    row!(Chrome149, Linux, Chrome, 149, Datacenter, I7_6600U, 1920, 1080, 1000, 60, POINTER_MOUSE, 6, false),
    row!(Chrome148, Linux, Chrome, 148, Datacenter, Ryzen3_2200U, 1920, 1080, 1500, 60, POINTER_MOUSE, 10, false),
];

fn emulation_of(profile: WProfile, platform: WPlatform) -> wreq::Emulation {
    Emulation::builder()
        .profile(profile)
        .platform(platform)
        .build()
        .into_emulation()
}

fn emulation_headers(row: &Row) -> (String, String, String) {
    let emu = emulation_of(row.profile, row.platform);
    let headers = &emu.headers;
    let ua = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let sec = headers
        .get(HeaderName::from_static("sec-ch-ua"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let lang = headers
        .get(ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("en-US,en;q=0.9")
        .to_owned();
    (ua, sec, lang)
}

fn build_profile(row: &Row, origin_seed: u64) -> Result<Arc<Profile>, String> {
    let (ua, sec, lang) = emulation_headers(row);
    let canvas_seed = session_state::canvas_seed_of_ua(&ua, origin_seed);
    let mut shell = Profile {
        ua: Arc::from(ua.as_str()),
        sec_ch_ua: Arc::from(sec.as_str()),
        accept_language: Arc::from(lang.as_str()),
        platform: row.os,
        preset: row.preset,
        locale: CompactString::new(session_state::DEFAULT_LOCALE),
        tz: CompactString::new(session_state::DEFAULT_TZ),
        screen_w: row.screen.0,
        screen_h: row.screen.1,
        dpr_x1000: row.dpr_x1000,
        canvas_seed,
        asn: 0,
        net: row.net,
        family: row.family,
        pointer_kind: row.pointer.0,
        pointer_speed: row.pointer.1,
        battery_saver: row.pointer.2,
        proxy: None,
        ..Profile::shell()
    };
    shell.rebind_identity();
    shell.validate().map_err(|e| e.to_string())?;
    Ok(Arc::new(shell))
}

#[derive(Clone)]
pub struct CatalogEntry {
    pub profile: Arc<Profile>,
    wprofile: WProfile,
    wplatform: WPlatform,
}

pub fn engine_catalog() -> Result<Vec<CatalogEntry>, String> {
    let mut out = Vec::with_capacity(ROWS.len());
    for row in ROWS {
        out.push(CatalogEntry {
            profile: build_profile(row, 0)?,
            wprofile: row.profile,
            wplatform: row.platform,
        });
    }
    Ok(out)
}

pub fn engine_catalog_with_proxies(proxies: &[String]) -> Result<Vec<CatalogEntry>, String> {
    let base = engine_catalog()?;
    if proxies.is_empty() {
        return Ok(base);
    }
    let mut out = Vec::with_capacity(proxies.len());
    for (i, raw) in proxies.iter().enumerate() {
        let cfg = ProxyConfig::parse(raw).map_err(|e| format!("proxy {raw:?}: {e}"))?;
        if !cfg.wreq_compatible() {
            return Err(format!(
                "proxy scheme {} ({raw:?}) unsupported by transport — refusing to go direct",
                cfg.scheme.as_str()
            ));
        }
        let template = &base[i % base.len()];
        let mut prof = Profile::clone(&template.profile);
        prof.canvas_seed =
            session_state::mix_proxy_identity(prof.canvas_seed, cfg.identity_key().as_str());
        prof.proxy = Some(cfg);
        prof.rebind_identity();
        prof.validate().map_err(|e| format!("proxy {raw:?}: {e}"))?;
        let prof = Arc::new(prof);
        out.push(CatalogEntry {
            profile: prof,
            wprofile: template.wprofile,
            wplatform: template.wplatform,
        });
    }
    Ok(out)
}

pub fn reslot_with_asn(profile: &Profile, asn: u32) -> Arc<Profile> {
    Arc::new(session_state::reslot_for_asn(profile, asn))
}

pub fn reslot_with_proxy(profile: &Profile, asn: u32, proxy: ProxyConfig) -> Arc<Profile> {
    let mut bound = profile.clone();
    if bound.proxy.is_none() {
        bound.proxy = Some(proxy);
    }
    Arc::new(session_state::reslot_for_asn(&bound, asn))
}

pub struct EngineSet {
    clients: Vec<Arc<wreq::Client>>,
    direct: Vec<Arc<wreq::Client>>,
}

fn build_client(entry: &CatalogEntry, policy: Policy) -> Result<wreq::Client, wreq::Error> {
    let emulation = emulation_of(entry.wprofile, entry.wplatform);
    let mut builder = wreq::Client::builder()
        .emulation(emulation)
        .redirect(policy)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .pool_max_idle_per_host(POOL_MAX_IDLE_PER_HOST)
        .pool_max_size(POOL_MAX_SIZE)
        .tcp_nodelay(true)
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT);
    if let Some(proxy) = &entry.profile.proxy {
        let p = wreq::Proxy::all(proxy.to_url_string().as_str()).map_err(|e| {
            tracing::error!(
                target: "net",
                "proxy {} rejected by transport: {e}",
                proxy.redacted_string()
            );
            e
        })?;
        builder = builder.proxy(p);
    }
    builder.build()
}

impl EngineSet {
    pub fn build(catalog: &[CatalogEntry]) -> Result<Self, wreq::Error> {
        let mut clients = Vec::with_capacity(catalog.len());
        let mut direct = Vec::with_capacity(catalog.len());
        for entry in catalog {
            clients.push(Arc::new(build_client(
                entry,
                Policy::limited(REDIRECT_LIMIT),
            )?));
            direct.push(Arc::new(build_client(entry, Policy::none())?));
        }
        Ok(EngineSet { clients, direct })
    }

    pub fn client_for(&self, slot: usize) -> &Arc<wreq::Client> {
        &self.clients[slot % self.clients.len()]
    }

    pub(crate) fn direct_for(&self, slot: usize) -> &Arc<wreq::Client> {
        &self.direct[slot % self.direct.len()]
    }
}
