//! Colours, type and spacing: light, dark, the Greaseweazle's purple and 90s GUI grey.

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Margin, Shadow, Shape, Stroke, TextStyle, Theme,
    ThemePreference, Visuals, layers::ShapeIdx, style::ScrollStyle,
};

/// The theme chosen in Settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Choice {
    #[default]
    System,
    Light,
    Dark,
    /// Classic in teal, the other accent in its right-click menu.
    Classic,
    /// Classic in blue, the accent it starts in.
    Blue,
    Vintage,
    Greaseweazle,
}

/// Each choice in Settings' order: its name, its hover, and the word theme.txt
/// keeps. Blue is Classic in the accent it starts in, under Classic's button.
pub const CHOICES: [(Choice, &str, &str, &str); 7] = [
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
        Choice::Vintage,
        "Vintage",
        "Always the '80s (light).",
        "vintage",
    ),
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

/// Shows `choice`: Greaseweazle is egui's dark theme, and Classic and Vintage
/// its light one, with their own colours in them.
pub fn apply(ctx: &egui::Context, choice: Choice) {
    let (dark, light, shown) = match choice {
        Choice::System => (&DARK, &LIGHT, ThemePreference::System),
        Choice::Light => (&DARK, &LIGHT, ThemePreference::Light),
        Choice::Dark => (&DARK, &LIGHT, ThemePreference::Dark),
        Choice::Classic => (&DARK, &CLASSIC, ThemePreference::Light),
        Choice::Blue => (&DARK, &BLUE, ThemePreference::Light),
        Choice::Vintage => (&DARK, &VINTAGE, ThemePreference::Light),
        Choice::Greaseweazle => (&GREASEWEAZLE, &LIGHT, ThemePreference::Dark),
    };
    ctx.set_visuals_of(Theme::Dark, visuals(dark, Visuals::dark()));
    ctx.set_visuals_of(Theme::Light, visuals(light, Visuals::light()));
    ctx.style_mut_of(Theme::Dark, |s| s.spacing.scroll = bars(dark));
    ctx.style_mut_of(Theme::Light, |s| s.spacing.scroll = bars(light));
    ctx.set_theme(shown);
}

/// egui's bars, which float over what they scroll and show under the pointer:
/// whole, not faint, where the palette has them bold.
fn bars(p: &Palette) -> ScrollStyle {
    let mut bars = ScrollStyle::floating();
    if p.bold_bars {
        bars.active_handle_opacity = 1.0;
    }
    bars
}

/// A text view's bars, drawn whenever there is more to see: a floating one
/// hides until hovered, and a wheel does not scroll sideways.
pub fn solid_bars(ui: &mut egui::Ui, p: &Palette) {
    ui.spacing_mut().scroll = ScrollStyle {
        foreground_color: p.bold_bars,
        ..ScrollStyle::solid()
    };
    // The theme paints an idle handle in the card's colour.
    ui.visuals_mut().widgets.inactive.bg_fill = p.line;
}

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
    /// A link's colour where the accent would read as a caption (link()).
    pub link: Option<Color32>,
    pub good: Color32,
    pub partial: Color32,
    pub bad: Color32,
    pub flux: Color32,
    pub written: Color32,
    pub erased: Color32,
    pub pending: Color32,
    /// Classic GUI look: solid selections and square corners (square()).
    pub classic: bool,
    /// gw's command lines and output on black in this colour, as a console
    /// (terminal()).
    pub console: Option<Color32>,
    /// Scroll bars in the text's colour at full strength: white on the purple.
    pub bold_bars: bool,
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
    link: None,
    good: Color32::from_rgb(61, 214, 140),
    partial: Color32::from_rgb(245, 184, 61),
    bad: Color32::from_rgb(242, 85, 90),
    flux: Color32::from_rgb(79, 182, 240),
    written: Color32::from_rgb(163, 139, 250),
    erased: Color32::from_rgb(107, 114, 128),
    pending: Color32::from_rgb(42, 47, 56),
    classic: false,
    console: None,
    bold_bars: false,
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
    link: None,
    good: Color32::from_rgb(31, 164, 99),
    partial: Color32::from_rgb(212, 138, 0),
    bad: Color32::from_rgb(220, 60, 67),
    flux: Color32::from_rgb(30, 143, 208),
    written: Color32::from_rgb(123, 97, 232),
    erased: Color32::from_rgb(154, 161, 173),
    pending: Color32::from_rgb(228, 231, 236),
    classic: false,
    console: None,
    bold_bars: false,
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
    link: None,
    good: Color32::from_rgb(61, 214, 140),
    partial: Color32::from_rgb(245, 184, 61),
    bad: Color32::from_rgb(255, 99, 112),
    flux: Color32::from_rgb(79, 182, 240),
    written: Color32::from_rgb(240, 140, 220),
    erased: Color32::from_rgb(130, 112, 150),
    pending: Color32::from_rgb(60, 25, 90),
    classic: false,
    console: Some(Color32::from_rgb(238, 230, 247)),
    bold_bars: true,
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
    link: None,
    good: Color32::from_rgb(0, 128, 0),
    partial: Color32::from_rgb(224, 160, 0),
    bad: Color32::from_rgb(200, 0, 0),
    flux: Color32::from_rgb(0, 51, 153),
    written: Color32::from_rgb(128, 0, 128),
    erased: Color32::from_rgb(128, 128, 128),
    pending: Color32::from_rgb(225, 227, 229),
    classic: true,
    // The classic console's light grey.
    console: Some(Color32::from_rgb(192, 192, 192)),
    bold_bars: false,
};

