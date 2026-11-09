//! Colors, fonts and small reusable widgets for a calm, neutral dark look.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Rect, Response, Sense,
    Stroke, Ui, vec2,
};

pub const BG: Color32 = Color32::from_rgb(0x13, 0x14, 0x17);
pub const SURFACE: Color32 = Color32::from_rgb(0x1b, 0x1c, 0x20);
pub const SURFACE_2: Color32 = Color32::from_rgb(0x23, 0x25, 0x2a);
pub const SURFACE_3: Color32 = Color32::from_rgb(0x2c, 0x2e, 0x34);
pub const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2c, 0x31);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xe9, 0xec);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8a, 0x8e, 0x97);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x5c, 0x60, 0x68);
pub const ACCENT: Color32 = Color32::from_rgb(0x5b, 0x8c, 0xff);
pub const RECORD: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
pub const MUTE: Color32 = Color32::from_rgb(0xf5, 0x9e, 0x0b);

/// Muted track colors, assigned by track order.
pub const TRACK_COLORS: [Color32; 6] = [
    Color32::from_rgb(0x5b, 0x8c, 0xff),
    Color32::from_rgb(0x34, 0xc3, 0x8f),
    Color32::from_rgb(0xf5, 0x9e, 0x0b),
    Color32::from_rgb(0xa7, 0x8b, 0xfa),
    Color32::from_rgb(0xf4, 0x72, 0xb6),
    Color32::from_rgb(0x22, 0xd3, 0xee),
];

pub fn track_color(index: usize) -> Color32 {
    TRACK_COLORS[index % TRACK_COLORS.len()]
}

pub const RADIUS: u8 = 6;

/// Bold UI font family.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, semibold())
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(
        &mut fonts,
        "inter",
        include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    );
    add(
        &mut fonts,
        "inter-semibold",
        include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    );
    add(
        &mut fonts,
        "lucide",
        include_bytes!("../../assets/fonts/lucide.ttf"),
    );

    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "inter".into());
    let mut bold_family = vec!["inter-semibold".to_string()];
    bold_family.extend(
        fonts.families[&FontFamily::Proportional]
            .iter()
            .skip(1)
            .cloned(),
    );
    fonts.families.insert(semibold(), bold_family);
    // Icons get their own family: Lucide also maps ordinary letters, and Inter maps
    // private-use codepoints, so the two must never share a fallback chain.
    fonts.families.insert(icon_family(), vec!["lucide".into()]);
    ctx.set_fonts(fonts);
}

fn icon_family() -> FontFamily {
    FontFamily::Name("icons".into())
}

/// Font for drawing a Lucide icon.
pub fn icon(size: f32) -> FontId {
    FontId::new(size, icon_family())
}

/// "icon  label" as one widget text, mixing the icon font and the UI font.
pub fn icon_text(icon_str: &str, label: &str, size: f32, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let fmt = |font_id: FontId| egui::TextFormat {
        font_id,
        color,
        valign: egui::Align::Center,
        ..Default::default()
    };
    job.append(icon_str, 0.0, fmt(icon(size + 1.0)));
    job.append(label, 10.0, fmt(font(size)));
    job
}

/// Apply Oxbeat's look.
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    ctx.global_style_mut(|style| {
        style.text_styles = [
            (egui::TextStyle::Small, font(11.0)),
            (egui::TextStyle::Body, font(13.5)),
            (egui::TextStyle::Button, font(13.5)),
            (egui::TextStyle::Heading, bold(18.0)),
            (egui::TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(12.0, 6.0);
        style.spacing.interact_size.y = 28.0;
        style.spacing.slider_rail_height = 4.0;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = Some(TEXT);
        v.panel_fill = SURFACE;
        v.window_fill = SURFACE;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(10);
        v.window_shadow = egui::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(140),
        };
        v.popup_shadow = v.window_shadow;
        v.extreme_bg_color = BG;
        v.faint_bg_color = SURFACE_2;
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0, TEXT);
        v.hyperlink_color = ACCENT;
        v.slider_trailing_fill = true;

        let r = CornerRadius::same(RADIUS);
        for (w, fill) in [
            (&mut v.widgets.noninteractive, SURFACE),
            (&mut v.widgets.inactive, SURFACE_2),
            (&mut v.widgets.hovered, SURFACE_3),
            (&mut v.widgets.active, SURFACE_3),
            (&mut v.widgets.open, SURFACE_3),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.bg_stroke = Stroke::NONE;
            w.corner_radius = r;
            w.expansion = 0.0;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
        v.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    });
}

