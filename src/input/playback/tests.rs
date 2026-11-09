use super::*;
use crate::model::RecordedEvent;

#[test]
fn pointer_anchor_targets_saved_start() {
    let target = PointerPoint { x: 640, y: 480 };
    assert_eq!(pointer_anchor_delta(target, (100, 200)), Some((540, 280)));
    assert_eq!(pointer_anchor_delta(target, (640, 480)), None);
    assert_eq!(pointer_anchor_delta(target, (700, 400)), Some((-60, 80)));
}

#[test]
fn groups_equal_timestamps() {
    let mut recording = Recording::empty();
    recording.events = vec![
        RecordedEvent::new(0, Action::Move { dx: 1, dy: 0 }),
        RecordedEvent::new(
            0,
            Action::Button {
                code: 0x110,
                pressed: true,
            },
        ),
        RecordedEvent::new(
            5_000,
            Action::Button {
                code: 0x110,
                pressed: false,
            },
        ),
    ];
    recording.duration_us = 5_000;
    let groups = group_events(&recording);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].range, 0..2);
    assert_eq!(groups[0].previous_at_us, 0);
    assert_eq!(groups[1].range, 2..3);
    assert_eq!(groups[1].previous_at_us, 0);
}

#[test]
fn speed_scaling() {
    assert_eq!(scale_micros(1_000_000, 1.0), Duration::from_secs(1));
    assert_eq!(scale_micros(1_000_000, 2.0), Duration::from_millis(500));
    assert_eq!(scale_micros(1_000_000, 0.5), Duration::from_secs(2));
}

#[test]
fn held_updates() {
    let mut held = HashSet::new();
    update_held(&mut held, 30, true);
    assert!(held.contains(&30));
    update_held(&mut held, 30, true);
    assert_eq!(held.len(), 1);
    update_held(&mut held, 30, false);
    assert!(held.is_empty());
}

#[test]
fn builds_input_events() {
    let events = events_for_actions(&[
        Action::Move { dx: 4, dy: -2 },
        Action::Scroll { dx: 0, dy: 1 },
    ]);
    assert_eq!(events.len(), 4);
    assert_eq!(events[0].code(), RelativeAxisCode::REL_X.0);
    assert_eq!(events[0].value(), 4);
    assert_eq!(events[2].code(), RelativeAxisCode::REL_HWHEEL.0);
    assert_eq!(events[3].value(), 1);
}

#[test]
fn invalid_empty_recording_is_rejected_before_device_creation() {
    let recording = Recording::empty();
    assert!(recording.is_empty());
}

/// This checks that this host can create a virtual input device. It is
/// intentionally ignored by default because it has a system side effect.
#[test]
#[ignore = "requires /dev/uinput access"]
fn can_create_virtual_device() {
    let mut recording = Recording::empty();
    recording.events = vec![
        RecordedEvent::new(
            0,
            Action::Key {
                code: KeyCode::KEY_A.0,
                pressed: true,
            },
        ),
        RecordedEvent::new(1, Action::Move { dx: 1, dy: -1 }),
        RecordedEvent::new(
            2,
            Action::Button {
                code: KeyCode::BTN_LEFT.0,
                pressed: true,
            },
        ),
    ];
    recording.duration_us = 2;
    let device = create_virtual_device(&recording);
    assert!(device.is_ok(), "{}", device.err().unwrap());
}
