use super::*;

#[test]
fn action_ids_roundtrip() {
    for definition in default_shortcut_definitions() {
        let expected = match definition.id {
            ACTION_RECORD_TOGGLE => HotkeyAction::ToggleRecording,
            ACTION_RECORD_PAUSE => HotkeyAction::ToggleRecordingPause,
            ACTION_PLAYBACK_TOGGLE => HotkeyAction::TogglePlayback,
            ACTION_EMERGENCY_STOP => HotkeyAction::EmergencyStop,
            _ => unreachable!(),
        };
        assert_eq!(HotkeyAction::from_id(definition.id), Some(expected));
    }
    assert_eq!(HotkeyAction::from_id("unknown"), None);
}

#[test]
fn merge_fills_missing_shortcuts() {
    let merged = merge_shortcut_info(vec![ShortcutInfo {
        id: ACTION_EMERGENCY_STOP.into(),
        label: "Emergency stop".into(),
        trigger: "Ctrl+Alt+E".into(),
    }]);
    assert_eq!(merged.len(), 4);
    assert_eq!(merged[0].id, ACTION_RECORD_TOGGLE);
    assert_eq!(merged[0].trigger, "Not assigned");
    assert_eq!(merged[3].id, ACTION_EMERGENCY_STOP);
    assert_eq!(merged[3].trigger, "Ctrl+Alt+E");
}

#[test]
fn all_default_triggers_are_unique() {
    let triggers = default_shortcut_definitions()
        .iter()
        .map(|definition| definition.preferred_trigger)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(triggers.len(), 4);
}
