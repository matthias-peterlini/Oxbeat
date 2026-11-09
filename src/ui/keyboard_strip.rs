//! Bottom strip: a live two-octave keyboard that lights up and can be clicked.

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::icons;
use super::studio::Studio;
use super::theme::{self, ACCENT, BG, TEXT, TEXT_DIM, TEXT_FAINT, bold, font};
use crate::engine::drums::PADS;
use crate::midi::keyboard::{MAX_OCTAVE, MIN_OCTAVE, pad_key_label};

const OCTAVES: u8 = 2;
const KEY_HEIGHT: f32 = 78.0;
const MOUSE_VELOCITY: u8 = 100;
const WHITE_KEY: Color32 = Color32::from_rgb(0xec, 0xed, 0xf0);
const BLACK_KEY: Color32 = Color32::from_rgb(0x1a, 0x1b, 0x1f);

fn is_black(semitone: u8) -> bool {
    matches!(semitone % 12, 1 | 3 | 6 | 8 | 10)
}

pub fn show(
    ui: &mut Ui,
    studio: &mut Studio,
    octave: &mut i8,
    mouse_note: &mut Option<u8>,
    keyboards: &[String],
) {
    let track = studio.project.track(studio.selected());
    let name = track.map_or("No track selected".to_string(), |t| t.name.clone());
    let drums = track.is_some_and(|t| t.is_drums());
    if track.is_some_and(|t| t.is_audio()) {
        voice_strip(ui, studio, &name);
        return;
    }
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(if drums {
                icons::DRUM
            } else {
                icons::KEYBOARD_MUSIC
            })
            .font(theme::icon(16.0))
            .color(ACCENT),
        );
        ui.label(egui::RichText::new(name).font(bold(13.5)));
        theme::vsep(ui, 14.0);
        if keyboards.is_empty() {
            let hint = if drums {
                "Computer keyboard: A – K and W E T Y play the pads"
            } else {
                "Computer keyboard: A – K play notes, Z / X change octave"
            };
            ui.label(egui::RichText::new(hint).color(TEXT_DIM));
        } else {
            let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter()
                .circle_filled(dot.center(), 4.0, Color32::from_rgb(0x34, 0xc3, 0x8f));
            ui.label(egui::RichText::new(keyboards.join(", ")).color(TEXT_DIM));
        }
        if drums {
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if theme::icon_button(ui, icons::CHEVRON_RIGHT, 26.0, false)
                .on_hover_text("Octave up  (X)")
                .clicked()
            {
                *octave = (*octave + 1).min(MAX_OCTAVE);
            }
            ui.label(egui::RichText::new(format!("C{octave}")).font(bold(13.0)));
            if theme::icon_button(ui, icons::CHEVRON_LEFT, 26.0, false)
                .on_hover_text("Octave down  (Z)")
                .clicked()
            {
                *octave = (*octave - 1).max(MIN_OCTAVE);
            }
            ui.label(
                egui::RichText::new("OCTAVE")
                    .font(font(9.5))
                    .color(TEXT_FAINT),
            );
        });
    });
    ui.add_space(6.0);
    if drums {
        drum_pads(ui, studio, mouse_note);
        return;
    }

    let base = ((*octave as i32 + 1) * 12).clamp(0, 127 - 24) as u8;
    let keys = 12 * OCTAVES + 1;
    let whites = (0..keys).filter(|s| !is_black(*s)).count() as f32;
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, KEY_HEIGHT), Sense::click_and_drag());
    let gap = 2.0;
    let white_w = (width + gap) / whites;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6, BG);

    let mut white_rects = Vec::new();
    let mut black_rects = Vec::new();
    let mut wi = 0.0;
    for s in 0..keys {
        let pitch = base + s;
        if is_black(s) {
            let x = rect.left() + wi * white_w - white_w * 0.31 - gap / 2.0;
            black_rects.push((
                pitch,
                Rect::from_min_size(pos2(x, rect.top()), vec2(white_w * 0.62, KEY_HEIGHT * 0.6)),
            ));
        } else {
            let x = rect.left() + wi * white_w;
            white_rects.push((
                pitch,
                Rect::from_min_size(pos2(x, rect.top()), vec2(white_w - gap, KEY_HEIGHT)),
            ));
            wi += 1.0;
        }
    }

    let bottom_round = egui::CornerRadius {
        nw: 0,
        ne: 0,
        sw: 5,
        se: 5,
    };
    for (pitch, r) in &white_rects {
        let lit = studio.shared.is_held(*pitch);
        painter.rect_filled(*r, bottom_round, if lit { ACCENT } else { WHITE_KEY });
        if pitch % 12 == 0 {
            painter.text(
                r.center_bottom() - vec2(0.0, 8.0),
                Align2::CENTER_BOTTOM,
                format!("C{}", *pitch as i32 / 12 - 1),
                font(10.0),
                if lit { TEXT } else { Color32::from_gray(140) },
            );
        }
    }
    for (pitch, r) in &black_rects {
        let lit = studio.shared.is_held(*pitch);
        painter.rect_filled(*r, bottom_round, if lit { ACCENT } else { BLACK_KEY });
        painter.rect_stroke(
            *r,
            bottom_round,
            Stroke::new(1.0, Color32::from_black_alpha(160)),
            StrokeKind::Outside,
        );
    }

    // Mouse playing (black keys first since they sit on top).
    let hovered = resp.interact_pointer_pos().and_then(|p| {
        black_rects
            .iter()
            .chain(white_rects.iter())
            .find(|(_, r)| r.contains(p))
            .map(|(pitch, _)| *pitch)
    });
    let want = if resp.is_pointer_button_down_on() {
        hovered
    } else {
        None
    };
    play_mouse(studio, mouse_note, want);
}