/// Classic in blue, its accent and flux swapped: a navy with enough green in
/// it to grey to steel, not lilac.
pub const BLUE: Palette = Palette {
    accent: CLASSIC.flux,
    flux: CLASSIC.accent,
    ..CLASSIC
};

/// 1980s computers: the beige of their cases, slate for what is chosen, and
/// a green phosphor screen for gw's text.
pub const VINTAGE: Palette = Palette {
    bg: Color32::from_rgb(201, 199, 175),
    sidebar: Color32::from_rgb(192, 184, 155),
    card: Color32::from_rgb(238, 241, 219),
    hover: Color32::from_rgb(227, 225, 201),
    line: Color32::from_rgb(124, 127, 130),
    line_strong: Color32::from_rgb(49, 53, 63),
    text: Color32::from_rgb(49, 53, 63),
    strong: Color32::from_rgb(49, 53, 63),
    dim: Color32::from_rgb(78, 84, 88),
    accent: Color32::from_rgb(72, 91, 99),
    on_accent: Color32::from_rgb(238, 241, 219),
    // Slate is a caption's grey here: a deeper shade of the map's flux blue.
    link: Some(Color32::from_rgb(40, 76, 132)),
    good: Color32::from_rgb(74, 124, 58),
    partial: Color32::from_rgb(198, 140, 36),
    bad: Color32::from_rgb(176, 58, 46),
    flux: Color32::from_rgb(64, 98, 150),
    written: Color32::from_rgb(128, 76, 140),
    erased: Color32::from_rgb(124, 127, 130),
    pending: Color32::from_rgb(227, 225, 201),
    classic: true,
    console: Some(Color32::from_rgb(51, 255, 102)),
    bold_bars: false,
};

impl Palette {
    /// A link's colour: the accent unless the palette has another.
    pub fn link(&self) -> Color32 {
        self.link.unwrap_or(self.accent)
    }
}

pub const RADIUS: u8 = 6;
/// The height of every field, list and button in a form row.
pub const FIELD_HEIGHT: f32 = 28.0;

/// The sidebar logo and window icon, 256 pixels square with a clear
/// background. Made by packaging/macos/icon.sh.
pub const LOGO: &[u8] = include_bytes!("../assets/logo.png");

/// The floppy from the icon, large, for the About window.
pub const ABOUT: &[u8] = include_bytes!("../assets/about.png");

/// The colours the window has, known by its links': each palette apply()
/// shows has a link colour of its own.
pub fn palette(ui: &egui::Ui) -> &'static Palette {
    let v = ui.visuals();
    let shown = [&LIGHT, &DARK, &GREASEWEAZLE, &CLASSIC, &BLUE, &VINTAGE];
    let unthemed = if v.dark_mode { &DARK } else { &LIGHT };
    shown
        .into_iter()
        .find(|p| p.link() == v.hyperlink_color)
        .unwrap_or(unthemed)
}

/// The colour `t` of the way from `a` to `b`, `t` from 0 to 1.
pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// Draws gw's command lines and output with `add`, in the window's palette
/// or, where that has a console, on black in the console's colour: a dark
/// palette's other colours are its own, a light one's Dark's.
pub fn terminal<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui, &Palette) -> R) -> R {
    ui.scope(|ui| {
        let p = palette(ui);
        let Some(text) = p.console else {
            return add(ui, p);
        };
        let rest = if ui.visuals().dark_mode { p } else { &DARK };
        let console = Palette {
            card: Color32::BLACK,
            text,
            ..*rest
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

/// Edges the focused text field in the accent where egui's edge, the
/// selection's text colour, is the field's own: Vintage's cream. A field
/// with no frame (the command line's) has no such edge and keeps none.
pub fn edge_focus(ctx: &egui::Context, p: &Palette) {
    if p.on_accent != p.card {
        return;
    }
    let focused = ctx.memory(|m| m.focused());
    let Some(field) = focused.and_then(|id| ctx.read_response(id)) else {
        return;
    };
    ctx.graphics_mut(|g| {
        if let Some(list) = g.get_mut(field.layer_id) {
            for i in 0..list.next_idx().0 {
                list.mutate_shape(ShapeIdx(i), |s| edge(&mut s.shape, field.rect, p));
            }
        }
    });
}

fn edge(shape: &mut Shape, field: egui::Rect, p: &Palette) {
    match shape {
        Shape::Rect(rect)
            if rect.stroke.color == p.on_accent
                && rect.fill == p.card
                && rect.rect.expand(1.0).contains_rect(field)
                && field.expand(1.0).contains_rect(rect.rect) =>
        {
            rect.stroke.color = p.accent;
        }
        Shape::Vec(shapes) => shapes.iter_mut().for_each(|s| edge(s, field, p)),
        _ => {}
    }
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
    v.hyperlink_color = p.link();
    v.weak_text_color = Some(p.dim);
    v.warn_fg_color = p.partial;
    v.error_fg_color = p.bad;
    // egui also edges a focused text box in this text colour: white in Classic.
    let (fill, text) = match p.classic {
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
