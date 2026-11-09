use super::*;

#[test]
fn sanitizes_extremes() {
    let settings = Settings {
        playback_speed: f32::INFINITY,
        loop_enabled: true,
        loop_count: 100_000,
        loop_delay_ms: 100_000,
        last_directory: PathBuf::new(),
    }
    .sanitized();
    assert_eq!(settings.playback_speed, 1.0);
    assert_eq!(settings.loop_count, MAX_LOOP_COUNT);
    assert_eq!(settings.loop_delay_ms, MAX_LOOP_DELAY_MS);
    assert!(!settings.last_directory.as_os_str().is_empty());
}

#[test]
fn loop_semantics() {
    let mut settings = Settings {
        loop_enabled: false,
        ..Settings::default()
    };
    assert_eq!(settings.loop_passes(), Some(1));

    settings.loop_enabled = true;
    settings.loop_count = 0;
    assert_eq!(settings.loop_passes(), None);

    settings.loop_count = 3;
    assert_eq!(settings.loop_passes(), Some(3));

    settings.loop_count = 1;
    assert_eq!(settings.loop_passes(), Some(1));
}

#[test]
fn missing_file_is_default() {
    let path = std::env::temp_dir().join("mimic-settings-does-not-exist.json");
    let (settings, warning) = Settings::load_from(&path);
    assert_eq!(settings, Settings::default());
    assert!(warning.is_none());
}

#[test]
fn save_load_roundtrip_and_clamp() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("mimic-settings-{stamp}.json"));
    let settings = Settings {
        playback_speed: 2.25,
        loop_enabled: true,
        loop_count: 7,
        loop_delay_ms: 250,
        last_directory: PathBuf::from("/tmp"),
    };
    settings.save_to(&path).unwrap();
    let (loaded, warning) = Settings::load_from(&path);
    assert!(warning.is_none());
    assert_eq!(loaded, settings);

    std::fs::write(&path, "not-json").unwrap();
    let (fallback, warning) = Settings::load_from(&path);
    assert_eq!(fallback, Settings::default());
    assert!(warning.is_some());
    let _ = std::fs::remove_file(path);
}
