use crate::asn;
use crate::grease;
use crate::hardware::MouseHardware;
use crate::persona::Persona;
use crate::preset::Preset;
use crate::proxy::ProxyConfig;
use compact_str::CompactString;
use compact_str::ToCompactString as _;
use core_utils::BytesExt as _;
use core_utils::canvas_hex_of;
use core_utils::Identity;
use core_utils::rng::seeds;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("ua/{field}: {detail}")]
    Ua {
        field: &'static str,
        detail: CompactString,
    },
    #[error("platform/{field}: {detail}")]
    Platform {
        field: &'static str,
        detail: CompactString,
    },
    #[error("screen/{field}: {detail}")]
    Screen {
        field: &'static str,
        detail: CompactString,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Windows,
    MacOS,
    Linux,
    Android,
}

impl Platform {
    pub const fn as_str(self) -> &'static str {
        match self {
            Platform::Windows => "Windows",
            Platform::MacOS => "macOS",
            Platform::Linux => "Linux",
            Platform::Android => "Android",
        }
    }

    pub const fn is_mobile(self) -> bool {
        matches!(self, Platform::Android)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Chrome { major: u16 },
    Edge { major: u16 },
    Firefox { major: u16 },
    Safari,
}

pub const POINTER_MOUSE: u8 = 0;
pub const POINTER_TOUCHPAD: u8 = 1;
pub const POINTER_TOUCH: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetKind {
    Datacenter,
    Residential,
    Mobile,
}



#[derive(Debug, Clone)]
pub struct Profile {
    pub ua: Arc<str>,
    pub sec_ch_ua: Arc<str>,
    pub accept_language: Arc<str>,
    pub platform: Platform,
    pub preset: Preset,
    pub locale: CompactString,
    pub tz: CompactString,
    pub screen_w: u32,
    pub screen_h: u32,
    pub dpr_x1000: u16,
    pub canvas_seed: u64,
    pub asn: u32,
    pub net: NetKind,
    pub family: Family,
    pub display_hz: u16,
    pub pointer_kind: u8,
    pub pointer_speed: u8,
    pub battery_saver: bool,
    pub proxy: Option<ProxyConfig>,
    pub persona: Persona,
    pub hw: MouseHardware,
}

impl Profile {
    #[inline(always)]
    pub fn hw_concurrency(&self) -> u8 {
        self.preset.hw_concurrency()
    }

    #[inline(always)]
    pub fn device_memory(&self) -> u8 {
        self.preset.device_memory()
    }

    #[inline(always)]
    pub fn cpu_scale(&self) -> f64 {
        self.preset.cpu_scale(self.platform)
    }

