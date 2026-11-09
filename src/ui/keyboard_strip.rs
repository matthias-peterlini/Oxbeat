//! Bottom strip: a live two-octave keyboard that lights up and can be clicked.

use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use super::ACCENT;
use super::studio::Studio;
use crate::midi::keyboard::{MAX_OCTAVE, MIN_OCTAVE};

const OCTAVES: u8 = 2;
const KEY_HEIGHT: f32 = 72.0;
const MOUSE_VELOCITY: u8 = 100;

fn is_black(semitone: u8) -> bool {
    matches!(semitone % 12, 1 | 3 | 6 | 8 | 10)
}

pub fn show(
    ui: &mut egui::Ui,
    studio: &Studio,
    octave: &mut i8,
    mouse_note: &mut Option<u8>,
    keyboards: &[String],
) {
    let name = studio
        .project
        .track(studio.selected())
        .map_or("No track selected".to_string(), |t| t.name.clone());
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("🎹 {name}")).strong());
        ui.separator();
        if keyboards.is_empty() {
            ui.label("Play with your keyboard: A S D F G H J K (Z / X change octave)");
        } else {
            ui.label(format!("Playing from {}", keyboards.join(", ")));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(*octave < MAX_OCTAVE, egui::Button::new("▶"))
                .clicked()
            {
                *octave += 1;
            }
            ui.label(format!("Octave C{octave}"));
            if ui
                .add_enabled(*octave > MIN_OCTAVE, egui::Button::new("◀"))
                .clicked()
            {
                *octave -= 1;
            }
        });
    });
    ui.add_space(4.0);

    let base = ((*octave as i32 + 1) * 12).clamp(0, 127 - 24) as u8;
    let keys = 12 * OCTAVES + 1;
    let whites = (0..keys).filter(|s| !is_black(*s)).count() as f32;
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, KEY_HEIGHT), Sense::click_and_drag());
    let white_w = width / whites;
    let painter = ui.painter_at(rect);

    // Layout: white keys first, black keys on top.
    let mut white_rects = Vec::new();
    let mut black_rects = Vec::new();
    let mut wi = 0.0;
    for s in 0..keys {
        let pitch = base + s;
        if is_black(s) {
            let x = rect.left() + wi * white_w - white_w * 0.3;
            black_rects.push((
                pitch,
                Rect::from_min_size(pos2(x, rect.top()), vec2(white_w * 0.6, KEY_HEIGHT * 0.6)),
            ));
        } else {
            let x = rect.left() + wi * white_w;
            white_rects.push((
                pitch,
                Rect::from_min_size(pos2(x, rect.top()), vec2(white_w - 1.0, KEY_HEIGHT)),
            ));
            wi += 1.0;
        }
    }

    for (pitch, r) in &white_rects {
        let lit = studio.shared.is_held(*pitch);
        painter.rect_filled(*r, 3.0, if lit { ACCENT } else { Color32::from_gray(232) });
        if pitch % 12 == 0 {
            painter.text(
                r.center_bottom() - vec2(0.0, 6.0),
                egui::Align2::CENTER_BOTTOM,
                format!("C{}", *pitch as i32 / 12 - 1),
                egui::FontId::proportional(10.0),
                Color32::from_gray(110),
            );
        }
    }
    for (pitch, r) in &black_rects {
        let lit = studio.shared.is_held(*pitch);
        painter.rect_filled(*r, 2.0, if lit { ACCENT } else { Color32::from_gray(25) });
        painter.rect_stroke(
            *r,
            2.0,
            Stroke::new(1.0, Color32::BLACK),
            StrokeKind::Inside,
        );
    }

    // Mouse playing (black keys take priority since they sit on top).
    let hovered = resp.interact_pointer_pos().and_then(|p| {
        black_rects
            .iter()
            .chain(white_rects.iter())
            .find(|(_, r)| r.contains(p))
            .map(|(pitch, _)| *pitch)
    });
    let down = resp.is_pointer_button_down_on();
    let want = if down { hovered } else { None };
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
