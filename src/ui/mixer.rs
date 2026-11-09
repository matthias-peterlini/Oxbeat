//! Bottom panel: a fader, pan knob and level meter for every track, plus the master.

use egui::{Align2, Color32, Rect, Sense, Ui, pos2, vec2};

use super::keyboard_strip::meter_fraction;
use super::studio::Studio;
use super::theme::{
    self, ACCENT, BG, MUTE, SURFACE_2, SURFACE_3, TEXT, TEXT_DIM, TEXT_FAINT, bold, font,
    track_color,
};
use crate::project::Track;

const STRIP_WIDTH: f32 = 78.0;
const FADER_HEIGHT: f32 = 96.0;
const MAX_VOLUME: f32 = 1.5;
const DEFAULT_VOLUME: f32 = 0.8;
const LEVEL_OK: Color32 = Color32::from_rgb(0x34, 0xc3, 0x8f);
const SOLO: Color32 = Color32::from_rgb(0x34, 0xc3, 0x8f);

/// A fader position as decibels ("-∞" at the bottom).
pub fn volume_label(volume: f32) -> String {
    if volume <= 0.001 {
        "-∞ dB".into()
    } else {
        format!("{:+.1} dB", 20.0 * volume.log10())
    }
}

/// A vertical level bar.
fn meter(ui: &Ui, rect: Rect, level: f32) {
    let p = ui.painter();
    p.rect_filled(rect, 2, BG);
    let fraction = meter_fraction(level);
    if fraction <= 0.0 {
        return;
    }
    let color = if level >= 0.97 {
        theme::RECORD
    } else if fraction > 0.85 {
        MUTE
    } else {
        LEVEL_OK
    };
    let top = rect.bottom() - rect.height() * fraction;
    p.rect_filled(
        Rect::from_min_max(pos2(rect.left(), top), rect.max),
        2,
        color,
    );
}

/// A vertical fader for `volume` (0.0..=`MAX_VOLUME`). True when moved.
fn fader(ui: &mut Ui, rect: Rect, volume: &mut f32, reset_to: f32) -> bool {
    let resp = ui.allocate_rect(rect, Sense::click_and_drag());
    let before = *volume;
    if resp.double_clicked() {
        *volume = reset_to;
    } else if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let t = ((rect.bottom() - 6.0 - p.y) / (rect.height() - 12.0)).clamp(0.0, 1.0);
        *volume = t * MAX_VOLUME;
    }
    let p = ui.painter();
    let x = rect.center().x;
    p.rect_filled(
        Rect::from_min_max(
            pos2(x - 1.5, rect.top() + 4.0),
            pos2(x + 1.5, rect.bottom() - 4.0),
        ),
        2,
        BG,
    );
    let t = (*volume / MAX_VOLUME).clamp(0.0, 1.0);
    let y = rect.bottom() - 6.0 - t * (rect.height() - 12.0);
    let handle = Rect::from_center_size(pos2(x, y), vec2(20.0, 12.0));
    let fill = if resp.hovered() || resp.dragged() {
        TEXT
    } else {
        Color32::from_rgb(0xb4, 0xb7, 0xbe)
    };
    p.rect_filled(handle, 3, fill);
    p.line_segment(
        [pos2(x - 6.0, y), pos2(x + 6.0, y)],
        egui::Stroke::new(1.0, BG),
    );
    resp.on_hover_text(volume_label(*volume));
    *volume != before
}

fn strip_frame(ui: &mut Ui, selected: bool, add: impl FnOnce(&mut Ui)) -> egui::Response {
    egui::Frame::NONE
        .fill(if selected { SURFACE_3 } else { SURFACE_2 })
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(6, 6))
        .show(ui, |ui| {
            ui.set_width(STRIP_WIDTH - 12.0);
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                add(ui);
            });
        })
        .response
}

fn name_label(ui: &mut Ui, name: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(STRIP_WIDTH - 12.0, 16.0), Sense::hover());
    ui.painter()
        .rect_filled(Rect::from_min_size(rect.min, vec2(3.0, 16.0)), 1, color);
    let clip = ui.painter_at(rect);
    clip.text(
        rect.left_center() + vec2(8.0, 0.0),
        Align2::LEFT_CENTER,
        name,
        bold(11.5),
        TEXT,
    );
}