    #[inline(always)]
    pub fn cpu_model(&self) -> &'static str {
        self.preset.cpu_model()
    }

    #[inline(always)]
    pub fn webgl_vendor(&self) -> &'static str {
        self.preset.webgl_vendor()
    }

    #[inline(always)]
    pub fn webgl_renderer(&self) -> &'static str {
        self.preset.renderer(self.platform)
    }

    #[inline]
    pub fn screen_css(&self) -> (f64, f64) {
        let dpr = self.device_pixel_ratio();
        (
            (f64::from(self.screen_w) / dpr).round(),
            (f64::from(self.screen_h) / dpr).round(),
        )
    }

    #[inline]
    fn geom(&self, salt: u64) -> Identity {
        Identity::new(self.canvas_seed).at(salt)
    }

    #[inline]
    pub fn taskbar_css_px(&self) -> f64 {
        match self.platform {
            Platform::Android => 0.0,
            Platform::MacOS => self.geom(seeds::SALT_TASKBAR).f64_in(70.0, 82.0),
            _ => match self.geom(seeds::SALT_TASKBAR).u64_in(0, 4) {
                0 => 0.0,
                1 => 30.0,
                _ => 40.0,
            },
        }
    }

    #[inline]
    pub fn chrome_css_px(&self) -> f64 {
        if self.platform.is_mobile() {
            return self.geom(seeds::SALT_CHROME).f64_in(56.0, 104.0);
        }
        let toolbar = if self.geom(seeds::SALT_BOOKMARKS).chance(0.35) {
            28.0
        } else {
            0.0
        };
        87.0 + toolbar
    }

    pub fn avail_css(&self) -> (f64, f64) {
        let (w, h) = self.screen_css();
        (w, (h - self.taskbar_css_px()).max(240.0))
    }

    pub fn viewport(&self) -> (f64, f64) {
        let (w, h) = self.avail_css();
        if self.platform.is_mobile() {
            (w, h.max(320.0))
        } else {
            (w, (h - self.chrome_css_px()).max(240.0))
        }
    }

    pub fn shell() -> Self {
        let mut s = Self {
            ua: Arc::from(""),
            sec_ch_ua: Arc::from(""),
            accept_language: Arc::from(""),
            platform: Platform::Windows,
            preset: Preset::I5_6300U,
            locale: CompactString::new(""),
            tz: CompactString::new(""),
            screen_w: 0,
            screen_h: 0,
            dpr_x1000: 1000,
            canvas_seed: 0,
            asn: 0,
            net: NetKind::Datacenter,
            family: Family::Chrome { major: 0 },
            display_hz: 60,
            pointer_kind: POINTER_MOUSE,
            pointer_speed: 10,
            battery_saver: false,
            proxy: None,
            persona: Persona::derive(0),
            hw: MouseHardware {
                poll_hz: 125,
                dpi: 96,
                pointer_speed: 10,
                accel: true,
                battery_saver: false,
                kind: POINTER_MOUSE,
            },
        };
        s.rebind_identity();
        s
    }

    pub fn rebind_identity(&mut self) {
        self.persona = Persona::derive(self.canvas_seed);
        self.hw = crate::hardware::hardware_of(self, self.persona);
        self.display_hz = crate::hardware::display_hz_for(self);
    }

    pub fn canvas_hex(&self) -> CompactString {
        canvas_hex_of(self.canvas_seed, self.webgl_vendor(), self.webgl_renderer())
    }

    #[inline]
    pub fn emit_hz(&self) -> u32 {
        let hz = u32::from(self.display_hz);
        if hz == 0 { 60 } else { hz.clamp(24, 240) }
    }

    #[inline]
    pub fn device_pixel_ratio(&self) -> f64 {
        f64::from(self.dpr_x1000) / 1000.0
    }

    pub fn validate(&self) -> Result<(), ProfileError> {
        let ua = self.ua.as_ref();
        let sec = self.sec_ch_ua.as_ref();
        let plat_token = match self.platform {
            Platform::Windows => "Windows NT",
            Platform::MacOS => "Macintosh",
            Platform::Linux => "X11",
            Platform::Android => "Android",
        };
        if !ua_has(ua.as_bytes(), plat_token.as_bytes()) {
            return Err(ProfileError::Ua {
                field: "platform-token",
                detail: CompactString::new(plat_token),
            });
        }
        if ua_has(ua.as_bytes(), b"HeadlessChrome")
            || ua_has(ua.as_bytes(), b"PhantomJS")
            || ua_has(ua.as_bytes(), b"python-requests")
        {
            return Err(ProfileError::Ua {
                field: "automation-mark",
                detail: CompactString::new("ua leaks automation"),
            });
        }
        let (name, major, uam) = match self.family {
            Family::Chrome { major } => ("Chrome", major, ua_major_after(ua, "Chrome/")),
            Family::Edge { major } => ("Edg", major, ua_major_after(ua, "Edg/")),
            Family::Firefox { major } => ("Firefox", major, ua_major_after(ua, "Firefox/")),
            Family::Safari => ("Version", 0, ua_major_after(ua, "Version/")),
        };
        let safari = matches!(self.family, Family::Safari);
        if !safari {
            if let Some(expect) = uam
                && expect != major
            {
                return Err(ProfileError::Ua {
                    field: "ua-major",
                    detail: format_args!("{name}: {expect} != {major}").to_compact_string(),
                });
            }
            let needle = vquoted_needle(";v=\"", major);
            if !ua_has(sec.as_bytes(), needle.as_bytes()) && name != "Firefox" {
                return Err(ProfileError::Ua {
                    field: "sec-ch-ua-major",
                    detail: format_args!("{name} {major} missing in sec-ch-ua")
                        .to_compact_string(),
                });
            }
        }
        if self.platform.is_mobile() != ua_has(ua.as_bytes(), b"Mobi") {
            return Err(ProfileError::Ua {
                field: "mobile-flag",
                detail: CompactString::new("platform mobile != ua Mobi"),
            });
        }
        if self.screen_w == 0 || self.screen_h == 0 {
            return Err(ProfileError::Screen {
                field: "dims",
                detail: CompactString::new("zero screen"),
            });
        }
        if !self.locale.contains('-') || self.locale.len() < 4 {
            return Err(ProfileError::Platform {
                field: "locale",
                detail: self.locale.clone(),
            });
        }
        let renderer = self.webgl_renderer();
        let vendor = self.webgl_vendor();
        let has_ci = |hay: &str, needle: &str| hay.as_bytes().contains_ci(needle.as_bytes());
        if has_ci(renderer, "llvmpipe") || has_ci(renderer, "swrast") {
            return Err(ProfileError::Screen {
                field: "renderer",
                detail: CompactString::new(renderer),
            });
        }
        let gpu_family = if has_ci(renderer, "nvidia") {
            "nvidia"
        } else if has_ci(renderer, "intel") {
            "intel"
        } else if has_ci(renderer, "radeon") || has_ci(renderer, "vega") || has_ci(renderer, "amd")
        {
            "amd"
        } else if has_ci(renderer, "apple") || has_ci(renderer, "metal") {
            "apple"
        } else {
            ""
        };
        match self.platform {
            Platform::Windows | Platform::Linux => {
                if gpu_family == "apple" || !renderer.starts_with("ANGLE (") {
                    return Err(ProfileError::Screen {
                        field: "renderer",
                        detail: CompactString::new(renderer),
                    });
                }
            }
            Platform::MacOS => {
                if gpu_family != "apple" {
                    return Err(ProfileError::Screen {
                        field: "renderer",
                        detail: CompactString::new(renderer),
                    });
                }
            }
            Platform::Android => {}
        }
        if !gpu_family.is_empty() && !has_ci(vendor, gpu_family) {
            return Err(ProfileError::Screen {
                field: "vendor",
                detail: CompactString::new(vendor),
            });
        }
        if !self.preset.supports(self.platform) {
            return Err(ProfileError::Platform {
                field: "preset",
                detail: CompactString::const_new("platform has no hardware pool"),
            });
        }
        let asn_row = asn::asn_lookup(self.asn);
        if let Some(row) = asn_row
            && !row.carrier.is_empty()
            && self.tz.as_str() != row.tz
        {
            return Err(ProfileError::Platform {
                field: "tz-geo",
                detail: self.tz.clone(),
            });
        }
        if !(core_utils::bench::CPU_SCALE_MIN..=core_utils::bench::CPU_SCALE_MAX).contains(&self.cpu_scale())
            || self.hw_concurrency() == 0
            || self.hw_concurrency() > 32
            || self.device_memory() == 0
            || self.pointer_kind > POINTER_TOUCH
            || !(1..=20).contains(&self.pointer_speed)
            || !(500..=4000).contains(&self.dpr_x1000)
        {
            return Err(ProfileError::Platform {
                field: "hw",
                detail: "cpu/mem/gpu class inconsistent".into(),
            });
        }
        if let Family::Chrome { major } = self.family
            && (120..=160).contains(&major)
        {
            let brands = grease::grease_brand_set(u32::from(major));
            let has_chromium = brands.iter().any(|(b, _)| *b == "Chromium");
            let has_chrome = brands.iter().any(|(b, _)| *b == "Google Chrome");
            if !has_chromium || !has_chrome {
                return Err(ProfileError::Ua {
                    field: "brands",
                    detail: CompactString::const_new("brand set missing Chromium/Google Chrome"),
                });
            }
            for (brand, version) in brands {
                if brand == "Google Chrome" && version.parse::<u32>().ok() != Some(u32::from(major))
                {
                    return Err(ProfileError::Ua {
                        field: "brands-version",
                        detail: format_args!("{version} != {major}").to_compact_string(),
                    });
                }
            }
            let header = grease::sec_chua_header_of(&brands);
            let needle = vquoted_needle("\"Chromium\";v=\"", major);
            if !ua_has(sec.as_bytes(), needle.as_bytes())
                && !ua_has(sec.as_bytes(), header.as_bytes())
            {
                return Err(ProfileError::Ua {
                    field: "sec-ch-ua-grease",
                    detail: header,
                });
            }
        }
        if let Some(proxy) = &self.proxy {
            if let Some(offset) = proxy.utc_offset {
                let host = proxy.host();
                if let Some((expected, actual)) =
                    crate::geo::check_tz_ip_consistency(host.as_str(), offset)
                {
                    return Err(ProfileError::Platform {
                        field: "proxy-utc",
                        detail: format_args!("{actual} != {expected} for {}", host.as_str())
                            .to_compact_string(),
                    });
                }
            }
            let proxy_ok = match asn_row {
                Some(row) => self.net == asn::net_kind_of(row.net),
                None => true,
            };
            if !proxy_ok {
                return Err(ProfileError::Platform {
                    field: "proxy-kind",
                    detail: CompactString::const_new(
                        "proxy asn marked datacenter/vpn/tor but net kind says otherwise",
                    ),
                });
            }
        }
        Ok(())
    }


    #[inline]
    pub fn chrome_major(&self) -> Option<u32> {
        match self.family {
            Family::Chrome { major } | Family::Edge { major } => Some(u32::from(major)),
            _ => None,
        }
    }


    pub fn ua_full_version(&self) -> Option<&'static str> {
        grease::chrome_full_version(self.chrome_major()?)
    }

}

