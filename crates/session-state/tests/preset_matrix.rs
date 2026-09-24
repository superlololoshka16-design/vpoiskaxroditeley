use session_state::{Platform, Preset};

#[test]
fn preset_scale_table_matches_law() {
    assert_eq!(Preset::I3_6100U.cpu_scale(Platform::Windows), 1.18);
    assert_eq!(Preset::I5_6200U.cpu_scale(Platform::Windows), 1.06);
    assert_eq!(Preset::I5_6300U.cpu_scale(Platform::Windows), 1.00);
    assert_eq!(Preset::I7_6500U.cpu_scale(Platform::Windows), 0.96);
    assert_eq!(Preset::I7_6600U.cpu_scale(Platform::Windows), 0.90);
    assert_eq!(Preset::Celeron3855U.cpu_scale(Platform::Linux), 1.55);
    assert_eq!(Preset::Celeron3955U.cpu_scale(Platform::Linux), 1.55);
    assert_eq!(Preset::I3_6006U.cpu_scale(Platform::Linux), 1.15);
    assert_eq!(Preset::I3_6100U.cpu_scale(Platform::Linux), 1.15);
    assert_eq!(Preset::I5_6300U.cpu_scale(Platform::Linux), 0.95);
    assert_eq!(Preset::I7_6500U.cpu_scale(Platform::Linux), 0.88);
    assert_eq!(Preset::I7_6600U.cpu_scale(Platform::Linux), 0.88);
    assert_eq!(Preset::I5_7200U.cpu_scale(Platform::Linux), 0.90);
}

#[test]
fn presets_support_exactly_windows_and_linux() {
    for p in session_state::POOL {
        assert!(
            p.supports(Platform::Windows),
            "{} must support windows",
            p.cpu_model()
        );
        assert!(
            p.supports(Platform::Linux),
            "{} must support linux",
            p.cpu_model()
        );
        assert!(
            !p.supports(Platform::MacOS),
            "{} must not support macos",
            p.cpu_model()
        );
        assert!(
            !p.supports(Platform::Android),
            "{} must not support android",
            p.cpu_model()
        );
    }
}

#[test]
fn preset_matrix_is_sane() {
    assert_eq!(Preset::I5_6300U.cpu_model(), "i5-6300u");
    assert_eq!(Preset::Ryzen3_2200U.cpu_scale(Platform::Windows), 0.84);
    for p in session_state::POOL.iter() {
        let spec = p.spec();
        assert!(!spec.cpu_model.is_empty());
        assert!(spec.hw_concurrency == 2 || spec.hw_concurrency == 4);
        assert!(spec.win_scale > 0.5 && spec.win_scale < 2.0);
        assert!(spec.linux_scale > 0.5 && spec.linux_scale < 2.0);
        assert!(spec.win_renderer.contains("Direct3D11"));
        assert!(spec.linux_renderer.contains("Mesa"));
        assert!(!spec.linux_renderer.contains("llvmpipe"));
        assert!(!spec.linux_renderer.contains("swrast"));
    }
    assert_eq!(Preset::Celeron3855U.hw_concurrency(), 2);
    assert_eq!(Preset::Celeron3955U.hw_concurrency(), 2);
    assert_eq!(Preset::I3_6006U.hw_concurrency(), 2);
    assert_eq!(Preset::I5_6300U.hw_concurrency(), 4);
    assert_eq!(Preset::I3_6006U.device_memory(), 4);
    assert_eq!(Preset::Celeron3855U.device_memory(), 4);
    assert_eq!(Preset::I5_6300U.device_memory(), 8);
    assert!(
        Preset::I5_7200U
            .renderer(Platform::Windows)
            .contains("HD Graphics 620")
    );
    assert!(
        Preset::I5_6300U
            .renderer(Platform::Windows)
            .contains("HD Graphics 520")
    );
    assert!(
        Preset::I5_3320M
            .renderer(Platform::Windows)
            .contains("HD Graphics 4000")
    );
    assert!(
        Preset::Ryzen3_2200U
            .renderer(Platform::Linux)
            .contains("Vega 3")
    );
    assert!(Preset::Ryzen3_2200U.webgl_vendor().contains("AMD"));
    assert!(Preset::I5_6300U.webgl_vendor().contains("Intel"));
    assert_eq!(Preset::Celeron3855U.cpu_scale(Platform::Linux), 1.55);
}
