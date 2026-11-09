//! Bottom strip: a live two-octave keyboard that lights up and can be clicked.

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::icons;
use super::studio::Studio;
use super::theme::{self, ACCENT, BG, TEXT, TEXT_DIM, TEXT_FAINT, bold, font};
use crate::midi::keyboard::{MAX_OCTAVE, MIN_OCTAVE};

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
        ui.label(
            egui::RichText::new(icons::KEYBOARD_MUSIC)
                .font(theme::icon(16.0))
                .color(ACCENT),
        );
        ui.label(egui::RichText::new(name).font(bold(13.5)));
        theme::vsep(ui, 14.0);
        if keyboards.is_empty() {
            ui.label(
                egui::RichText::new("Computer keyboard: A – K play notes, Z / X change octave")
                    .color(TEXT_DIM),
            );
        } else {
            let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter()
                .circle_filled(dot.center(), 4.0, Color32::from_rgb(0x34, 0xc3, 0x8f));
            ui.label(egui::RichText::new(keyboards.join(", ")).color(TEXT_DIM));
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
