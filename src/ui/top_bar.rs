//! Transport, position display, project menu and the "Add Track" dialog.

use egui::{Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

use super::icons;
use super::studio::{Studio, bar_and_beat};
use super::theme::{
    self, ACCENT, BORDER, RECORD, SURFACE_2, SURFACE_3, TEXT, TEXT_DIM, TEXT_FAINT, bold, font,
};

pub const HEIGHT: f32 = 56.0;

pub enum Action {
    None,
    New,
    Open,
    Save,
    Export,
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
                            studio.project.set_tempo(tempo.round());
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

pub fn show(
    ui: &mut Ui,
    studio: &mut Studio,
    dialog: &mut super::sound_browser::Browser,
) -> Action {
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
                ui.separator();
                if item(ui, icons::DOWNLOAD, "Export Song…", "Ctrl+E") {
                    action = Action::Export;
                }
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new(&studio.project.name).font(bold(14.0)));
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_enabled_ui(studio.can_undo(), |ui| {
                if theme::icon_button(ui, icons::UNDO, 28.0, false)
                    .on_hover_text("Undo  (Ctrl+Z)")
                    .clicked()
                {
                    studio.undo();
                }
            });
            ui.add_enabled_ui(studio.can_redo(), |ui| {
                if theme::icon_button(ui, icons::REDO, 28.0, false)
                    .on_hover_text("Redo  (Ctrl+Shift+Z)")
                    .clicked()
                {
                    studio.redo();
                }
            });
        });
    });

    // Center: transport + display
    let center = Rect::from_center_size(full.center(), vec2(680.0, HEIGHT));
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
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut metronome = studio.metronome;
            if theme::icon_button(ui, icons::METRONOME, 30.0, metronome)
                .on_hover_text("Metronome: click on every beat")
                .clicked()
            {
                metronome = !metronome;
                studio.set_metronome(metronome);
            }
            if theme::icon_button(ui, icons::COUNT_IN, 30.0, studio.count_in)
                .on_hover_text("Count-in: one bar of clicks before recording starts")
                .clicked()
            {
                studio.count_in = !studio.count_in;
            }
            if theme::icon_button(ui, icons::REPEAT, 30.0, studio.loop_on)
                .on_hover_text("Loop: repeat the range marked on the ruler")
                .clicked()
            {
                studio.set_loop(!studio.loop_on);
            }
            ui.add_space(8.0);
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
                dialog.open_new_track();
            }
        },
    );
    ui.allocate_rect(full, Sense::hover());
    action
}
