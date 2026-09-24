use crate::profile::Platform;

macro_rules! presets {
    ($($name:ident),* $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum Preset {
            $($name),*
        }
        pub const POOL: &[Preset] = &[$(Preset::$name),*];
    };
}

presets!(
    I5_3320M,
    I5_4200U,
    Celeron3855U,
    Celeron3955U,
    I3_6006U,
    I3_6100U,
    I5_6200U,
    I5_6300U,
    I7_6500U,
    I7_6600U,
    I5_7200U,
    Ryzen3_2200U,
);

pub struct PresetSpec {
    pub cpu_model: &'static str,
    pub hw_concurrency: u8,
    pub device_memory: u8,
    pub win_scale: f64,
    pub linux_scale: f64,
    pub win_renderer: &'static str,
    pub linux_renderer: &'static str,
    pub webgl_vendor: &'static str,
    pub max_mouse_hz: u32,
    pub max_display_hz: u16,
}

const L4000: &str = "ANGLE (Intel, Mesa DRI Intel(R) Ivybridge Mobile, OpenGL 4.2 (Core Profile) Mesa 23.2.1)";
const W4000: &str = "ANGLE (Intel, Intel(R) HD Graphics 4000 Direct3D11 vs_5_0 ps_5_0, D3D11)";
const INTEL: &str = "Google Inc. (Intel)";
const AMD: &str = "Google Inc. (AMD)";
const W4400: &str = "ANGLE (Intel, Intel(R) HD Graphics 4400 Direct3D11 vs_5_0 ps_5_0, D3D11)";
const L4400: &str = "ANGLE (Intel, Mesa DRI Intel(R) Haswell Mobile, OpenGL 4.5 (Core Profile) Mesa 23.2.1)";
const W510: &str = "ANGLE (Intel, Intel(R) HD Graphics 510 Direct3D11 vs_5_0 ps_5_0, D3D11)";
const L510: &str = "ANGLE (Intel, Mesa Intel(R) HD Graphics 510 (SKL GT1), OpenGL 4.6 (Core Profile) Mesa 23.2.1)";
const W520: &str = "ANGLE (Intel, Intel(R) HD Graphics 520 Direct3D11 vs_5_0 ps_5_0, D3D11)";
const L520: &str = "ANGLE (Intel, Mesa Intel(R) HD Graphics 520 (SKL GT2), OpenGL 4.6 (Core Profile) Mesa 23.2.1)";
const W620: &str = "ANGLE (Intel, Intel(R) HD Graphics 620 Direct3D11 vs_5_0 ps_5_0, D3D11)";
const L620: &str = "ANGLE (Intel, Mesa Intel(R) HD Graphics 620 (KBL GT2), OpenGL 4.6 (Core Profile) Mesa 23.2.1)";
const WVEGA3: &str = "ANGLE (AMD, AMD Radeon(TM) Vega 3 Graphics Direct3D11 vs_5_0 ps_5_0, D3D11)";
const LVEGA3: &str = "ANGLE (AMD, AMD Radeon (TM) Vega 3 Graphics (radeonsi raven aco), OpenGL 4.6 (Core Profile) Mesa 23.2.1)";

macro_rules! s {
    ($model:literal, $hw:expr, $mem:expr, $ws:expr, $ls:expr, $win:expr, $lin:expr, $vendor:expr, $mhz:expr, $dhz:expr) => {
        PresetSpec {
            cpu_model: $model,
            hw_concurrency: $hw,
            device_memory: $mem,
            win_scale: $ws,
            linux_scale: $ls,
            win_renderer: $win,
            linux_renderer: $lin,
            webgl_vendor: $vendor,
            max_mouse_hz: $mhz,
            max_display_hz: $dhz,
        }
    };
}

#[rustfmt::skip]
pub const SPECS: [PresetSpec; 12] = [
    s!("i5-3320m",      4, 8, 1.08, 1.02, W4000,  L4000,  INTEL, 250, 60),
    s!("i5-4200u",      4, 8, 1.16, 1.10, W4400,  L4400,  INTEL, 250, 60),
    s!("celeron-3855u", 2, 4, 1.63, 1.55, W510,   L510,   INTEL, 125, 60),
    s!("celeron-3955u", 2, 4, 1.63, 1.55, W510,   L510,   INTEL, 125, 60),
    s!("i3-6006u",      2, 4, 1.36, 1.15, W520,   L520,   INTEL, 250, 60),
    s!("i3-6100u",      4, 8, 1.18, 1.15, W520,   L520,   INTEL, 250, 75),
    s!("i5-6200u",      4, 8, 1.06, 1.01, W520,   L520,   INTEL, 250, 75),
    s!("i5-6300u",      4, 8, 1.00, 0.95, W520,   L520,   INTEL, 250, 75),
    s!("i7-6500u",      4, 8, 0.96, 0.88, W520,   L520,   INTEL, 250, 75),
    s!("i7-6600u",      4, 8, 0.90, 0.88, W520,   L520,   INTEL, 250, 75),
    s!("i5-7200u",      4, 8, 0.90, 0.90, W620,   L620,   INTEL, 500, 60),
    s!("ryzen3-2200u",  4, 8, 0.84, 0.80, WVEGA3, LVEGA3, AMD,   500, 60),
];

impl Preset {
    #[inline]
    pub const fn spec(self) -> &'static PresetSpec {
        &SPECS[self as usize]
    }

    #[inline]
    pub const fn cpu_model(self) -> &'static str {
        self.spec().cpu_model
    }

    #[inline]
    pub const fn hw_concurrency(self) -> u8 {
        self.spec().hw_concurrency
    }

    #[inline]
    pub const fn device_memory(self) -> u8 {
        self.spec().device_memory
    }

    #[inline]
    pub fn cpu_scale(self, platform: Platform) -> f64 {
        match platform {
            Platform::Linux => self.spec().linux_scale,
            _ => self.spec().win_scale,
        }
    }

    #[inline]
    pub fn renderer(self, platform: Platform) -> &'static str {
        match platform {
            Platform::Linux => self.spec().linux_renderer,
            _ => self.spec().win_renderer,
        }
    }

    #[inline]
    pub const fn webgl_vendor(self) -> &'static str {
        self.spec().webgl_vendor
    }

    #[inline]
    pub const fn max_mouse_hz(self) -> u32 {
        self.spec().max_mouse_hz
    }

    #[inline]
    pub const fn max_display_hz(self) -> u16 {
        self.spec().max_display_hz
    }
    #[inline]
    pub fn supports(self, platform: Platform) -> bool {
        matches!(platform, Platform::Windows | Platform::Linux)
    }
}

const _: () = assert!(SPECS.len() == POOL.len());
