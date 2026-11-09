//! Transport, LCD, project menu and the "+ Track" dialog.

use egui::{Color32, RichText};

use super::studio::{Studio, format_position};
use super::{ACCENT, LCD, LCD_TEXT, RECORD};
use crate::engine::synth::PRESETS;

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

fn big(text: &str) -> RichText {
    RichText::new(text).size(18.0)
}

pub fn show(ui: &mut egui::Ui, studio: &mut Studio, dialog: &mut AddTrackDialog) -> Action {
    let mut action = Action::None;
    ui.horizontal(|ui| {
        ui.menu_button(big("≡"), |ui| {
            if ui.button("New project   Ctrl+N").clicked() {
                action = Action::New;
            }
            if ui.button("Open…   Ctrl+O").clicked() {
                action = Action::Open;
            }
            if ui.button("Save   Ctrl+S").clicked() {
                action = Action::Save;
            }
        });
        ui.add_space(8.0);

        if ui
            .button(big("⏮"))
            .on_hover_text("Back to start (Enter)")
            .clicked()
        {
            studio.rewind();
        }
        let play = if studio.playing { "⏹" } else { "▶" };
        if ui
            .button(big(play))
            .on_hover_text("Play / stop (Space)")
            .clicked()
        {
            studio.play_stop();
        }
        let rec_color = if studio.recording {
            Color32::WHITE
        } else {
            RECORD
        };
        let rec = egui::Button::new(big("⏺").color(rec_color)).fill(if studio.recording {
            RECORD
        } else {
            ui.visuals().widgets.inactive.weak_bg_fill
        });
        if ui
            .add(rec)
            .on_hover_text("Record on the selected track (R)")
            .clicked()
        {
            studio.record();
        }
        ui.add_space(16.0);

        egui::Frame::NONE
            .fill(LCD)
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(14, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let lcd = |t: String| RichText::new(t).monospace().size(18.0).color(LCD_TEXT);
                    ui.label(lcd(format_position(studio.position)));
                    ui.label(lcd("·".into()));
                    let mut tempo = studio.project.tempo_bpm;
                    let r = ui
                        .add(
                            egui::DragValue::new(&mut tempo)
                                .range(40.0..=240.0)
                                .speed(0.5)
                                .fixed_decimals(0),
                        )
                        .on_hover_text("Tempo — drag to change");
                    if r.changed() {
                        studio.project.tempo_bpm = tempo.round();
                        studio.sync();
                    }
                    ui.label(lcd("BPM ·".into()));
                    ui.label(lcd(studio.project.key.clone()));
                });
            });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let add = egui::Button::new(big("+ Track").color(Color32::WHITE)).fill(ACCENT);
            if ui.add(add).clicked() {
                dialog.open = true;
                dialog.choosing_sound = false;
            }
        });
    });
    action
}

pub fn add_track_dialog(ctx: &egui::Context, studio: &mut Studio, dialog: &mut AddTrackDialog) {
    if !dialog.open {
        return;
    }
    let r = egui::Modal::new(egui::Id::unique("add-track")).show(ctx, |ui| {
        ui.set_width(420.0);
        if !dialog.choosing_sound {
            ui.heading("What do you want to add?");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let size = egui::vec2(124.0, 96.0);
                if ui
                    .add_sized(size, egui::Button::new(big("🎹\nInstrument")))
                    .clicked()
                {
                    dialog.choosing_sound = true;
                }
                ui.add_enabled(false, egui::Button::new(big("🎤\nMic")).min_size(size))
                    .on_disabled_hover_text("Coming soon");
                ui.add_enabled(false, egui::Button::new(big("🥁\nDrums")).min_size(size))
                    .on_disabled_hover_text("Coming soon");
            });
        } else {
            ui.heading("Pick a sound");
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                for preset in PRESETS {
                    if ui
                        .add_sized(egui::vec2(96.0, 56.0), egui::Button::new(big(preset)))
                        .clicked()
                    {
                        let id = studio.project.add_instrument_track(preset);
                        studio.sync();
                        studio.select(id);
                        dialog.open = false;
                    }
                }
            });
        }
        ui.add_space(12.0);
        if ui.button("Cancel").clicked() {
            dialog.open = false;
        }
    });
    if r.should_close() {
        dialog.open = false;
    }
}