/// Returns true when the track changed.
fn track_strip(ui: &mut Ui, track: &mut Track, color: Color32, level: f32, selected: bool) -> bool {
    let mut changed = false;
    strip_frame(ui, selected, |ui| {
        name_label(ui, &track.name, color);
        let mut pan = (track.pan + 1.0) / 2.0;
        let r = theme::knob(ui, &mut pan, 0.5, 26.0, ACCENT).on_hover_text(pan_label(track.pan));
        if r.changed() {
            track.pan = pan * 2.0 - 1.0;
            // Snap to the middle so "centre" is easy to hit.
            if track.pan.abs() < 0.04 {
                track.pan = 0.0;
            }
            changed = true;
        }
        let (area, _) = ui.allocate_exact_size(vec2(44.0, FADER_HEIGHT), Sense::hover());
        let fader_rect = Rect::from_min_size(area.min, vec2(28.0, FADER_HEIGHT));
        changed |= fader(ui, fader_rect, &mut track.volume, DEFAULT_VOLUME);
        meter(
            ui,
            Rect::from_min_max(
                pos2(area.right() - 8.0, area.top() + 4.0),
                area.max - vec2(0.0, 4.0),
            ),
            level,
        );
        ui.label(
            egui::RichText::new(volume_label(track.volume))
                .font(font(10.0))
                .color(TEXT_DIM),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space((ui.available_width() - 48.0).max(0.0) / 2.0);
            changed |= theme::toggle_chip(ui, "M", &mut track.mute, MUTE)
                .on_hover_text("Mute")
                .clicked();
            changed |= theme::toggle_chip(ui, "S", &mut track.solo, SOLO)
                .on_hover_text("Solo")
                .clicked();
        });
    });
    changed
}

fn pan_label(pan: f32) -> String {
    if pan.abs() < 0.005 {
        "Centre".into()
    } else if pan < 0.0 {
        format!("{:.0}% left", -pan * 100.0)
    } else {
        format!("{:.0}% right", pan * 100.0)
    }
}

/// Returns true when the master volume changed.
fn master_strip(ui: &mut Ui, volume: &mut f32, levels: [f32; 2]) -> bool {
    let mut changed = false;
    strip_frame(ui, false, |ui| {
        name_label(ui, "Master", TEXT);
        // Same height as a track's pan knob, so the faders line up.
        let (rect, _) = ui.allocate_exact_size(vec2(STRIP_WIDTH - 12.0, 26.0), Sense::hover());
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "OUTPUT",
            font(9.5),
            TEXT_FAINT,
        );
        let (area, _) = ui.allocate_exact_size(vec2(52.0, FADER_HEIGHT), Sense::hover());
        let fader_rect = Rect::from_min_size(area.min, vec2(28.0, FADER_HEIGHT));
        changed |= fader(ui, fader_rect, volume, 1.0);
        for (i, level) in levels.iter().enumerate() {
            let x = area.right() - 18.0 + i as f32 * 10.0;
            meter(
                ui,
                Rect::from_min_max(
                    pos2(x, area.top() + 4.0),
                    pos2(x + 8.0, area.bottom() - 4.0),
                ),
                *level,
            );
        }
        ui.label(
            egui::RichText::new(volume_label(*volume))
                .font(font(10.0))
                .color(TEXT_DIM),
        );
        let clipping = levels.iter().any(|l| *l >= 0.97);
        ui.label(
            egui::RichText::new(if clipping { "LIMITING" } else { "LIMITER ON" })
                .font(font(9.0))
                .color(if clipping { MUTE } else { TEXT_FAINT }),
        )
        .on_hover_text("The master limiter keeps a loud mix from distorting");
    });
    changed
}

pub fn show(ui: &mut Ui, studio: &mut Studio) {
    let selected = studio.selected();
    let mut changed = false;
    let mut select = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        // The master stays put on the right; tracks scroll beside it.
        let tracks_width = (ui.available_width() - STRIP_WIDTH - 14.0).max(STRIP_WIDTH);
        ui.allocate_ui(vec2(tracks_width, 0.0), |ui| {
            egui::ScrollArea::horizontal()
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for (i, track) in studio.project.tracks.iter_mut().enumerate() {
                            let level = studio.track_levels.get(i).copied().unwrap_or(0.0);
                            let id = track.id;
                            let r = ui
                                .push_id(id, |ui| {
                                    changed |= track_strip(
                                        ui,
                                        track,
                                        track_color(i),
                                        level,
                                        id == selected,
                                    );
                                })
                                .response;
                            if r.interact(Sense::click()).clicked() {
                                select = Some(id);
                            }
                        }
                    });
                });
        });
        theme::vsep(ui, FADER_HEIGHT + 96.0);
        changed |= master_strip(ui, &mut studio.project.master_volume, studio.master_level);
    });
    if let Some(id) = select {
        studio.select(id);
    }
    if changed {
        studio.sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_reads_in_decibels() {
        assert_eq!(volume_label(1.0), "+0.0 dB");
        assert_eq!(volume_label(0.5), "-6.0 dB");
        assert_eq!(volume_label(0.0), "-∞ dB");
    }

    #[test]
    fn pan_reads_as_a_side() {
        assert_eq!(pan_label(0.0), "Centre");
        assert_eq!(pan_label(-0.5), "50% left");
        assert_eq!(pan_label(1.0), "100% right");
    }
}
