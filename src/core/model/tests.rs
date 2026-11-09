use super::*;

fn sample() -> Recording {
    let mut recording = Recording::empty();
    recording.events = vec![
        RecordedEvent::new(1_000, Action::Move { dx: 4, dy: -2 }),
        RecordedEvent::new(
            1_500,
            Action::Button {
                code: BTN_LEFT,
                pressed: true,
            },
        ),
        RecordedEvent::new(
            2_000,
            Action::Button {
                code: BTN_LEFT,
                pressed: false,
            },
        ),
        RecordedEvent::new(
            5_000,
            Action::Key {
                code: 30,
                pressed: true,
            },
        ),
        RecordedEvent::new(
            7_000,
            Action::Key {
                code: 30,
                pressed: false,
            },
        ),
        RecordedEvent::new(9_000, Action::Scroll { dx: 0, dy: 1 }),
    ];
    recording.duration_us = 9_000;
    recording.pointer_start = Some(PointerPoint { x: 640, y: 480 });
    recording
}

#[test]
fn roundtrip_json() {
    let recording = sample();
    let json = serde_json::to_string_pretty(&recording).unwrap();
    let decoded: Recording = serde_json::from_str(&json).unwrap();
    assert_eq!(recording, decoded);
    assert!(json.contains("\"type\": \"key\""));
    assert!(json.contains("\"at_us\""));
    assert!(json.contains("\"pointer_start\""));
}

#[test]
fn loads_legacy_recording_without_pointer_start() {
    let mut value = serde_json::to_value(sample()).unwrap();
    value.as_object_mut().unwrap().remove("pointer_start");
    let decoded: Recording = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.pointer_start, None);
}

#[test]
fn validates_sample() {
    sample().validate().unwrap();
}

#[test]
fn rejects_incompatible_version() {
    let mut recording = sample();
    recording.version = 99;
    assert!(matches!(
        recording.validate(),
        Err(ValidationError::UnsupportedVersion(99))
    ));
}

#[test]
fn rejects_bad_format() {
    let mut recording = sample();
    recording.format = "something-else".into();
    assert!(matches!(
        recording.validate(),
        Err(ValidationError::BadFormat(_))
    ));
}

#[test]
fn rejects_non_monotonic_events() {
    let mut recording = sample();
    recording.events[2].at_us = 1_000;
    assert!(matches!(
        recording.validate(),
        Err(ValidationError::NonMonotonic { .. })
    ));
}

#[test]
fn rejects_out_of_range_key_and_button() {
    let mut recording = sample();
    recording.events[0] = RecordedEvent::new(
        1_000,
        Action::Key {
            code: 0,
            pressed: true,
        },
    );
    assert!(matches!(
        recording.validate(),
        Err(ValidationError::BadKeyCode { .. })
    ));

    let mut recording = sample();
    recording.events[0] = RecordedEvent::new(
        1_000,
        Action::Button {
            code: 0x100,
            pressed: true,
        },
    );
    assert!(matches!(
        recording.validate(),
        Err(ValidationError::BadButtonCode { .. })
    ));
}

#[test]
fn rejects_empty_movement() {
    let mut recording = sample();
    recording.events[0] = RecordedEvent::new(1_000, Action::Move { dx: 0, dy: 0 });
    assert!(matches!(
        recording.validate(),
        Err(ValidationError::EmptyMove { .. })
    ));
}

#[test]
fn rejects_duration_shorter_than_events() {
    let mut recording = sample();
    recording.duration_us = 500;
    assert!(matches!(
        recording.validate(),
        Err(ValidationError::DurationShorterThanEvents { .. })
    ));
}

#[test]
fn format_duration_is_readable() {
    assert_eq!(format_duration(0), "00:00.000");
    assert_eq!(format_duration(1_234_567), "00:01.234");
    assert_eq!(format_duration(61_500_000), "01:01.500");
    assert_eq!(format_duration(3_661_000_000), "1:01:01.000");
}

#[test]
fn finish_never_shortens_below_last_event() {
    let mut recording = Recording::empty();
    recording
        .events
        .push(RecordedEvent::new(50, Action::Move { dx: 1, dy: 1 }));
    recording.finish(10);
    assert_eq!(recording.duration_us, 50);
}
