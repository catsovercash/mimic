//! Reusable painting and formatting helpers for Mimic's egui interface.

use std::f32::consts::PI;

use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Frame, Label, Margin, Pos2, Rect,
    RichText, Sense, Stroke, StrokeKind, Vec2,
};

use crate::controller::{BannerLevel, Mode, Snapshot};
use crate::hotkeys::ShortcutInfo;
use crate::settings::Settings;
use crate::theme;

pub(super) fn state_badge(mode: Mode, paused: bool) -> (&'static str, Color32, &'static str) {
    match (mode, paused) {
        (Mode::Idle, _) => ("IDLE", theme::MUTED, ""),
        (Mode::Recording, true) => ("PAUSED", theme::AMBER, "Recording is paused"),
        (Mode::Recording, false) => ("RECORDING", theme::RED, "Capturing input"),
        (Mode::Playing, true) => ("PAUSED", theme::AMBER, "Playback is paused"),
        (Mode::Playing, false) => ("PLAYING", theme::GREEN, "Replaying macro"),
    }
}

pub(super) fn secondary_line(snapshot: &Snapshot) -> String {
    let events = format!(
        "{} event{}",
        format_count(snapshot.event_count),
        plural(snapshot.event_count)
    );
    match snapshot.mode {
        Mode::Recording => {
            if snapshot.paused {
                format!("{events} captured · paused")
            } else if snapshot.devices.is_empty() {
                format!("{events} captured · waiting for input devices")
            } else {
                format!(
                    "{events} captured · {} input device{}",
                    snapshot.devices.len(),
                    plural(snapshot.devices.len())
                )
            }
        }
        Mode::Playing => {
            let passes = match snapshot.playback_passes {
                None => "∞".to_owned(),
                Some(1) => "1".to_owned(),
                Some(total) => total.to_string(),
            };
            format!(
                "{events} · loop {}/{} · {:.2}× speed",
                snapshot.playback_pass, passes, snapshot.settings.playback_speed
            )
        }
        Mode::Idle => {
            if snapshot.event_count == 0 {
                "No recording loaded · Record or Open to begin".to_owned()
            } else {
                format!("{events} · ready to play")
            }
        }
    }
}

pub(super) fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

pub(super) fn format_count(count: usize) -> String {
    let raw = count.to_string();
    let mut out = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, ch) in raw.chars().enumerate() {
        if index > 0 && (raw.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub(super) fn loop_label(settings: &Settings) -> String {
    if !settings.loop_enabled {
        "Loop".to_owned()
    } else if settings.loop_count == 0 {
        "Loop ∞".to_owned()
    } else {
        format!("Loop {}×", settings.loop_count.max(1))
    }
}

pub(super) fn progress_track(ui: &mut egui::Ui, fraction: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, theme::BORDER_SOFT);
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * fraction, rect.height()));
        painter.rect_filled(fill, 2.0, color);
    }
}

pub(super) fn status_pill(ui: &mut egui::Ui, label: &str, color: Color32, tooltip: &str) {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::new(10.5, FontFamily::Proportional),
        color,
    );
    let size = Vec2::new(galley.size().x + 24.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 24);
    ui.painter().rect_filled(rect, 11.0, bg);
    ui.painter()
        .circle_filled(Pos2::new(rect.left() + 10.0, rect.center().y), 3.0, color);
    let text_pos = Pos2::new(rect.left() + 17.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(text_pos, galley, color);
    if !tooltip.is_empty() {
        response.on_hover_text(tooltip);
    }
}

