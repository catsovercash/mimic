//! Mimic's compact GUI. It renders from an immutable snapshot produced by the
//! controller and sends commands back without touching input devices itself.

use eframe::egui::{
    self, Align, Align2, CornerRadius, FontFamily, Frame, Label, Layout, Margin, RichText, Sense,
    Stroke, Vec2,
};

use crate::controller::{ControllerHandle, Mode, Snapshot, UiCommand};
use crate::hotkeys::HotkeyUiState;
use crate::model::format_duration;
use crate::settings::{Settings, MAX_LOOP_COUNT, MAX_LOOP_DELAY_MS, MAX_SPEED, MIN_SPEED};
use crate::theme;

pub struct MimicApp {
    controller: ControllerHandle,
    settings_open: bool,
    settings_draft: Option<Settings>,
    pending_action: Option<PendingAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingAction {
    Record,
    Open,
}

impl MimicApp {
    pub fn new(controller: ControllerHandle) -> Self {
        Self {
            controller,
            settings_open: false,
            settings_draft: None,
            pending_action: None,
        }
    }

    fn open_settings(&mut self, snapshot: &Snapshot) {
        if snapshot.mode.is_busy() {
            return;
        }
        self.settings_draft = Some(snapshot.settings.clone());
        self.settings_open = true;
    }

    fn request_action(&mut self, action: PendingAction, snapshot: &Snapshot) {
        if snapshot.dirty && snapshot.event_count > 0 {
            self.pending_action = Some(action);
        } else {
            self.perform_action(action);
        }
    }

    fn perform_action(&mut self, action: PendingAction) {
        match action {
            PendingAction::Record => self.controller.send(UiCommand::StartRecording),
            PendingAction::Open => self.controller.send(UiCommand::Open),
        }
    }

    fn handle_escape(&mut self, ctx: &egui::Context, snapshot: &Snapshot) {
        if !ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            return;
        }
        if self.pending_action.is_some() {
            self.pending_action = None;
        } else if self.settings_open {
            self.settings_open = false;
            self.settings_draft = None;
        } else if snapshot.mode.is_busy() {
            self.controller.send(UiCommand::Stop);
        }
    }
}

impl eframe::App for MimicApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let snapshot = self.controller.snapshot();

        if snapshot.mode.is_busy() {
            self.settings_open = false;
            self.settings_draft = None;
        }
        self.handle_escape(ctx, &snapshot);

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::BG)
                    .inner_margin(Margin::symmetric(20, 16)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        self.header(ui, &snapshot);
                        self.banner(ui, &snapshot);
                        ui.add_space(12.0);
                        self.timer_card(ui, &snapshot);
                        ui.add_space(10.0);
                        self.controls(ui, &snapshot);
                        ui.add_space(14.0);
                        self.footer(ui, &snapshot);
                    });
            });

        self.settings_window(ctx, &snapshot);
        self.confirm_window(ctx, &snapshot);
    }
}

impl MimicApp {
    fn header(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        ui.horizontal(|ui| {
            paint_logo(ui);
            ui.add_space(3.0);
            ui.vertical(|ui| {
                ui.add_space(1.0);
                ui.add(Label::new(
                    RichText::new("Mimic")
                        .size(23.0)
                        .strong()
                        .color(theme::TEXT),
                ));
                ui.add(Label::new(
                    RichText::new("Tiny macro recorder for KDE Plasma")
                        .size(11.5)
                        .color(theme::MUTED),
                ));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let enabled = !snapshot.mode.is_busy() && !snapshot.dialog_pending;
                let tooltip = if enabled {
                    "Settings"
                } else {
                    "Stop recording or playback to change settings"
                };
                if icon_button(ui, Icon::Settings, enabled, tooltip).clicked() {
                    self.open_settings(snapshot);
                }
            });
        });
    }