fn play_mouse(studio: &Studio, mouse_note: &mut Option<u8>, want: Option<u8>) {
    if want != *mouse_note {
        if let Some(p) = mouse_note.take() {
            studio.shared.note_off(p);
        }
        if let Some(p) = want {
            studio.shared.note_on(p, MOUSE_VELOCITY);
            *mouse_note = Some(p);
        }
    }
}

/// A row of 12 drum pads that light up and can be clicked.
fn drum_pads(ui: &mut Ui, studio: &Studio, mouse_note: &mut Option<u8>) {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, KEY_HEIGHT), Sense::click_and_drag());
    let gap = 6.0;
    let n = PADS.len() as f32;
    let w = (width - gap * (n - 1.0)) / n;
    let painter = ui.painter_at(rect);
    let mut hit = None;
    for (i, (note, label)) in PADS.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(rect.left() + i as f32 * (w + gap), rect.top()),
            vec2(w, KEY_HEIGHT),
        );
        let lit = studio.shared.is_held(*note);
        let hover = resp.hover_pos().is_some_and(|p| r.contains(p));
        let fill = if lit {
            ACCENT
        } else if hover {
            theme::SURFACE_3
        } else {
            theme::SURFACE_2
        };
        painter.rect_filled(r, 8, fill);
        painter.text(
            r.center() - vec2(0.0, 6.0),
            Align2::CENTER_CENTER,
            *label,
            bold(12.5),
            TEXT,
        );
        painter.text(
            r.center_bottom() - vec2(0.0, 12.0),
            Align2::CENTER_CENTER,
            pad_key_label(i),
            font(10.5),
            if lit { TEXT } else { TEXT_FAINT },
        );
        if resp.is_pointer_button_down_on()
            && resp.interact_pointer_pos().is_some_and(|p| r.contains(p))
        {
            hit = Some(*note);
        }
    }
    play_mouse(studio, mouse_note, hit);
}

const LEVEL_OK: Color32 = Color32::from_rgb(0x34, 0xc3, 0x8f);
/// Quietest level the meter shows.
const METER_FLOOR_DB: f32 = -54.0;

/// How far along the meter a level (0.0..=1.0) reaches.
pub(super) fn meter_fraction(level: f32) -> f32 {
    if level <= 0.0 {
        return 0.0;
    }
    ((20.0 * level.log10() - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

/// The strip for a voice track: which microphone is in use and how loud it is.
fn voice_strip(ui: &mut Ui, studio: &mut Studio, name: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(icons::MIC)
                .font(theme::icon(16.0))
                .color(ACCENT),
        );
        ui.label(egui::RichText::new(name).font(bold(13.5)));
        theme::vsep(ui, 14.0);
        match &studio.input_name {
            Some(mic) => {
                let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.0, LEVEL_OK);
                ui.label(egui::RichText::new(mic).color(TEXT_DIM));
            }
            None => {
                ui.label(
                    egui::RichText::new("No microphone found. Plug one in to record.")
                        .color(TEXT_DIM),
                );
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = theme::icon_text(icons::IMPORT, "Import audio", 13.0, TEXT_DIM);
            if ui
                .add(egui::Button::new(label).frame(false))
                .on_hover_text("Add a WAV, MP3, FLAC or OGG file at the playhead")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("Import audio")
                    .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "aif", "aiff"])
                    .pick_file()
                && let Err(e) = studio.import_audio(&path)
            {
                studio
                    .load_errors
                    .push(format!("Could not import that file: {e}"));
            }
        });
    });
    ui.add_space(6.0);

    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, KEY_HEIGHT), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 8, theme::SURFACE_2);
    let bar = Rect::from_min_size(rect.min + vec2(20.0, 24.0), vec2(rect.width() - 40.0, 10.0));
    p.rect_filled(bar, 5, BG);
    let fraction = meter_fraction(studio.input_level);
    if fraction > 0.0 {
        let color = if studio.input_level >= 0.98 {
            theme::RECORD
        } else if fraction > 0.85 {
            theme::MUTE
        } else {
            LEVEL_OK
        };
        let fill = Rect::from_min_size(bar.min, vec2(bar.width() * fraction, bar.height()));
        p.rect_filled(fill, 5, color);
    }
    let hint = if studio.recording {
        "Recording… press R or Space to stop"
    } else if studio.input_name.is_some() {
        "Press R to record. Wear headphones while the monitor is on."
    } else {
        "Voice tracks record from your microphone"
    };
    p.text(
        pos2(rect.center().x, bar.bottom() + 20.0),
        Align2::CENTER_CENTER,
        hint,
        font(12.0),
        if studio.recording { TEXT } else { TEXT_FAINT },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_is_logarithmic() {
        assert_eq!(meter_fraction(0.0), 0.0);
        assert_eq!(meter_fraction(1.0), 1.0);
        assert_eq!(meter_fraction(0.001), 0.0);
        let half = meter_fraction(0.5);
        assert!(half > 0.85 && half < 0.92, "{half}");
    }
}
