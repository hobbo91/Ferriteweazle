//! Colours, type and spacing: light, dark, the Greaseweazle's purple and 90s GUI grey.

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Margin, Shadow, Shape, Stroke, TextStyle, Theme,
    ThemePreference, Visuals, layers::ShapeIdx,
};

/// The theme chosen in Settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Choice {
    #[default]
    System,
    Light,
    Dark,
    Classic,
    /// Classic with a blue accent, from Classic's right-click menu.
    Blue,
    Greaseweazle,
}

/// Each choice in Settings' order: its name, its hover, and the word theme.txt
/// keeps. Blue is Classic's other accent, with no button of its own.
pub const CHOICES: [(Choice, &str, &str, &str); 6] = [
    (Choice::System, "System", "Follow the system.", ""),
    (Choice::Light, "Light", "Always light.", "light"),
    (Choice::Dark, "Dark", "Always dark.", "dark"),
    (
        Choice::Classic,
        "Classic",
        "Always the '90s (light, right-click for accent).",
        "classic",
    ),
    (Choice::Blue, "Blue", "", "blue"),
    (
        Choice::Greaseweazle,
        "Greaseweazle v4.1",
        "Always purple (dark).",
        "greaseweazle",
    ),
];

impl From<Theme> for Choice {
    fn from(theme: Theme) -> Choice {
        match theme {
            Theme::Dark => Choice::Dark,
            Theme::Light => Choice::Light,
        }
    }
}

/// Shows `choice`: Greaseweazle and Classic are egui's dark and light themes
/// with their own colours in them.
pub fn apply(ctx: &egui::Context, choice: Choice) {
    let (dark, light, shown) = match choice {
        Choice::System => (&DARK, &LIGHT, ThemePreference::System),
        Choice::Light => (&DARK, &LIGHT, ThemePreference::Light),
        Choice::Dark => (&DARK, &LIGHT, ThemePreference::Dark),
        Choice::Classic => (&DARK, &CLASSIC, ThemePreference::Light),
        Choice::Blue => (&DARK, &BLUE, ThemePreference::Light),
        Choice::Greaseweazle => (&GREASEWEAZLE, &LIGHT, ThemePreference::Dark),
    };
    ctx.set_visuals_of(Theme::Dark, visuals(dark, Visuals::dark()));
    ctx.set_visuals_of(Theme::Light, visuals(light, Visuals::light()));
    ctx.set_theme(shown);
}

#[derive(Clone, Copy)]
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
    /// Classic GUI look: solid selections and square corners (square()).
    pub win9x: bool,
    /// gw's command lines and output on black, as a console (terminal()).
    pub console: bool,
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
    win9x: false,
    console: false,
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
    win9x: false,
    console: false,
};

/// From the purple of the Greaseweazle's board, rgb(94, 25, 139): darker for
/// the window, so that its text reads clearly.
pub const GREASEWEAZLE: Palette = Palette {
    bg: Color32::from_rgb(36, 12, 56),
    sidebar: Color32::from_rgb(26, 8, 41),
    card: Color32::from_rgb(52, 20, 78),
    hover: Color32::from_rgb(66, 27, 98),
    line: Color32::from_rgb(76, 36, 108),
    line_strong: Color32::from_rgb(104, 58, 142),
    text: Color32::from_rgb(238, 230, 247),
    strong: Color32::WHITE,
    dim: Color32::from_rgb(188, 170, 210),
    accent: Color32::from_rgb(178, 118, 255),
    on_accent: Color32::WHITE,
    good: Color32::from_rgb(61, 214, 140),
    partial: Color32::from_rgb(245, 184, 61),
    bad: Color32::from_rgb(255, 99, 112),
    flux: Color32::from_rgb(79, 182, 240),
    written: Color32::from_rgb(240, 140, 220),
    erased: Color32::from_rgb(130, 112, 150),
    pending: Color32::from_rgb(60, 25, 90),
    win9x: false,
    console: true,
};