/// A square, borderless icon button. `active` keeps it highlighted.
pub fn icon_button(ui: &mut Ui, glyph: &str, size: f32, active: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let fill = if active {
        SURFACE_3
    } else if resp.hovered() {
        SURFACE_2
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, RADIUS, fill);
    let color = if resp.hovered() || active {
        TEXT
    } else {
        TEXT_DIM
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        icon(size * 0.52),
        color,
    );
    resp
}

/// A small lettered toggle ("M", "S") used in track headers.
pub fn toggle_chip(ui: &mut Ui, label: &str, on: &mut bool, on_color: Color32) -> Response {
    toggle_chip_with(ui, label, bold(11.0), on, on_color)
}

/// Like `toggle_chip`, with an icon glyph instead of a letter.
pub fn icon_chip(ui: &mut Ui, glyph: &str, on: &mut bool, on_color: Color32) -> Response {
    toggle_chip_with(ui, glyph, icon(12.0), on, on_color)
}

fn toggle_chip_with(
    ui: &mut Ui,
    label: &str,
    font: FontId,
    on: &mut bool,
    on_color: Color32,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let (fill, text) = if *on {
        (on_color, BG)
    } else if resp.hovered() {
        (Color32::from_rgb(0x3a, 0x3d, 0x45), TEXT)
    } else {
        (SURFACE_3, TEXT_DIM)
    };
    ui.painter().rect_filled(rect, 4, fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        text,
    );
    resp
}

/// A filled primary button with an optional leading icon.
pub fn primary_button(ui: &mut Ui, icon: Option<&str>, label: &str) -> Response {
    let job = match icon {
        Some(i) => {
            let mut job = icon_text(i, label, 13.5, Color32::WHITE);
            job.sections[1].format.font_id = bold(13.5);
            job
        }
        None => egui::text::LayoutJob::simple_singleline(label.into(), bold(13.5), Color32::WHITE),
    };
    let galley = ui.painter().layout_job(job);
    let size = vec2(galley.size().x + 28.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() {
        ACCENT.linear_multiply(1.12)
    } else {
        ACCENT
    };
    ui.painter().rect_filled(rect, RADIUS, fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
    resp
}

/// Thin separator line inside a horizontal layout.
pub fn vsep(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
    ui.painter()
        .rect_filled(Rect::from_min_size(rect.min, rect.size()), 0, BORDER);
}

/// Angle of a knob's pointer at position `t` (0.0..=1.0): a 270° sweep, open at the bottom.
fn knob_angle(t: f32) -> f32 {
    std::f32::consts::PI * (0.75 + 1.5 * t.clamp(0.0, 1.0))
}

/// A rotary knob for `value` (0.0..=1.0). Drag up or down to turn it (hold Shift for
/// fine steps); double-click to go back to `default`.
pub fn knob(ui: &mut Ui, value: &mut f32, default: f32, size: f32, color: Color32) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let before = *value;
    if resp.double_clicked() {
        *value = default;
    } else if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let per_pixel = if fine { 1.0 / 900.0 } else { 1.0 / 160.0 };
        *value = (*value - resp.drag_delta().y * per_pixel).clamp(0.0, 1.0);
    }
    if *value != before {
        resp.mark_changed();
    }

    let center = rect.center();
    let radius = size / 2.0 - 2.0;
    let arc = |from: f32, to: f32| -> Vec<egui::Pos2> {
        let steps = 24;
        (0..=steps)
            .map(|i| {
                let a = knob_angle(from + (to - from) * i as f32 / steps as f32);
                center + radius * vec2(a.cos(), a.sin())
            })
            .collect()
    };
    let p = ui.painter();
    let width = (size / 12.0).clamp(2.0, 3.5);
    p.add(egui::Shape::line(
        arc(0.0, 1.0),
        Stroke::new(width, SURFACE_3),
    ));
    // The lit part runs from where the knob rests (centre for pan, zero for most).
    let rest = if (default - 0.5).abs() < 1e-3 {
        0.5
    } else {
        0.0
    };
    if (*value - rest).abs() > 1e-3 {
        p.add(egui::Shape::line(
            arc(rest, *value),
            Stroke::new(width, color),
        ));
    }
    let a = knob_angle(*value);
    let dir = vec2(a.cos(), a.sin());
    let hot = resp.hovered() || resp.dragged();
    p.circle_filled(
        center,
        radius - width - 1.5,
        if hot {
            SURFACE_3
        } else {
            Color32::from_rgb(0x30, 0x33, 0x3a)
        },
    );
    p.line_segment(
        [
            center + dir * (radius * 0.25),
            center + dir * (radius - width - 3.0),
        ],
        Stroke::new(1.5, if hot { TEXT } else { TEXT_DIM }),
    );
    resp.on_hover_cursor(egui::CursorIcon::ResizeVertical)
}
