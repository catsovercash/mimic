use super::*;

#[test]
fn frame_groups_motion_and_scroll() {
    let mut frame = FrameBuffer::default();
    frame.add_motion(3, 4);
    frame.push_action(Action::Button {
        code: 0x110,
        pressed: true,
    });
    frame.add_motion(-1, 2);
    frame.add_scroll(0, 1);
    let actions = frame.take();
    assert_eq!(
        actions,
        vec![
            Action::Move { dx: 2, dy: 6 },
            Action::Button {
                code: 0x110,
                pressed: true
            },
            Action::Scroll { dx: 0, dy: 1 }
        ]
    );
    assert!(frame.is_empty());
}

#[test]
fn frame_drops_empty_motion() {
    let mut frame = FrameBuffer::default();
    frame.add_motion(0, 0);
    assert!(frame.take().is_empty());
}

#[test]
fn sysfs_bitmap_parsing_matches_low_word() {
    // This is the shape of sysfs output for a mouse with BTN_LEFT..BTN_TASK
    // set (word index 4 when represented low-to-high).
    let words = ["1f0000", "0", "0", "0", "0"];
    assert!(bitmap_has(&words, 0x110));
    assert!(bitmap_has(&words, 0x111));
    assert!(!bitmap_has(&words, 0x112 + 10));
}

#[test]
fn sysfs_capabilities_ignore_unusable_mouse_interface() {
    // The XCTECH mouse exposes an extra event node with ABS axes only and
    // no key or relative-axis capabilities. It must not be reported as a
    // missing keyboard/mouse.
    assert!(!sysfs_capabilities_look_relevant("", "0"));
    assert!(!sysfs_capabilities_look_relevant("0", "0"));
}

#[test]
fn sysfs_capabilities_ignore_media_only_endpoint() {
    // A microphone's media buttons are not a keyboard for permission
    // diagnostics. Key 164 (KEY_PLAYPAUSE) lives in the third 64-bit word.
    assert!(!sysfs_capabilities_look_relevant("1000000000 0 0", "0"));
}

#[test]
fn sysfs_capabilities_detect_keyboard_and_mouse() {
    // KEY_A = 30 in the first (lowest) 64-bit word.
    let keyboard = format!("0 0 {:x}", 1u64 << 30);
    assert!(sysfs_capabilities_look_relevant(&keyboard, "0"));

    // BTN_LEFT = 272: word 4 of the low-to-high bitmap, printed first by
    // sysfs. REL_X = 0.
    assert!(sysfs_capabilities_look_relevant("10000 0 0 0 0", "1"));
    assert!(!sysfs_capabilities_look_relevant("10000 0 0 0 0", "0"));
}

#[test]
fn key_classification_ignores_buttons() {
    assert!(is_keyboard_key(30));
    assert!(is_keyboard_key(164));
    assert!(is_keyboard_key(0x2ff));
    assert!(!is_keyboard_key(0));
    assert!(!is_keyboard_key(0x110));
}

#[test]
fn timeline_pause_excludes_time() {
    let mut timeline = Timeline::start();
    std::thread::sleep(Duration::from_millis(10));
    timeline.pause();
    let paused_at = timeline.elapsed_us();
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(timeline.elapsed_us(), paused_at);
    timeline.resume();
    std::thread::sleep(Duration::from_millis(10));
    let elapsed = timeline.elapsed_us();
    assert!((paused_at..paused_at + 100_000).contains(&elapsed));
}