    fn banner(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let Some(banner) = &snapshot.banner else {
            return;
        };
        ui.add_space(12.0);
        let (fill, stroke, accent) = banner_colors(banner.level);
        Frame::new()
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, stroke))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    ui.painter().circle_filled(dot_rect.center(), 3.5, accent);
                    ui.vertical(|ui| {
                        ui.add(
                            Label::new(
                                RichText::new(&banner.title)
                                    .size(12.5)
                                    .strong()
                                    .color(theme::TEXT),
                            )
                            .wrap(),
                        );
                        if let Some(detail) = &banner.detail {
                            ui.add(
                                Label::new(
                                    RichText::new(detail).size(11.0).color(theme::TEXT_SOFT),
                                )
                                .wrap(),
                            );
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                        if icon_button(ui, Icon::Close, true, "Dismiss").clicked() {
                            self.controller.send(UiCommand::DismissBanner(banner.id));
                        }
                    });
                });
            });
    }

    fn timer_card(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let (label, color, badge) = state_badge(snapshot.mode, snapshot.paused);
        let timer_color = if snapshot.mode == Mode::Idle {
            theme::TEXT
        } else {
            color
        };

        Frame::new()
            .fill(theme::CARD)
            .stroke(Stroke::new(1.0_f32, theme::BORDER))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::symmetric(18, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    status_pill(ui, label, color, badge);
                    ui.add_space(4.0);
                    let name = if snapshot.recovered {
                        "Recovered recording"
                    } else {
                        snapshot.macro_name.as_str()
                    };
                    ui.add(
                        Label::new(RichText::new(name).size(12.5).color(theme::TEXT_SOFT))
                            .truncate(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if snapshot.dirty && snapshot.event_count > 0 {
                            unsaved_badge(ui);
                        }
                    });
                });

                ui.add_space(5.0);
                ui.vertical_centered(|ui| {
                    ui.add(Label::new(
                        RichText::new(format_duration(snapshot.elapsed_us))
                            .family(FontFamily::Monospace)
                            .size(45.0)
                            .strong()
                            .color(timer_color),
                    ));
                    ui.add_space(1.0);
                    let subline = secondary_line(snapshot);
                    ui.add(Label::new(
                        RichText::new(subline).size(12.0).color(theme::MUTED),
                    ));
                });
                ui.add_space(9.0);

                let fraction = if snapshot.mode == Mode::Playing {
                    let total = snapshot.macro_duration_us.max(1);
                    (snapshot.elapsed_us as f32 / total as f32).clamp(0.0, 1.0)
                } else if snapshot.mode == Mode::Recording {
                    if snapshot.paused {
                        0.55
                    } else {
                        1.0
                    }
                } else {
                    0.0
                };
                let fill_color = if snapshot.paused {
                    theme::AMBER
                } else if snapshot.mode == Mode::Playing {
                    theme::GREEN
                } else {
                    theme::RED
                };
                progress_track(ui, fraction, fill_color);
            });
    }

    fn controls(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let busy = snapshot.mode.is_busy();
        let blocked = snapshot.dialog_pending;
        Frame::new()
            .fill(theme::CARD)
            .stroke(Stroke::new(1.0_f32, theme::BORDER))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::symmetric(14, 13))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                let width = ui.available_width();
                let gap = 9.0;
                let half = ((width - gap) / 2.0).max(120.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    let record = action_button(
                        ui,
                        Vec2::new(half, 48.0),
                        "Record",
                        Icon::Record,
                        ButtonTone::Record,
                        !busy && !blocked,
                        if busy {
                            Some("A session is already running")
                        } else {
                            Some("Start capturing keyboard and mouse input")
                        },
                    );
                    if record.clicked() {
                        self.request_action(PendingAction::Record, snapshot);
                    }

                    let play_enabled = !busy && !blocked && snapshot.event_count > 0;
                    let play = action_button(
                        ui,
                        Vec2::new(half, 48.0),
                        "Play",
                        Icon::Play,
                        ButtonTone::Play,
                        play_enabled,
                        if busy {
                            Some("Stop the current session before playing")
                        } else if snapshot.event_count == 0 {
                            Some("Record or open a macro first")
                        } else {
                            Some("Replay the current recording")
                        },
                    );
                    if play.clicked() {
                        self.controller.send(UiCommand::Play);
                    }
                });

                ui.add_space(6.0);

                let count = 5usize;
                let gap = 6.0;
                let small_width =
                    ((ui.available_width() - gap * (count as f32 - 1.0)) / count as f32).max(66.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    let pause_label = if snapshot.paused { "Resume" } else { "Pause" };
                    let pause_icon = if snapshot.paused {
                        Icon::Play
                    } else {
                        Icon::Pause
                    };
                    let pause = action_button(
                        ui,
                        Vec2::new(small_width, 36.0),
                        pause_label,
                        pause_icon,
                        if snapshot.paused {
                            ButtonTone::Warm
                        } else {
                            ButtonTone::Neutral
                        },
                        busy && !blocked,
                        if busy {
                            Some("Pause or resume the current session")
                        } else {
                            Some("Available during recording or playback")
                        },
                    );
                    if pause.clicked() {
                        self.controller.send(UiCommand::PauseResume);
                    }

                    let stop = action_button(
                        ui,
                        Vec2::new(small_width, 36.0),
                        "Stop",
                        Icon::Stop,
                        ButtonTone::Danger,
                        busy,
                        if busy {
                            Some("Stop and release any held keys or buttons")
                        } else {
                            Some("Nothing is running")
                        },
                    );
                    if stop.clicked() {
                        self.controller.send(UiCommand::Stop);
                    }

                    let loop_label = loop_label(&snapshot.settings);
                    let loop_button = action_button(
                        ui,
                        Vec2::new(small_width, 36.0),
                        &loop_label,
                        Icon::Loop,
                        if snapshot.settings.loop_enabled {
                            ButtonTone::Active
                        } else {
                            ButtonTone::Neutral
                        },
                        !busy && !blocked,
                        Some(if snapshot.settings.loop_enabled {
                            "Looping is on — click to play once"
                        } else {
                            "Enable looping (count and delay are in Settings)"
                        }),
                    );
                    if loop_button.clicked() {
                        self.controller.send(UiCommand::ToggleLoop);
                    }

                    let open = action_button(
                        ui,
                        Vec2::new(small_width, 36.0),
                        "Open",
                        Icon::Folder,
                        ButtonTone::Neutral,
                        !busy && !blocked,
                        Some("Open a saved .mimic recording"),
                    );
                    if open.clicked() {
                        self.request_action(PendingAction::Open, snapshot);
                    }

                    let save_enabled = !busy && !blocked && snapshot.event_count > 0;
                    let save = action_button(
                        ui,
                        Vec2::new(small_width, 36.0),
                        "Save",
                        Icon::Save,
                        ButtonTone::Neutral,
                        save_enabled,
                        if snapshot.event_count == 0 {
                            Some("There is nothing to save yet")
                        } else {
                            Some("Save this recording as a .mimic file")
                        },
                    );
                    if save.clicked() {
                        self.controller.send(UiCommand::Save);
                    }
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let settings = &snapshot.settings;
                    let loop_text = if settings.loop_enabled {
                        match settings.loop_count {
                            0 => "∞ loops".to_owned(),
                            1 => "1 loop".to_owned(),
                            n => format!("{n} loops"),
                        }
                    } else {
                        "single pass".to_owned()
                    };
                    let summary = format!(
                        "Speed {:.2}×   ·   {}   ·   {:.2} s loop delay",
                        settings.playback_speed,
                        loop_text,
                        settings.loop_delay_ms as f64 / 1000.0
                    );
                    ui.add(
                        Label::new(RichText::new(summary).size(10.5).color(theme::FAINT))
                            .truncate(),
                    );
                });
            });
    }

    fn footer(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        if let Some(notice) = &snapshot.notice {
            ui.add_space(2.0);
            ui.add(Label::new(
                RichText::new(notice).size(11.0).color(theme::TEXT_SOFT),
            ));
            ui.add_space(5.0);
        }

        match &snapshot.hotkeys {
            HotkeyUiState::Ready(shortcuts) => {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for shortcut in shortcuts {
                        shortcut_hint(ui, shortcut);
                    }
                });
            }
            HotkeyUiState::Starting => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add(Label::new(
                        RichText::new("Registering global shortcuts…")
                            .size(10.5)
                            .color(theme::MUTED),
                    ));
                });
            }
            HotkeyUiState::AwaitingApproval => {
                ui.add(Label::new(
                    RichText::new("Waiting for KDE global-shortcut approval…")
                        .size(10.5)
                        .color(theme::AMBER),
                ));
            }
            HotkeyUiState::Unavailable(error) => {
                ui.horizontal_wrapped(|ui| {
                    ui.add(Label::new(
                        RichText::new("Global shortcuts unavailable.")
                            .size(10.5)
                            .color(theme::RED),
                    ));
                    if ui
                        .add(Label::new(
                            RichText::new("Configure")
                                .size(10.5)
                                .color(theme::BLUE)
                                .underline(),
                        ))
                        .interact(Sense::click())
                        .clicked()
                    {
                        self.open_settings(snapshot);
                    }
                    if ui
                        .add(Label::new(
                            RichText::new("Retry")
                                .size(10.5)
                                .color(theme::BLUE)
                                .underline(),
                        ))
                        .interact(Sense::click())
                        .clicked()
                    {
                        self.controller.send(UiCommand::RetryHotkeys);
                    }
                    ui.add(Label::new(
                        RichText::new(error).size(10.0).color(theme::FAINT),
                    ));
                });
            }
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(Label::new(
                RichText::new(format!("Mimic {}", env!("CARGO_PKG_VERSION")))
                    .size(10.0)
                    .color(theme::FAINT),
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(Label::new(
                    RichText::new("KDE Plasma · Wayland · evdev + uinput")
                        .size(10.0)
                        .color(theme::FAINT),
                ));
            });
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context, snapshot: &Snapshot) {
        if !self.settings_open {
            return;
        }
        let Some(mut draft) = self.settings_draft.take() else {
            self.settings_open = false;
            return;
        };
        let mut open = true;
        let mut cancel = false;
        let mut apply = false;
        let scroll_height = (ctx.screen_rect().height() - 190.0).clamp(180.0, 560.0);

        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(480.0)
            .min_width(420.0)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                egui::ScrollArea::vertical()
                    .max_height(scroll_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        section_heading(ui, "Playback");
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [112.0, 24.0],
                                Label::new(
                                    RichText::new("Speed").size(12.5).color(theme::TEXT_SOFT),
                                ),
                            );
                            ui.add(
                                egui::Slider::new(&mut draft.playback_speed, MIN_SPEED..=MAX_SPEED)
                                    .step_by(0.05)
                                    .fixed_decimals(2)
                                    .suffix("×"),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [112.0, 24.0],
                                Label::new(
                                    RichText::new("Loops").size(12.5).color(theme::TEXT_SOFT),
                                ),
                            );
                            ui.add(
                                egui::DragValue::new(&mut draft.loop_count)
                                    .range(0..=MAX_LOOP_COUNT)
                                    .speed(0.2),
                            );
                            ui.add(Label::new(
                                RichText::new("0 = repeat forever")
                                    .size(10.5)
                                    .color(theme::FAINT),
                            ));
                        });
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [112.0, 24.0],
                                Label::new(
                                    RichText::new("Loop delay")
                                        .size(12.5)
                                        .color(theme::TEXT_SOFT),
                                ),
                            );
                            ui.add(
                                egui::DragValue::new(&mut draft.loop_delay_ms)
                                    .range(0..=MAX_LOOP_DELAY_MS)
                                    .speed(10.0)
                                    .suffix(" ms"),
                            );
                        });
                        ui.add(Label::new(
                            RichText::new(
                                "Looping can also be toggled from the main window. These \
                                 settings apply to the next playback. Mouse replay uses a \
                                 flat, unaccelerated virtual pointer for 1:1 movement; use \
                                 KDE's Flat pointer profile for exact cursor reproduction.",
                            )
                            .size(10.5)
                            .color(theme::FAINT),
                        ));

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(8.0);
                        section_heading(ui, "Global shortcuts");
                        ui.add_space(3.0);
                        match &snapshot.hotkeys {
                            HotkeyUiState::Ready(shortcuts) => {
                                shortcut_table(ui, shortcuts);
                                ui.add_space(4.0);
                                if ui.button("Configure shortcuts…").clicked() {
                                    self.controller.send(UiCommand::ConfigureShortcuts);
                                }
                                ui.add(Label::new(
                                    RichText::new(
                                        "KDE manages these bindings. They also work while \
                                         Mimic is in the background or looping.",
                                    )
                                    .size(10.0)
                                    .color(theme::FAINT),
                                ));
                            }
                            HotkeyUiState::Starting => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label("Connecting to the desktop portal…");
                                });
                            }
                            HotkeyUiState::AwaitingApproval => {
                                ui.add(Label::new(
                                    RichText::new(
                                        "Waiting for you to approve shortcuts in the KDE dialog…",
                                    )
                                    .size(11.0)
                                    .color(theme::AMBER),
                                ));
                            }
                            HotkeyUiState::Unavailable(error) => {
                                ui.add(
                                    Label::new(RichText::new(error).size(11.0).color(theme::RED))
                                        .wrap(),
                                );
                                if ui.button("Retry shortcut registration").clicked() {
                                    self.controller.send(UiCommand::RetryHotkeys);
                                }
                            }
                        }

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(8.0);
                        section_heading(ui, "Input access");
                        ui.add(
                            Label::new(
                                RichText::new(
                                    "Recording reads Linux evdev devices and playback writes \
                                     to a uinput virtual device. Mimic's GUI never runs as root.",
                                )
                                .size(10.5)
                                .color(theme::TEXT_SOFT),
                            )
                            .wrap(),
                        );
                        ui.add_space(3.0);
                        ui.add(
                            Label::new(
                                RichText::new(
                                    "If capture reports permission errors, run \
                                     scripts/install-permissions.sh once. See README.md for \
                                     the security implications.",
                                )
                                .size(10.5)
                                .color(theme::MUTED),
                            )
                            .wrap(),
                        );
                    });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset defaults").clicked() {
                        let last_directory = draft.last_directory.clone();
                        draft = Settings {
                            last_directory,
                            ..Settings::default()
                        };
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Save settings").color(theme::BG))
                                    .fill(theme::GREEN)
                                    .stroke(Stroke::NONE)
                                    .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            apply = true;
                            cancel = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            });

        if apply {
            self.controller
                .send(UiCommand::ApplySettings(draft.clone()));
        }
        if cancel || !open {
            self.settings_open = false;
            self.settings_draft = None;
        } else {
            self.settings_draft = Some(draft);
        }
    }

    fn confirm_window(&mut self, ctx: &egui::Context, snapshot: &Snapshot) {
        let Some(action) = self.pending_action else {
            return;
        };
        let mut close = false;
        let mut chose = None;
        egui::Window::new("Unsaved recording")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(390.0)
            .show(ctx, |ui| {
                ui.set_width(370.0);
                ui.add(
                    Label::new(
                        RichText::new("The current recording has unsaved changes.")
                            .size(13.0)
                            .color(theme::TEXT),
                    )
                    .wrap(),
                );
                ui.add(
                    Label::new(
                        RichText::new(
                            "Save it from the main window first, or discard it and continue. \
                             Your latest recording is also backed up for crash recovery.",
                        )
                        .size(11.0)
                        .color(theme::MUTED),
                    )
                    .wrap(),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let text = match action {
                            PendingAction::Record => "Discard and record",
                            PendingAction::Open => "Discard and open",
                        };
                        if ui
                            .add(
                                egui::Button::new(RichText::new(text).color(theme::BG))
                                    .fill(theme::RED)
                                    .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            chose = Some(true);
                        }
                        if ui.button("Save first…").clicked() {
                            chose = Some(false);
                        }
                    });
                });
            });

        if let Some(discard) = chose {
            if discard {
                self.perform_action(action);
            } else {
                self.controller.send(UiCommand::Save);
            }
            close = true;
        }
        if close {
            self.pending_action = None;
        }
        let _ = snapshot;
    }
}

mod widgets;
use self::widgets::*;
