//! Bottom panel: the selected track's effects, as a row of cards with knobs.

use egui::{Sense, Ui, vec2};

use super::icons;
use super::studio::Studio;
use super::theme::{self, ACCENT, SURFACE_2, TEXT, TEXT_DIM, TEXT_FAINT, bold, font};
use crate::project::effects::{Effect, EffectKind, PRESETS, Param, Unit, preset};

const KNOB_SIZE: f32 = 38.0;
const KNOB_CELL: f32 = 62.0;
const CHOICE_WIDTH: f32 = 170.0;
const CARD_HEIGHT: f32 = 104.0;
const ON_COLOR: egui::Color32 = egui::Color32::from_rgb(0x34, 0xc3, 0x8f);

/// Knobs whose range spans decades feel right on a logarithmic scale.
fn is_log(p: &Param) -> bool {
    matches!(p.unit, Unit::Hz | Unit::Ms) && p.min > 0.0 && p.max / p.min >= 20.0
}

/// Where a value sits on its knob, 0.0..=1.0.
pub fn to_knob(p: &Param, value: f32) -> f32 {
    let t = if is_log(p) {
        (value / p.min).ln() / (p.max / p.min).ln()
    } else {
        (value - p.min) / (p.max - p.min)
    };
    t.clamp(0.0, 1.0)
}

pub fn from_knob(p: &Param, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if is_log(p) {
        p.min * (p.max / p.min).powf(t)
    } else {
        p.min + (p.max - p.min) * t
    }
}

pub fn format_value(p: &Param, value: f32) -> String {
    match p.unit {
        Unit::Percent => format!("{:.0}%", value * 100.0),
        Unit::Db => format!("{value:+.1} dB"),
        Unit::Hz if value >= 1000.0 => format!("{:.1} kHz", value / 1000.0),
        Unit::Hz if value >= 20.0 => format!("{value:.0} Hz"),
        Unit::Hz => format!("{value:.1} Hz"),
        Unit::Ms if value >= 10.0 => format!("{value:.0} ms"),
        Unit::Ms => format!("{value:.1} ms"),
        Unit::Ratio => format!("{value:.1} : 1"),
        Unit::Choice(names) => names
            .get(value.round() as usize)
            .copied()
            .unwrap_or("")
            .to_string(),
    }
}

enum Edit {
    None,
    Changed,
    Remove(usize),
    /// Swap with the next effect.
    MoveRight(usize),
    MoveLeft(usize),
}

pub fn show(ui: &mut Ui, studio: &mut Studio) {
    let selected = studio.selected();
    let Some(track) = studio.project.track_mut(selected) else {
        ui.label(egui::RichText::new("Select a track to add effects").color(TEXT_DIM));
        return;
    };
    let mut edit = Edit::None;

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(icons::SLIDERS)
                .font(theme::icon(16.0))
                .color(ACCENT),
        );
        ui.label(egui::RichText::new(format!("{} effects", track.name)).font(bold(13.5)));
        theme::vsep(ui, 14.0);
        let hint = if track.effects.is_empty() {
            "No effects yet. Add one, or start from a preset."
        } else {
            "Sound flows left to right. Drag a knob; double-click to reset it."
        };
        ui.label(egui::RichText::new(hint).color(TEXT_DIM));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let add = ui.add(
                egui::Button::new(theme::icon_text(icons::PLUS, "Add effect", 13.0, TEXT))
                    .min_size(vec2(0.0, 26.0)),
            );
            egui::Popup::menu(&add).show(|ui| {
                for kind in EffectKind::ALL {
                    let text = egui::RichText::new(kind.name()).font(bold(13.0));
                    let r = ui
                        .add(egui::Button::new(text).min_size(vec2(230.0, 28.0)))
                        .on_hover_text(kind.caption());
                    if r.clicked() {
                        track.effects.push(Effect::new(kind));
                        edit = Edit::Changed;
                    }
                }
            });
            let presets = ui.add(
                egui::Button::new(theme::icon_text(icons::SPARKLES, "Presets", 13.0, TEXT))
                    .min_size(vec2(0.0, 26.0)),
            );
            egui::Popup::menu(&presets).show(|ui| {
                for name in PRESETS {
                    if ui
                        .add(egui::Button::new(name).min_size(vec2(170.0, 28.0)))
                        .clicked()
                    {
                        track.effects = preset(name);
                        edit = Edit::Changed;
                    }
                }
                if !track.effects.is_empty() {
                    ui.separator();
                    if ui
                        .add(egui::Button::new("Remove all").min_size(vec2(170.0, 28.0)))
                        .clicked()
                    {
                        track.effects.clear();
                        edit = Edit::Changed;
                    }
                }
            });
        });
    });
    ui.add_space(6.0);

    let count = track.effects.len();
    egui::ScrollArea::horizontal()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.set_min_height(CARD_HEIGHT);
                for (i, effect) in track.effects.iter_mut().enumerate() {
                    ui.push_id(i, |ui| card(ui, effect, i, count, &mut edit));
                }
                if count == 0 {
                    let (rect, _) = ui.allocate_exact_size(
                        vec2(ui.available_width().max(200.0), CARD_HEIGHT),
                        Sense::hover(),
                    );
                    ui.painter().rect_filled(rect, 8, SURFACE_2);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Reverb, delay, EQ, compressor, pitch correction and more",
                        font(13.0),
                        TEXT_FAINT,
                    );
                }
            });
        });

    match edit {
        Edit::None => return,
        Edit::Changed => {}
        Edit::Remove(i) => {
            track.effects.remove(i);
        }
        Edit::MoveRight(i) => track.effects.swap(i, i + 1),
        Edit::MoveLeft(i) => track.effects.swap(i, i - 1),
    }
    studio.sync();
}

