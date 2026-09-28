//! Colours, type and spacing, in light and dark.

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Visuals,
};

pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    /// Fields, cards and popups.
    pub card: Color32,
    pub hover: Color32,
    pub line: Color32,
    pub line_strong: Color32,
    pub text: Color32,
    /// Headings and names: `RichText::strong`.
    pub strong: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub good: Color32,
    pub partial: Color32,
    pub bad: Color32,
    pub flux: Color32,
    pub written: Color32,
    pub erased: Color32,
    pub pending: Color32,
}

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(22, 24, 29),
    sidebar: Color32::from_rgb(17, 19, 23),
    card: Color32::from_rgb(30, 33, 40),
    hover: Color32::from_rgb(38, 42, 51),
    line: Color32::from_rgb(44, 49, 59),
    line_strong: Color32::from_rgb(62, 68, 80),
    text: Color32::from_rgb(218, 222, 229),
    strong: Color32::WHITE,
    dim: Color32::from_rgb(144, 152, 166),
    accent: Color32::from_rgb(109, 140, 255),
    on_accent: Color32::WHITE,
    good: Color32::from_rgb(61, 214, 140),
    partial: Color32::from_rgb(245, 184, 61),
    bad: Color32::from_rgb(242, 85, 90),
    flux: Color32::from_rgb(79, 182, 240),
    written: Color32::from_rgb(163, 139, 250),
    erased: Color32::from_rgb(107, 114, 128),
    pending: Color32::from_rgb(42, 47, 56),
};

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(246, 247, 249),
    sidebar: Color32::from_rgb(236, 238, 242),
    card: Color32::WHITE,
    hover: Color32::from_rgb(240, 242, 245),
    line: Color32::from_rgb(220, 224, 230),
    line_strong: Color32::from_rgb(197, 203, 212),
    text: Color32::from_rgb(38, 42, 50),
    strong: Color32::from_rgb(8, 10, 14),
    dim: Color32::from_rgb(94, 102, 116),
    accent: Color32::from_rgb(61, 99, 221),
    on_accent: Color32::WHITE,
    good: Color32::from_rgb(31, 164, 99),
    partial: Color32::from_rgb(212, 138, 0),
    bad: Color32::from_rgb(220, 60, 67),
    flux: Color32::from_rgb(30, 143, 208),
    written: Color32::from_rgb(123, 97, 232),
    erased: Color32::from_rgb(154, 161, 173),
    pending: Color32::from_rgb(228, 231, 236),
};

pub const RADIUS: u8 = 6;
/// The height of every field, list and button in a form row.
pub const FIELD_HEIGHT: f32 = 28.0;

/// The sidebar logo and window icon, 256 pixels square with a clear
/// background. Made by packaging/macos/icon.sh.
pub const LOGO: &[u8] = include_bytes!("../assets/logo.png");

pub fn palette(ui: &egui::Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

/// The colour `t` of the way from `a` to `b`, `t` from 0 to 1.
pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(&DARK, Visuals::dark()));
    ctx.set_visuals_of(Theme::Light, visuals(&LIGHT, Visuals::light()));
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::proportional(22.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(8.0, 8.0);
        s.button_padding = egui::vec2(10.0, 5.0);
        s.interact_size.y = FIELD_HEIGHT;
        s.menu_margin = Margin::same(8);
        s.combo_height = 360.0;
        style.interaction.selectable_labels = false;
    });
}

fn visuals(p: &Palette, mut v: Visuals) -> Visuals {
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.window_stroke = Stroke::new(1.0, p.line);
    v.extreme_bg_color = p.card;
    v.text_edit_bg_color = Some(p.card);
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.card;
    v.hyperlink_color = p.accent;
    v.weak_text_color = Some(p.dim);
    v.warn_fg_color = p.partial;
    v.error_fg_color = p.bad;
    v.selection.bg_fill = p.accent.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(60),
    };
    v.window_shadow = v.popup_shadow;

    let w = &mut v.widgets;
    for (state, fill, stroke, text) in [
        (&mut w.noninteractive, p.bg, p.line, p.text),
        (&mut w.inactive, p.card, p.line, p.text),
        (&mut w.hovered, p.hover, p.line_strong, p.text),
        // egui draws strong text in this colour too.
        (&mut w.active, p.hover, p.accent, p.strong),
        (&mut w.open, p.hover, p.line_strong, p.text),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.fg_stroke = Stroke::new(1.0, text);
        state.corner_radius = CornerRadius::same(RADIUS);
        state.expansion = 0.0;
    }
    w.active.bg_stroke.width = 1.5;
    v
}
