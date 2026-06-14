//! Visual theme + fonts for the launcher (egui).

use eframe::egui;
use egui::{Color32, FontData, FontDefinitions, FontFamily, Rounding, Stroke};

pub const ACCENT: Color32 = Color32::from_rgb(0x4c, 0x93, 0xf6);
pub const BG: Color32 = Color32::from_rgb(0x0f, 0x11, 0x14);
pub const SURFACE: Color32 = Color32::from_rgb(0x17, 0x1a, 0x1f);
pub const SURFACE_HI: Color32 = Color32::from_rgb(0x21, 0x26, 0x2d);
pub const SURFACE_ALT: Color32 = Color32::from_rgb(0x1b, 0x20, 0x26);
pub const TEXT: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0xc5, 0xcb, 0xd3);
pub const BORDER: Color32 = Color32::from_rgb(0x2d, 0x33, 0x3a);

pub const OK: Color32 = Color32::from_rgb(0x3f, 0xc3, 0x80);
pub const WARN: Color32 = Color32::from_rgb(0xe6, 0xa9, 0x3a);
pub const ERR: Color32 = Color32::from_rgb(0xe5, 0x5b, 0x4f);
pub const INFO: Color32 = Color32::from_rgb(0x69, 0xc1, 0xd3);

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_visuals(visuals());
    install_style(ctx);
    if (ctx.pixels_per_point() - 1.0).abs() < f32::EPSILON {
        ctx.set_pixels_per_point(1.25);
    }
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let ui_candidates: &[&str] = &[
        r"C:\Windows\Fonts\segoeui.ttf",
        "/Library/Fonts/Arial.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    let mono_candidates: &[&str] = &[
        r"C:\Windows\Fonts\consola.ttf",
        "/System/Library/Fonts/Menlo.ttc",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    ];
    let mut loaded = false;
    for p in ui_candidates {
        if let Ok(bytes) = std::fs::read(p) {
            fonts.font_data.insert("ui".into(), FontData::from_owned(bytes).into());
            fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "ui".into());
            loaded = true;
            break;
        }
    }
    for p in mono_candidates {
        if let Ok(bytes) = std::fs::read(p) {
            fonts.font_data.insert("mono".into(), FontData::from_owned(bytes).into());
            fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "mono".into());
            break;
        }
    }
    if loaded {
        ctx.set_fonts(fonts);
    }
}

fn install_style(ctx: &egui::Context) {
    use egui::{FontFamily::Proportional, FontId, TextStyle};
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Heading, FontId::new(21.0, Proportional)),
        (TextStyle::Body, FontId::new(14.5, Proportional)),
        (TextStyle::Button, FontId::new(14.5, Proportional)),
        (TextStyle::Small, FontId::new(12.0, Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, egui::FontFamily::Monospace)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 9.0);
    style.spacing.button_padding = egui::vec2(13.0, 8.0);
    ctx.set_style(style);
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    let radius = Rounding::same(8.0);
    v.override_text_color = None;
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = Color32::from_rgb(0x0a, 0x0c, 0x10);
    v.faint_bg_color = SURFACE;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT.linear_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);

    v.widgets.noninteractive.bg_fill = SURFACE;
    v.widgets.noninteractive.weak_bg_fill = SURFACE;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.rounding = radius;

    v.widgets.inactive.bg_fill = SURFACE_HI;
    v.widgets.inactive.weak_bg_fill = SURFACE_HI;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.rounding = radius;

    v.widgets.hovered.bg_fill = SURFACE_HI.linear_multiply(1.4);
    v.widgets.hovered.weak_bg_fill = SURFACE_HI.linear_multiply(1.4);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.linear_multiply(0.7));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.rounding = radius;

    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.weak_bg_fill = ACCENT;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.rounding = radius;

    v.window_rounding = radius;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .rounding(Rounding::same(8.0))
        .inner_margin(egui::Margin::same(15.0))
        .outer_margin(egui::Margin {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 11.0,
        })
}