pub(super) fn unsaved_badge(ui: &mut egui::Ui) {
    let galley = ui.painter().layout_no_wrap(
        "UNSAVED".to_owned(),
        FontId::new(9.5, FontFamily::Proportional),
        theme::AMBER,
    );
    let size = Vec2::new(galley.size().x + 12.0, 18.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(
        rect,
        5.0,
        Color32::from_rgba_unmultiplied(theme::AMBER.r(), theme::AMBER.g(), theme::AMBER.b(), 20),
    );
    ui.painter().galley(
        Pos2::new(rect.left() + 6.0, rect.center().y - galley.size().y * 0.5),
        galley,
        theme::AMBER,
    );
}

pub(super) fn shortcut_hint(ui: &mut egui::Ui, shortcut: &ShortcutInfo) {
    let display = if shortcut.trigger == "Not assigned" {
        "—".to_owned()
    } else {
        shortcut.trigger.clone()
    };
    Frame::new()
        .fill(theme::INPUT)
        .stroke(Stroke::new(1.0_f32, theme::BORDER_SOFT))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.add(Label::new(
                RichText::new(display)
                    .family(FontFamily::Monospace)
                    .size(9.5)
                    .color(theme::TEXT_SOFT),
            ));
        });
    let label = match shortcut.id.as_str() {
        crate::hotkeys::ACTION_RECORD_TOGGLE => "Record",
        crate::hotkeys::ACTION_RECORD_PAUSE => "Pause",
        crate::hotkeys::ACTION_PLAYBACK_TOGGLE => "Play",
        crate::hotkeys::ACTION_EMERGENCY_STOP => "E-stop",
        _ => shortcut.label.as_str(),
    };
    ui.add(Label::new(
        RichText::new(label).size(9.5).color(theme::MUTED),
    ));
    ui.add_space(3.0);
}

pub(super) fn shortcut_table(ui: &mut egui::Ui, shortcuts: &[ShortcutInfo]) {
    egui::Grid::new("mimic-shortcut-grid")
        .num_columns(2)
        .spacing([14.0, 5.0])
        .show(ui, |ui| {
            for shortcut in shortcuts {
                ui.add(Label::new(
                    RichText::new(&shortcut.label)
                        .size(11.0)
                        .color(theme::TEXT_SOFT),
                ));
                let display = if shortcut.trigger == "Not assigned" {
                    "Not assigned".to_owned()
                } else {
                    shortcut.trigger.clone()
                };
                ui.add(Label::new(
                    RichText::new(display)
                        .family(FontFamily::Monospace)
                        .size(10.5)
                        .color(theme::TEXT),
                ));
                ui.end_row();
            }
        });
}

pub(super) fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.add(Label::new(
        RichText::new(text.to_ascii_uppercase())
            .size(10.0)
            .strong()
            .color(theme::FAINT),
    ));
}

