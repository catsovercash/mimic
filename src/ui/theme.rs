//! Visual identity for Mimic's compact desktop window.

use eframe::egui::{self, Color32, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2};

pub const BG: Color32 = Color32::from_rgb(10, 12, 17);
pub const CARD: Color32 = Color32::from_rgb(18, 21, 29);
pub const CARD_RAISED: Color32 = Color32::from_rgb(23, 27, 37);
pub const INPUT: Color32 = Color32::from_rgb(14, 17, 23);
pub const BORDER: Color32 = Color32::from_rgb(37, 43, 56);
pub const BORDER_SOFT: Color32 = Color32::from_rgb(29, 34, 45);
pub const TEXT: Color32 = Color32::from_rgb(235, 239, 247);
pub const TEXT_SOFT: Color32 = Color32::from_rgb(190, 198, 214);
pub const MUTED: Color32 = Color32::from_rgb(130, 140, 158);
pub const FAINT: Color32 = Color32::from_rgb(83, 92, 109);
pub const RED: Color32 = Color32::from_rgb(255, 92, 108);
pub const RED_DARK: Color32 = Color32::from_rgb(58, 24, 31);
pub const GREEN: Color32 = Color32::from_rgb(91, 224, 160);
pub const GREEN_DARK: Color32 = Color32::from_rgb(19, 48, 38);
pub const AMBER: Color32 = Color32::from_rgb(244, 197, 105);
pub const AMBER_DARK: Color32 = Color32::from_rgb(48, 39, 20);
pub const BLUE: Color32 = Color32::from_rgb(120, 169, 255);

pub fn install(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::dark();

    visuals.panel_fill = BG;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = INPUT;
    visuals.faint_bg_color = CARD_RAISED;
    visuals.code_bg_color = INPUT;
    visuals.override_text_color = Some(TEXT);
    visuals.hyperlink_color = BLUE;
    visuals.warn_fg_color = AMBER;
    visuals.error_fg_color = RED;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_corner_radius = egui::CornerRadius::same(14);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(120, 169, 255, 64);
    visuals.selection.stroke = Stroke::new(1.0_f32, BLUE);
    visuals.slider_trailing_fill = true;

    style.visuals = visuals;
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(23.0, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(11.5, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(12.5, FontFamily::Monospace),
    );

    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(13.0, 7.0);
    style.spacing.interact_size = Vec2::new(42.0, 32.0);
    style.spacing.slider_width = 240.0;
    style.spacing.window_margin = Margin::same(16);
    style.spacing.menu_margin = Margin::same(8);
    style.animation_time = 0.12;

    let widgets = &mut style.visuals.widgets;
    widgets.noninteractive.bg_fill = CARD;
    widgets.noninteractive.weak_bg_fill = CARD;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER_SOFT);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_SOFT);
    widgets.inactive.bg_fill = CARD_RAISED;
    widgets.inactive.weak_bg_fill = CARD_RAISED;
    widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT_SOFT);
    widgets.inactive.corner_radius = egui::CornerRadius::same(8);
    widgets.hovered.bg_fill = Color32::from_rgb(31, 37, 50);
    widgets.hovered.weak_bg_fill = Color32::from_rgb(31, 37, 50);
    widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(56, 66, 86));
    widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    widgets.hovered.corner_radius = egui::CornerRadius::same(8);
    widgets.active.bg_fill = Color32::from_rgb(38, 45, 61);
    widgets.active.weak_bg_fill = Color32::from_rgb(38, 45, 61);
    widgets.active.bg_stroke = Stroke::new(1.0_f32, BLUE.gamma_multiply(0.7));
    widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    widgets.active.corner_radius = egui::CornerRadius::same(8);
    widgets.open.bg_fill = CARD_RAISED;
    widgets.open.bg_stroke = Stroke::new(1.0_f32, BORDER);
    widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);
    widgets.open.corner_radius = egui::CornerRadius::same(8);

    ctx.set_style(style);
}

/// A small RGBA icon generated at runtime so the binary has no image-decoding
/// dependency and still gets a proper window icon.
pub fn window_icon() -> egui::IconData {
    const SIZE: usize = 128;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let fx = (x as f32 + 0.5) / SIZE as f32;
            let fy = (y as f32 + 0.5) / SIZE as f32;
            // Rounded dark tile.
            let corner = 0.18;
            let inside = (fx > corner || fy > corner || (fx - corner).hypot(fy - corner) < corner)
                && (fx < 1.0 - corner
                    || fy > corner
                    || (fx - (1.0 - corner)).hypot(fy - corner) < corner)
                && (fx > corner
                    || fy < 1.0 - corner
                    || (fx - corner).hypot(fy - (1.0 - corner)) < corner)
                && (fx < 1.0 - corner
                    || fy < 1.0 - corner
                    || (fx - (1.0 - corner)).hypot(fy - (1.0 - corner)) < corner);
            if !inside {
                continue;
            }
            let mut color: [f32; 4] = [19.0, 23.0, 32.0, 255.0];
            let (cx, cy) = (0.38, 0.50);
            if (fx - cx).hypot(fy - cy) < 0.20 {
                color = [255.0, 92.0, 108.0, 255.0];
            }
            // Play triangle on the right.
            let (px, py) = (0.54, 0.50);
            let dx = fx - px;
            let dy = (fy - py).abs();
            if dx > 0.0 && dx < 0.26 && dy < dx * 0.78 {
                color = [91.0, 224.0, 160.0, 255.0];
            }
            let index = (y * SIZE + x) * 4;
            rgba[index] = color[0].round() as u8;
            rgba[index + 1] = color[1].round() as u8;
            rgba[index + 2] = color[2].round() as u8;
            rgba[index + 3] = color[3].round() as u8;
        }
    }
    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}
