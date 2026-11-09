use super::*;
use crate::model::{Action, RecordedEvent};

fn temp_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("mimic-test-{name}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn recording() -> Recording {
    let mut recording = Recording::empty();
    recording.events = vec![
        RecordedEvent::new(1_000, Action::Move { dx: 3, dy: -1 }),
        RecordedEvent::new(
            2_000,
            Action::Key {
                code: 30,
                pressed: true,
            },
        ),
    ];
    recording.duration_us = 2_500;
    recording
}

#[test]
fn saves_loads_and_overwrites_atomically() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("test.mimic");
    let original = recording();
    save_recording(&path, &original).unwrap();
    assert_eq!(load_recording(&path).unwrap(), original);

    let mut updated = original.clone();
    updated.name = Some("Updated".into());
    save_recording(&path, &updated).unwrap();
    assert_eq!(load_recording(&path).unwrap(), updated);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_malformed_json() {
    let dir = temp_dir("malformed");
    let path = dir.join("bad.mimic");
    fs::write(&path, b"{not json").unwrap();
    assert!(matches!(
        load_recording(&path),
        Err(StorageError::Malformed(_))
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_wrong_format() {
    let dir = temp_dir("wrongformat");
    let path = dir.join("bad.mimic");
    let mut value = serde_json::to_value(recording()).unwrap();
    value["format"] = "not-mimic".into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let error = load_recording(&path).unwrap_err();
    assert!(matches!(
        error,
        StorageError::Invalid(crate::error::ValidationError::BadFormat(_))
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_invalid_recording_before_save() {
    let dir = temp_dir("invalid-save");
    let path = dir.join("invalid.mimic");
    let mut invalid = recording();
    invalid.version = 999;
    assert!(save_recording(&path, &invalid).is_err());
    assert!(!path.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn adds_extension_when_missing() {
    assert_eq!(
        ensure_extension(PathBuf::from("my macro")),
        PathBuf::from("my macro.mimic")
    );
    assert_eq!(
        ensure_extension(PathBuf::from("my.macro.json")),
        PathBuf::from("my.macro.json")
    );
}