pub(super) fn banner_colors(level: BannerLevel) -> (Color32, Color32, Color32) {
    match level {
        BannerLevel::Info => (
            Color32::from_rgb(18, 27, 39),
            Color32::from_rgb(39, 62, 84),
            theme::BLUE,
        ),
        BannerLevel::Warning => (
            Color32::from_rgb(35, 29, 18),
            Color32::from_rgb(75, 58, 28),
            theme::AMBER,
        ),
        BannerLevel::Error => (
            Color32::from_rgb(38, 20, 24),
            Color32::from_rgb(83, 37, 45),
            theme::RED,
        ),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ButtonTone {
    Record,
    Play,
    Neutral,
    Danger,
    Warm,
    Active,
}

pub(super) fn action_button(
    ui: &mut egui::Ui,
    size: Vec2,
    label: &str,
    icon: Icon,
    tone: ButtonTone,
    enabled: bool,
    tooltip: Option<&str>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();

    let (base, hover, text_color, stroke) = match tone {
        ButtonTone::Record => (
            Color32::from_rgb(210, 61, 77),
            Color32::from_rgb(230, 76, 92),
            Color32::from_rgb(255, 248, 249),
            Stroke::new(1.0_f32, Color32::from_rgb(244, 119, 132)),
        ),
        ButtonTone::Play => (
            Color32::from_rgb(65, 208, 149),
            Color32::from_rgb(92, 226, 170),
            Color32::from_rgb(8, 24, 17),
            Stroke::new(1.0_f32, Color32::from_rgb(132, 241, 195)),
        ),
        ButtonTone::Neutral => (
            theme::CARD_RAISED,
            Color32::from_rgb(31, 37, 50),
            theme::TEXT_SOFT,
            Stroke::new(1.0_f32, theme::BORDER),
        ),
        ButtonTone::Danger => (
            Color32::from_rgb(33, 23, 27),
            Color32::from_rgb(47, 28, 34),
            theme::RED,
            Stroke::new(1.0_f32, Color32::from_rgb(72, 38, 46)),
        ),
        ButtonTone::Warm => (
            Color32::from_rgb(42, 34, 19),
            Color32::from_rgb(55, 44, 24),
            theme::AMBER,
            Stroke::new(1.0_f32, Color32::from_rgb(90, 70, 33)),
        ),
        ButtonTone::Active => (
            Color32::from_rgb(20, 44, 37),
            Color32::from_rgb(24, 54, 45),
            theme::GREEN,
            Stroke::new(1.0_f32, Color32::from_rgb(40, 88, 71)),
        ),
    };

    let (fill, text_color, stroke) = if !enabled {
        (
            Color32::from_rgb(18, 21, 28),
            theme::FAINT,
            Stroke::new(1.0_f32, theme::BORDER_SOFT),
        )
    } else if hovered {
        (hover, text_color, stroke)
    } else {
        (base, text_color, stroke)
    };

    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(10),
        fill,
        stroke,
        StrokeKind::Inside,
    );
    if enabled && hovered {
        painter.rect_stroke(
            rect,
            CornerRadius::same(10),
            Stroke::new(1.0_f32, text_color.gamma_multiply(0.35)),
            StrokeKind::Inside,
        );
    }

    let font = FontId::new(
        if size.y >= 44.0 { 14.5 } else { 12.5 },
        FontFamily::Proportional,
    );
    let galley = painter.layout_no_wrap(label.to_owned(), font, text_color);
    let icon_size = if size.y >= 44.0 { 18.0 } else { 15.0 };
    let gap = if size.y >= 44.0 { 9.0 } else { 6.0 };
    let content_width = icon_size + gap + galley.size().x;
    let start_x = rect.center().x - content_width * 0.5;
    let icon_center = Pos2::new(start_x + icon_size * 0.5, rect.center().y);
    paint_icon(painter, icon, icon_center, icon_size, text_color);
    painter.galley(
        Pos2::new(
            start_x + icon_size + gap,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        text_color,
    );

    let mut response = response;
    if enabled {
        response = response.on_hover_cursor(CursorIcon::PointingHand);
    }
    if let Some(tooltip) = tooltip {
        response = response.on_hover_text(tooltip);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response
}

pub(super) fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    enabled: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(32.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 9.0, theme::CARD_RAISED);
        painter.rect_stroke(
            rect,
            CornerRadius::same(9),
            Stroke::new(1.0_f32, theme::BORDER),
            StrokeKind::Inside,
        );
    }
    let color = if enabled {
        if hovered {
            theme::TEXT
        } else {
            theme::TEXT_SOFT
        }
    } else {
        theme::FAINT
    };
    paint_icon(painter, icon, rect.center(), 17.0, color);
    let mut response = response;
    if enabled {
        response = response.on_hover_cursor(CursorIcon::PointingHand);
    }
    response.on_hover_text(tooltip)
}

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Record,
    Play,
    Pause,
    Stop,
    Loop,
    Folder,
    Save,
    Settings,
    Close,
}