/// 90s GUIs standard scheme, on rgb(195, 199, 203): silver-grey
/// with lighter grey fields, and the teal of the 1990s.
pub const CLASSIC: Palette = Palette {
    bg: Color32::from_rgb(195, 199, 203),
    sidebar: Color32::from_rgb(195, 199, 203),
    card: Color32::from_rgb(225, 227, 229),
    hover: Color32::from_rgb(210, 213, 216),
    line: Color32::from_rgb(128, 128, 128),
    line_strong: Color32::from_rgb(64, 64, 64),
    text: Color32::BLACK,
    strong: Color32::BLACK,
    dim: Color32::from_rgb(64, 64, 64),
    accent: Color32::from_rgb(0, 128, 128),
    on_accent: Color32::WHITE,
    good: Color32::from_rgb(0, 128, 0),
    partial: Color32::from_rgb(224, 160, 0),
    bad: Color32::from_rgb(200, 0, 0),
    flux: Color32::from_rgb(0, 51, 153),
    written: Color32::from_rgb(128, 0, 128),
    erased: Color32::from_rgb(128, 128, 128),
    pending: Color32::from_rgb(225, 227, 229),
    win9x: true,
    console: true,
};

/// Classic in blue, its accent and flux swapped: a navy with enough green in
/// it to grey to steel, not lilac.
pub const BLUE: Palette = Palette {
    accent: CLASSIC.flux,
    flux: CLASSIC.accent,
    ..CLASSIC
};

/// The classic console's light grey on black, for gw's text in Classic.
pub const CONSOLE: Palette = Palette {
    card: Color32::BLACK,
    text: Color32::from_rgb(192, 192, 192),
    ..DARK
};

pub const RADIUS: u8 = 6;
/// The height of every field, list and button in a form row.
pub const FIELD_HEIGHT: f32 = 28.0;

/// The sidebar logo and window icon, 256 pixels square with a clear
/// background. Made by packaging/macos/icon.sh.
pub const LOGO: &[u8] = include_bytes!("../assets/logo.png");

/// The colours the window has, known by its links': each palette apply()
/// shows has an accent of its own.
pub fn palette(ui: &egui::Ui) -> &'static Palette {
    let v = ui.visuals();
    let shown = [&LIGHT, &DARK, &GREASEWEAZLE, &CLASSIC, &BLUE];
    let unthemed = if v.dark_mode { &DARK } else { &LIGHT };
    shown
        .into_iter()
        .find(|p| p.accent == v.hyperlink_color)
        .unwrap_or(unthemed)
}

/// The colour `t` of the way from `a` to `b`, `t` from 0 to 1.
pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// Draws gw's command lines and output with `add`, in the window's palette
/// or, where that has a console, on black: a dark palette in its own colours,
/// a light one in the Windows console's.
pub fn terminal<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui, &Palette) -> R) -> R {
    ui.scope(|ui| {
        let p = palette(ui);
        let console = match (p.console, ui.visuals().dark_mode) {
            (false, _) => return add(ui, p),
            (true, true) => Palette {
                card: Color32::BLACK,
                ..*p
            },
            (true, false) => CONSOLE,
        };
        *ui.visuals_mut() = visuals(&console, Visuals::dark());
        add(ui, &console)
    })
    .inner
}

/// Squares every corner drawn so far this frame.
/// Each shape sets its own rounding, in too many places to pass a palette.
pub fn square(ctx: &egui::Context) {
    let layers: Vec<_> = ctx.memory(|m| m.layer_ids().collect());
    ctx.graphics_mut(|g| {
        for layer in layers {
            if let Some(list) = g.get_mut(layer) {
                for i in 0..list.next_idx().0 {
                    list.mutate_shape(ShapeIdx(i), |s| unround(&mut s.shape));
                }
            }
        }
    });
}

fn unround(shape: &mut Shape) {
    match shape {
        Shape::Rect(rect) => rect.corner_radius = CornerRadius::ZERO,
        Shape::Vec(shapes) => shapes.iter_mut().for_each(unround),
        _ => {}
    }
}

/// Type and spacing; apply() gives the colours.
pub fn install(ctx: &egui::Context) {
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
    // egui also edges a focused text box in this text colour: white in Classic.
    let (fill, text) = match p.win9x {
        true => (p.accent, p.on_accent),
        false => (p.accent.gamma_multiply(0.35), p.accent),
    };
    v.selection.bg_fill = fill;
    v.selection.stroke = Stroke::new(1.0, text);
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