fn card(ui: &mut Ui, effect: &mut Effect, index: usize, count: usize, edit: &mut Edit) {
    let specs = effect.kind.params();
    // Wide enough for the knobs, and for the title with its buttons.
    let knobs: f32 = specs
        .iter()
        .map(|p| match p.unit {
            Unit::Choice(_) => CHOICE_WIDTH + 14.0,
            _ => KNOB_CELL + 2.0,
        })
        .sum();
    let title = ui
        .painter()
        .layout_no_wrap(effect.kind.name().into(), bold(12.5), TEXT)
        .size()
        .x;
    let width = knobs.max(title + 22.0 + 8.0 + 14.0 + 60.0);
    egui::Frame::NONE
        .fill(SURFACE_2)
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_height(CARD_HEIGHT - 16.0);
            ui.set_width(width);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if theme::icon_chip(ui, icons::POWER, &mut effect.on, ON_COLOR)
                        .on_hover_text("Turn this effect on or off")
                        .clicked()
                    {
                        *edit = Edit::Changed;
                    }
                    ui.add_space(4.0);
                    let color = if effect.on { TEXT } else { TEXT_DIM };
                    ui.label(
                        egui::RichText::new(effect.kind.name())
                            .font(bold(12.5))
                            .color(color),
                    );
                    ui.add_space(10.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        if theme::icon_button(ui, icons::X, 20.0, false)
                            .on_hover_text("Remove")
                            .clicked()
                        {
                            *edit = Edit::Remove(index);
                        }
                        if index + 1 < count
                            && theme::icon_button(ui, icons::CHEVRON_RIGHT, 20.0, false)
                                .on_hover_text("Move later in the chain")
                                .clicked()
                        {
                            *edit = Edit::MoveRight(index);
                        }
                        if index > 0
                            && theme::icon_button(ui, icons::CHEVRON_LEFT, 20.0, false)
                                .on_hover_text("Move earlier in the chain")
                                .clicked()
                        {
                            *edit = Edit::MoveLeft(index);
                        }
                    });
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (i, spec) in specs.iter().enumerate() {
                        if control(ui, effect, i, spec) {
                            *edit = Edit::Changed;
                        }
                    }
                });
            });
        });
}

/// One knob (or list) with its name and value. True when the value changed.
fn control(ui: &mut Ui, effect: &mut Effect, i: usize, spec: &Param) -> bool {
    let value = effect.get(i);
    if let Unit::Choice(names) = spec.unit {
        let mut choice = value.round() as usize;
        ui.vertical(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(spec.name.to_uppercase())
                    .font(font(9.5))
                    .color(TEXT_FAINT),
            );
            egui::ComboBox::from_id_salt(("choice", i))
                .width(CHOICE_WIDTH)
                .selected_text(names.get(choice).copied().unwrap_or(""))
                .show_ui(ui, |ui| {
                    for (n, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut choice, n, *name);
                    }
                });
        });
        if choice != value.round() as usize {
            effect.set(i, choice as f32);
            return true;
        }
        return false;
    }

    let mut changed = false;
    ui.allocate_ui_with_layout(
        vec2(KNOB_CELL, KNOB_SIZE + 30.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let mut t = to_knob(spec, value);
            let color = if effect.on { ACCENT } else { TEXT_FAINT };
            let r = theme::knob(ui, &mut t, to_knob(spec, spec.default), KNOB_SIZE, color);
            if r.changed() {
                effect.set(i, from_knob(spec, t));
                changed = true;
            }
            // Show the number while it is being set, the name otherwise.
            let (text, color) = if r.hovered() || r.dragged() {
                (format_value(spec, effect.get(i)), TEXT)
            } else {
                (spec.name.to_string(), TEXT_DIM)
            };
            ui.label(egui::RichText::new(text).font(font(10.5)).color(color));
        },
    );
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knob_positions_round_trip_for_every_knob() {
        for kind in EffectKind::ALL {
            for spec in kind.params() {
                for t in [0.0, 0.25, 0.5, 1.0] {
                    let back = to_knob(spec, from_knob(spec, t));
                    assert!((back - t).abs() < 1e-4, "{} {t} -> {back}", spec.name);
                }
                assert_eq!(from_knob(spec, 0.0), spec.min);
                assert!((from_knob(spec, 1.0) - spec.max).abs() <= spec.max.abs() * 1e-4);
            }
        }
    }

    #[test]
    fn values_read_naturally() {
        let find =
            |kind: EffectKind, name: &str| *kind.params().iter().find(|p| p.name == name).unwrap();
        assert_eq!(format_value(&find(EffectKind::Reverb, "Mix"), 0.25), "25%");
        assert_eq!(format_value(&find(EffectKind::Eq, "Low"), 3.0), "+3.0 dB");
        assert_eq!(
            format_value(&find(EffectKind::Eq, "Mid Freq"), 2500.0),
            "2.5 kHz"
        );
        assert_eq!(
            format_value(&find(EffectKind::Delay, "Time"), 350.0),
            "350 ms"
        );
        assert_eq!(
            format_value(&find(EffectKind::Compressor, "Ratio"), 4.0),
            "4.0 : 1"
        );
        assert_eq!(
            format_value(&find(EffectKind::PitchCorrect, "Scale"), 1.0),
            "C major / A minor"
        );
    }
}