pub(super) fn paint_icon(
    painter: &egui::Painter,
    icon: Icon,
    center: Pos2,
    size: f32,
    color: Color32,
) {
    let stroke = Stroke::new((size * 0.11).max(1.4), color);
    match icon {
        Icon::Record => {
            painter.circle_filled(center, size * 0.31, color);
        }
        Icon::Play => {
            let points = vec![
                Pos2::new(center.x - size * 0.22, center.y - size * 0.34),
                Pos2::new(center.x + size * 0.34, center.y),
                Pos2::new(center.x - size * 0.22, center.y + size * 0.34),
            ];
            painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
        }
        Icon::Pause => {
            let bar_width = size * 0.15;
            let bar_height = size * 0.62;
            painter.rect_filled(
                Rect::from_center_size(
                    Pos2::new(center.x - size * 0.16, center.y),
                    Vec2::new(bar_width, bar_height),
                ),
                1.5,
                color,
            );
            painter.rect_filled(
                Rect::from_center_size(
                    Pos2::new(center.x + size * 0.16, center.y),
                    Vec2::new(bar_width, bar_height),
                ),
                1.5,
                color,
            );
        }
        Icon::Stop => {
            painter.rect_filled(
                Rect::from_center_size(center, Vec2::splat(size * 0.54)),
                2.5,
                color,
            );
        }
        Icon::Loop => {
            let radius = size * 0.34;
            let mut points = Vec::new();
            let start = -PI * 0.55;
            let end = PI * 1.25;
            let steps = 24;
            for index in 0..=steps {
                let t = index as f32 / steps as f32;
                let angle = start + (end - start) * t;
                points.push(Pos2::new(
                    center.x + radius * angle.cos(),
                    center.y + radius * angle.sin(),
                ));
            }
            let arrow_tip = *points.last().unwrap();
            let before = points[points.len() - 2];
            painter.add(egui::Shape::line(points, stroke));
            let dir = (arrow_tip - before).normalized();
            let normal = Vec2::new(-dir.y, dir.x);
            let base = arrow_tip - dir * size * 0.24;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    arrow_tip + dir * size * 0.04,
                    base + normal * size * 0.13,
                    base - normal * size * 0.13,
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Folder => {
            let rect = Rect::from_center_size(
                Pos2::new(center.x, center.y + size * 0.03),
                Vec2::new(size * 0.74, size * 0.54),
            );
            painter.rect_stroke(rect, CornerRadius::same(2), stroke, StrokeKind::Middle);
            painter.line_segment(
                [
                    Pos2::new(rect.left(), rect.top()),
                    Pos2::new(rect.left() + size * 0.28, rect.top()),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(rect.left() + size * 0.28, rect.top()),
                    Pos2::new(rect.left() + size * 0.34, rect.top() - size * 0.12),
                ],
                stroke,
            );
        }
        Icon::Save => {
            let rect = Rect::from_center_size(center, Vec2::splat(size * 0.66));
            painter.rect_stroke(rect, CornerRadius::same(2), stroke, StrokeKind::Middle);
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(rect.left() + size * 0.17, rect.top() + size * 0.06),
                    Vec2::new(size * 0.28, size * 0.19),
                ),
                1.0,
                color,
            );
            let bottom = Rect::from_min_size(
                Pos2::new(rect.left() + size * 0.15, rect.bottom() - size * 0.24),
                Vec2::new(size * 0.36, size * 0.18),
            );
            painter.rect_filled(bottom, 1.0, color.gamma_multiply(0.75));
        }
        Icon::Settings => {
            let line_count = 3;
            for index in 0..line_count {
                let y = center.y + (index as f32 - 1.0) * size * 0.26;
                painter.line_segment(
                    [
                        Pos2::new(center.x - size * 0.36, y),
                        Pos2::new(center.x + size * 0.36, y),
                    ],
                    Stroke::new(1.3_f32, color),
                );
                let knob_x = match index {
                    0 => center.x - size * 0.12,
                    1 => center.x + size * 0.16,
                    _ => center.x - size * 0.03,
                };
                painter.circle_filled(Pos2::new(knob_x, y), size * 0.1, theme::CARD);
                painter.circle_stroke(
                    Pos2::new(knob_x, y),
                    size * 0.1,
                    Stroke::new(1.3_f32, color),
                );
            }
        }
        Icon::Close => {
            painter.line_segment(
                [
                    Pos2::new(center.x - size * 0.22, center.y - size * 0.22),
                    Pos2::new(center.x + size * 0.22, center.y + size * 0.22),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + size * 0.22, center.y - size * 0.22),
                    Pos2::new(center.x - size * 0.22, center.y + size * 0.22),
                ],
                stroke,
            );
        }
    }
}

pub(super) fn paint_logo(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 12.0, theme::CARD_RAISED);
    painter.rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0_f32, theme::BORDER),
        StrokeKind::Inside,
    );
    painter.circle_filled(
        Pos2::new(rect.center().x - 4.5, rect.center().y),
        7.0,
        theme::RED,
    );
    let points = vec![
        Pos2::new(rect.center().x + 2.0, rect.center().y - 7.5),
        Pos2::new(rect.center().x + 12.5, rect.center().y),
        Pos2::new(rect.center().x + 2.0, rect.center().y + 7.5),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        theme::GREEN,
        Stroke::NONE,
    ));
}
