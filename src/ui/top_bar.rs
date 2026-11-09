//! Transport, position display, project menu and the "Add Track" dialog.

use egui::{Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

use super::icons;
use super::studio::{Studio, bar_and_beat};
use super::theme::{
    self, ACCENT, BORDER, RECORD, SURFACE_2, SURFACE_3, TEXT, TEXT_DIM, TEXT_FAINT, bold, font,
};
use crate::engine::synth::PRESETS;

pub const HEIGHT: f32 = 56.0;

pub enum Action {
    None,
    New,
    Open,
    Save,
}

#[derive(Default)]
pub struct AddTrackDialog {
    pub open: bool,
    choosing_sound: bool,
}

#[derive(Clone, Copy)]
enum Glyph {
    Rewind,
    Play,
    Stop,
    Record,
}

/// Round transport button with a painted (filled) glyph.
fn transport_button(ui: &mut Ui, glyph: Glyph, active: bool) -> egui::Response {
    let size = 36.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let p = ui.painter();
    let bg = match (glyph, active) {
        (Glyph::Record, true) => RECORD,
        (Glyph::Play | Glyph::Stop, true) => ACCENT,
        _ if resp.hovered() => SURFACE_3,
        _ => SURFACE_2,
    };
    p.circle_filled(rect.center(), size / 2.0, bg);
    let c = rect.center();
    let fg = if active { Color32::WHITE } else { TEXT };
    match glyph {
        Glyph::Play => {
            let s = 7.0;
            p.add(Shape::convex_polygon(
                vec![
                    pos2(c.x - s * 0.6, c.y - s),
                    pos2(c.x + s, c.y),
                    pos2(c.x - s * 0.6, c.y + s),
                ],
                fg,
                Stroke::NONE,
            ));
        }
        Glyph::Stop => {
            p.rect_filled(Rect::from_center_size(c, vec2(11.0, 11.0)), 2, fg);
        }
        Glyph::Record => {
            let color = if active { Color32::WHITE } else { RECORD };
            p.circle_filled(c, 6.5, color);
        }
        Glyph::Rewind => {
            let s = 6.0;
            p.rect_filled(
                Rect::from_min_size(pos2(c.x - s - 2.0, c.y - s), vec2(2.5, s * 2.0)),
                1,
                fg,
            );
            p.add(Shape::convex_polygon(
                vec![
                    pos2(c.x + s, c.y - s),
                    pos2(c.x + s, c.y + s),
                    pos2(c.x - s + 1.5, c.y),
                ],
                fg,
                Stroke::NONE,
            ));
        }
    }
    resp
}

/// Draws text with every character in a fixed-width cell so numbers don't jitter.
fn fixed_digits(ui: &Ui, left: Pos2, text: &str, size: f32, color: Color32) -> f32 {
    let cell = size * 0.64;
    for (i, ch) in text.chars().enumerate() {
        let cx = left.x + cell * (i as f32 + 0.5);
        ui.painter().text(
            pos2(cx, left.y),
            Align2::CENTER_CENTER,
            ch,
            bold(size),
            color,
        );
    }
    cell * text.chars().count() as f32
}

fn caption(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(font(9.5)).color(TEXT_FAINT));
}