fn ua_has(ua: &[u8], token: &[u8]) -> bool {
    ua.find_sub(token).is_some()
}

fn vquoted_needle(prefix: &str, major: u16) -> CompactString {
    let mut needle = CompactString::new(prefix);
    core_utils::push_int_into(&mut needle, i64::from(major));
    needle.push('"');
    needle
}

fn ua_major_after(ua: &str, marker: &str) -> Option<u16> {
    let v = ua.as_bytes().find_u32(marker.as_bytes())?;
    if v > 999 {
        return None;
    }
    u16::try_from(v).ok()
}

pub fn derived_from(
    base: &Profile,
    preset: Preset,
    locale: &str,
    tz: &str,
    asn: u32,
    net: NetKind,
    canvas_seed: u64,
) -> Profile {
    let mut out = base.clone();
    out.preset = preset;
    out.locale = CompactString::new(locale);
    out.tz = CompactString::new(tz);
    out.canvas_seed = canvas_seed;
    out.asn = asn;
    out.net = net;
    out.rebind_identity();
    out
}

fn preset_for_skill(canvas_seed: u64) -> Preset {
    let skill = core_utils::Identity::new(canvas_seed)
        .at(core_utils::rng::seeds::SALT_SKILL)
        .f64_unit();
    let pool = crate::preset::POOL;
    let idx = (skill * (pool.len() as f64 - 1.0)).round() as usize;
    pool[idx.min(pool.len() - 1)]
}