fn display(ui: &mut Ui, studio: &mut Studio) {
    let (bar, in_bar) = bar_and_beat(studio.position);

    egui::Frame::NONE
        .fill(theme::BG)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(14, 3))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let (rect, _) = ui.allocate_exact_size(vec2(70.0, 38.0), Sense::hover());
                ui.painter().text(
                    rect.left_top() + vec2(0.0, 3.0),
                    Align2::LEFT_TOP,
                    "BAR",
                    font(9.5),
                    TEXT_FAINT,
                );
                ui.painter().text(
                    rect.left_top() + vec2(48.0, 3.0),
                    Align2::LEFT_TOP,
                    "BEAT",
                    font(9.5),
                    TEXT_FAINT,
                );
                let y = rect.top() + 26.0;
                fixed_digits(ui, pos2(rect.left(), y), &format!("{bar:03}"), 17.0, TEXT);
                fixed_digits(
                    ui,
                    pos2(rect.left() + 48.0, y),
                    &in_bar.to_string(),
                    17.0,
                    TEXT,
                );

                theme::vsep(ui, 26.0);

                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    caption(ui, "TEMPO");
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.spacing_mut().interact_size.y = 20.0;
                        let mut tempo = studio.project.tempo_bpm;
                        let r = ui
                            .add(
                                egui::DragValue::new(&mut tempo)
                                    .range(40.0..=240.0)
                                    .speed(0.5)
                                    .fixed_decimals(0),
                            )
                            .on_hover_text("Drag or double-click to change the tempo");
                        if r.changed() {
                            studio.project.tempo_bpm = tempo.round();
                            studio.sync();
                        }
                        ui.label(egui::RichText::new("BPM").font(font(11.0)).color(TEXT_DIM));
                    });
                });

                theme::vsep(ui, 26.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    caption(ui, "KEY");
                    ui.label(egui::RichText::new(&studio.project.key).font(bold(13.0)));
                });

                theme::vsep(ui, 26.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    caption(ui, "TIME");
                    ui.label(egui::RichText::new("4/4").font(bold(13.0)));
                });
            });
        });
}

pub fn show(ui: &mut Ui, studio: &mut Studio, dialog: &mut AddTrackDialog) -> Action {
    let mut action = Action::None;
    let full = ui.available_rect_before_wrap();
    let full = Rect::from_min_size(full.min, vec2(full.width(), HEIGHT));

    // Left: menu + project name
    let left = Rect::from_min_size(full.min, vec2(260.0, HEIGHT));
    ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
        ui.horizontal_centered(|ui| {
            let menu = theme::icon_button(ui, icons::MENU, 32.0, false).on_hover_text("Menu");
            egui::Popup::menu(&menu).show(|ui| {
                let item = |ui: &mut Ui, icon: &str, label: &str, shortcut: &str| {
                    ui.add(
                        egui::Button::new(theme::icon_text(icon, label, 13.5, TEXT))
                            .shortcut_text(shortcut)
                            .min_size(vec2(210.0, 30.0)),
                    )
                    .clicked()
                };
                if item(ui, icons::FILE_PLUS, "New Project", "Ctrl+N") {
                    action = Action::New;
                }
                if item(ui, icons::FOLDER_OPEN, "Open…", "Ctrl+O") {
                    action = Action::Open;
                }
                if item(ui, icons::SAVE, "Save", "Ctrl+S") {
                    action = Action::Save;
                }
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new(&studio.project.name).font(bold(14.0)));
        });
    });

    // Center: transport + display
    let center = Rect::from_center_size(full.center(), vec2(560.0, HEIGHT));
    ui.scope_builder(egui::UiBuilder::new().max_rect(center), |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if transport_button(ui, Glyph::Rewind, false)
                .on_hover_text("Go to start  (Enter)")
                .clicked()
            {
                studio.rewind();
            }
            let glyph = if studio.playing {
                Glyph::Stop
            } else {
                Glyph::Play
            };
            if transport_button(ui, glyph, studio.playing && !studio.recording)
                .on_hover_text("Play / Stop  (Space)")
                .clicked()
            {
                studio.play_stop();
            }
            if transport_button(ui, Glyph::Record, studio.recording)
                .on_hover_text("Record on the selected track  (R)")
                .clicked()
            {
                studio.record();
            }
            ui.add_space(10.0);
            display(ui, studio);
        });
    });

    // Right: add track
    let right = Rect::from_min_max(pos2(full.right() - 200.0, full.top()), full.max);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(right)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            if theme::primary_button(ui, Some(icons::PLUS), "Add Track").clicked() {
                dialog.open = true;
                dialog.choosing_sound = false;
            }
        },
    );
    ui.allocate_rect(full, Sense::hover());
    action
}

/// A selectable card with a large icon, title and caption.
fn type_card(ui: &mut Ui, icon: &str, title: &str, caption: &str, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(146.0, 132.0), Sense::click());
    let hovered = enabled && resp.hovered();
    let p = ui.painter();
    p.rect_filled(rect, 10, if hovered { SURFACE_3 } else { SURFACE_2 });
    if hovered {
        p.rect_stroke(rect, 10, Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
    }
    let dim = if enabled { 1.0 } else { 0.4 };
    p.text(
        rect.center_top() + vec2(0.0, 44.0),
        Align2::CENTER_CENTER,
        icon,
        theme::icon(30.0),
        TEXT.gamma_multiply(dim),
    );
    p.text(
        rect.center_top() + vec2(0.0, 84.0),
        Align2::CENTER_CENTER,
        title,
        bold(14.0),
        TEXT.gamma_multiply(dim),
    );
    p.text(
        rect.center_top() + vec2(0.0, 105.0),
        Align2::CENTER_CENTER,
        caption,
        font(11.5),
        TEXT_DIM.gamma_multiply(dim),
    );
    enabled && resp.clicked()
}

fn preset_caption(preset: &str) -> &'static str {
    match preset {
        "Keys" => "Soft electric piano",
        "Bass" => "Warm, round synth bass",
        "Lead" => "Bright square lead",
        "Pad" => "Slow, wide synth pad",
        _ => "",
    }
}

pub fn add_track_dialog(ctx: &egui::Context, studio: &mut Studio, dialog: &mut AddTrackDialog) {
    if !dialog.open {
        return;
    }
    let r = egui::Modal::new(egui::Id::unique("add-track"))
        .frame(
            egui::Frame::window(&ctx.global_style())
                .inner_margin(24)
                .corner_radius(12),
        )
        .show(ctx, |ui| {
            ui.set_width(462.0);
            ui.horizontal(|ui| {
                if dialog.choosing_sound
                    && theme::icon_button(ui, icons::CHEVRON_LEFT, 28.0, false)
                        .on_hover_text("Back")
                        .clicked()
                {
                    dialog.choosing_sound = false;
                }
                let title = if dialog.choosing_sound {
                    "Choose a Sound"
                } else {
                    "New Track"
                };
                ui.label(egui::RichText::new(title).font(bold(18.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::icon_button(ui, icons::X, 28.0, false).clicked() {
                        dialog.open = false;
                    }
                });
            });
            ui.add_space(16.0);

            if !dialog.choosing_sound {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    if type_card(ui, icons::PIANO, "Instrument", "Play with a keyboard", true) {
                        dialog.choosing_sound = true;
                    }
                    type_card(ui, icons::MIC, "Voice", "Coming soon", false);
                    type_card(ui, icons::DRUM, "Drums", "Coming soon", false);
                });
            } else {
                for preset in PRESETS {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
                    let p = ui.painter();
                    if resp.hovered() {
                        p.rect_filled(rect, 8, SURFACE_2);
                    }
                    let icon_rect =
                        Rect::from_min_size(rect.min + vec2(10.0, 9.0), vec2(36.0, 36.0));
                    p.rect_filled(icon_rect, 8, SURFACE_3);
                    p.text(
                        icon_rect.center(),
                        Align2::CENTER_CENTER,
                        icons::AUDIO_LINES,
                        theme::icon(17.0),
                        ACCENT,
                    );
                    let x = icon_rect.right() + 14.0;
                    p.text(
                        pos2(x, rect.top() + 18.0),
                        Align2::LEFT_CENTER,
                        preset,
                        bold(14.0),
                        TEXT,
                    );
                    p.text(
                        pos2(x, rect.top() + 36.0),
                        Align2::LEFT_CENTER,
                        preset_caption(preset),
                        font(12.0),
                        TEXT_DIM,
                    );
                    if resp.clicked() {
                        let id = studio.project.add_instrument_track(preset);
                        studio.sync();
                        studio.select(id);
                        dialog.open = false;
                    }
                }
            }
        });
    if r.should_close() {
        dialog.open = false;
    }
}