pub fn reslot_for_asn(base: &Profile, asn: u32) -> Profile {
    let asn = base.proxy.as_ref().and_then(|p| p.asn).unwrap_or(asn);
    let info = asn_info(asn);
    let canvas_seed = core_utils::rng::mix64(base.canvas_seed ^ u64::from(asn));
    derived_from(
        base,
        preset_for_skill(canvas_seed),
        info.locale,
        info.tz,
        asn,
        info.net,
        canvas_seed,
    )
}

pub struct AsnInfo {
    pub carrier: &'static str,
    pub net: NetKind,
    pub tz: &'static str,
    pub locale: &'static str,
}

pub fn asn_info(asn: u32) -> AsnInfo {
    match asn::asn_lookup(asn) {
        Some(r) => AsnInfo {
            carrier: r.carrier,
            net: asn::net_kind_of(r.net),
            tz: if r.tz.is_empty() {
                crate::geo::DEFAULT_TZ
            } else {
                r.tz
            },
            locale: if r.locale.is_empty() {
                crate::geo::DEFAULT_LOCALE
            } else {
                r.locale
            },
        },
        None => AsnInfo {
            carrier: "unknown",
            net: NetKind::Residential,
            tz: crate::geo::DEFAULT_TZ,
            locale: crate::geo::DEFAULT_LOCALE,
        },
    }
}

#[inline]
pub fn mix64_seed_text(seed: u64, text: &str) -> u64 {
    core_utils::rng::mix64(seed ^ core_utils::xxh3::hash(text.as_bytes()))
}

#[inline]
pub fn canvas_seed_of_ua(ua: &str, seed: u64) -> u64 {
    mix64_seed_text(seed, ua)
}

#[inline]
pub fn mix_proxy_identity(seed: u64, proxy_url: &str) -> u64 {
    mix64_seed_text(seed, proxy_url)
}
