//! Bottom panel: the notes of the selected clip on a grid. Instrument clips get a
//! piano roll; drum clips get one row per drum.

use std::time::{Duration, Instant};

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::icons;
use super::studio::Studio;
use super::theme::{
    self, ACCENT, BG, SURFACE_2, TEXT, TEXT_DIM, TEXT_FAINT, bold, font, track_color,
};
use crate::engine::drums::PADS;
use crate::project::Note;
use crate::project::edit::{MIN_LENGTH, snap};

const BODY_HEIGHT: f32 = 204.0;
const PX_PER_BEAT: f32 = 56.0;
const PIANO_ROW: f32 = 12.0;
const DRUM_ROW: f32 = 17.0;
const PIANO_KEYS_WIDTH: f32 = 52.0;
const DRUM_KEYS_WIDTH: f32 = 84.0;
const HIGHEST_PITCH: u8 = 108;
const LOWEST_PITCH: u8 = 21;
/// Distance from a note's right end that grabs its length rather than the note.
const RESIZE_GRIP: f32 = 6.0;
const NEW_VELOCITY: u8 = 100;
/// How long a note sounds when it is clicked or drawn.
const PREVIEW: Duration = Duration::from_millis(180);
const GRIDS: [(&str, f64); 4] = [("1/4", 1.0), ("1/8", 0.5), ("1/16", 0.25), ("1/32", 0.125)];
const NEW_CLIP_BEATS: f64 = 8.0;

struct Drag {
    note: usize,
    resize: bool,
    /// Beats from the note's start to where it was grabbed.
    grab: f64,
}

pub struct Editor {
    /// Grid spacing in beats.
    pub grid: f64,
    /// Index of the picked note in the clip.
    pub selected: Option<usize>,
    /// Length new notes get: whatever the last note was stretched to.
    note_length: f64,
    drag: Option<Drag>,
    /// A note sounding briefly because it was just clicked.
    preview: Option<(u8, Instant)>,
    /// A key held down with the mouse.
    key_held: Option<u8>,
    /// The clip the view was last scrolled for.
    shown: Option<u64>,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            grid: 0.25,
            selected: None,
            note_length: 0.5,
            drag: None,
            preview: None,
            key_held: None,
            shown: None,
        }
    }
}

impl Editor {
    fn stop_preview(&mut self, studio: &Studio) {
        if let Some((pitch, _)) = self.preview.take() {
            studio.shared.note_off(pitch);
        }
    }

    /// Delete the picked note. True when there was one.
    pub fn delete_selected(&mut self, studio: &mut Studio) -> bool {
        let Some(index) = self.selected.take() else {
            return false;
        };
        let Some(clip) = studio
            .selected_clip
            .and_then(|c| studio.project.clip_mut(c))
        else {
            return false;
        };
        if index >= clip.notes.len() {
            return false;
        }
        clip.notes.remove(index);
        studio.sync();
        true
    }
}

fn is_black(pitch: u8) -> bool {
    matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
}

/// The rows of the grid, top to bottom: (pitch, label).
fn rows(drums: bool) -> Vec<(u8, String)> {
    if drums {
        PADS.iter()
            .rev()
            .map(|(pitch, name)| (*pitch, name.to_string()))
            .collect()
    } else {
        (LOWEST_PITCH..=HIGHEST_PITCH)
            .rev()
            .map(|p| {
                let label = if p % 12 == 0 {
                    format!("C{}", p as i32 / 12 - 1)
                } else {
                    String::new()
                };
                (p, label)
            })
            .collect()
    }
}

fn empty_state(ui: &mut Ui, studio: &mut Studio) {
    let track = studio.project.track(studio.selected());
    let can_draw = track.is_some_and(|t| !t.is_audio());
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(icons::PENCIL)
                .font(theme::icon(16.0))
                .color(ACCENT),
        );
        ui.label(egui::RichText::new("Note editor").font(bold(13.5)));
        theme::vsep(ui, 14.0);
        let hint = if can_draw {
            "Double-click a clip to edit its notes, or start an empty one."
        } else {
            "Voice clips have no notes. Select an instrument or drum clip."
        };
        ui.label(egui::RichText::new(hint).color(TEXT_DIM));
    });
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::hover());
    ui.painter().rect_filled(rect, 8, SURFACE_2);
    if !can_draw {
        return;
    }
    let button = Rect::from_center_size(rect.center(), vec2(220.0, 32.0));
    let clicked = ui
        .scope_builder(egui::UiBuilder::new().max_rect(button), |ui| {
            theme::primary_button(ui, Some(icons::PLUS), "New clip at the playhead").clicked()
        })
        .inner;
    if clicked {
        let track = studio.selected();
        let bar = (studio.position / 4.0).floor() * 4.0;
        if let Some(id) = studio.project.add_empty_clip(track, bar, NEW_CLIP_BEATS) {
            studio.selected_clip = Some(id);
            studio.sync();
        }
    }
}

pub fn show(ui: &mut Ui, studio: &mut Studio, ed: &mut Editor) {
    // Let a clicked note ring briefly, then stop it.
    if let Some((_, since)) = ed.preview {
        if since.elapsed() >= PREVIEW {
            ed.stop_preview(studio);
        } else {
            ui.ctx().request_repaint_after(PREVIEW);
        }
    }

    let found = studio
        .selected_clip
        .and_then(|id| studio.project.clip(id))
        .filter(|(_, clip)| !clip.is_audio())
        .map(|(track, clip)| {
            let index = studio
                .project
                .tracks
                .iter()
                .position(|t| t.id == track.id)
                .unwrap_or(0);
            (clip.id, track.name.clone(), track.is_drums(), index)
        });
    let Some((clip_id, track_name, drums, track_index)) = found else {
        ed.shown = None;
        ed.selected = None;
        empty_state(ui, studio);
        return;
    };
    let color = track_color(track_index);
    let mut changed = false;

    // ---- Header ----
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(icons::PENCIL)
                .font(theme::icon(16.0))
                .color(ACCENT),
        );
        ui.label(egui::RichText::new(format!("{track_name} notes")).font(bold(13.5)));
        theme::vsep(ui, 14.0);
        ui.label(
            egui::RichText::new("GRID")
                .font(font(9.5))
                .color(TEXT_FAINT),
        );
        let current = GRIDS
            .iter()
            .find(|(_, g)| *g == ed.grid)
            .map_or("1/16", |(name, _)| name);
        egui::ComboBox::from_id_salt("editor-grid")
            .width(58.0)
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (name, grid) in GRIDS {
                    ui.selectable_value(&mut ed.grid, grid, name);
                }
            });
        if ui
            .button("Quantize")
            .on_hover_text("Move every note onto the nearest grid line")
            .clicked()
            && let Some(clip) = studio.project.clip_mut(clip_id)
        {
            clip.quantize(ed.grid);
            changed = true;
        }
        if let Some(i) = ed.selected
            && let Some(note) = studio
                .project
                .clip_mut(clip_id)
                .and_then(|c| c.notes.get_mut(i))
        {
            theme::vsep(ui, 14.0);
            ui.label(
                egui::RichText::new("VELOCITY")
                    .font(font(9.5))
                    .color(TEXT_FAINT),
            );
            let mut velocity = note.velocity;
            if ui
                .add(egui::DragValue::new(&mut velocity).range(1..=127))
                .on_hover_text("How hard the note is played")
                .changed()
            {
                note.velocity = velocity;
                changed = true;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(
                    "Click to add · drag to move or stretch · right-click to delete",
                )
                .color(TEXT_DIM),
            );
        });
    });
    ui.add_space(6.0);

    // ---- Grid ----
    let rows = rows(drums);
    let row_h = if drums { DRUM_ROW } else { PIANO_ROW };
    let keys_w = if drums {
        DRUM_KEYS_WIDTH
    } else {
        PIANO_KEYS_WIDTH
    };
    let row_of = |pitch: u8| rows.iter().position(|(p, _)| *p == pitch);
    let position = studio.position;
    let shared = studio.shared.clone();
    let Some(clip) = studio.project.clip_mut(clip_id) else {
        return;
    };
    let length = clip.length_beats;
    let clip_start = clip.start_beat;

    // Drum grids are short; a piano roll gets the full height.
    let body_height = BODY_HEIGHT.min(rows.len() as f32 * row_h);
    ui.set_min_height(body_height);
    let mut scroll = egui::ScrollArea::both()
        .max_height(body_height)
        .min_scrolled_height(body_height)
        .auto_shrink([false, false]);
    if ed.shown != Some(clip_id) {
        // First look at this clip: put its notes (or middle C) in the middle.
        let (low, high) = clip.notes.iter().fold((127u8, 0u8), |(lo, hi), n| {
            (lo.min(n.pitch), hi.max(n.pitch))
        });
        let middle = if clip.notes.is_empty() {
            60
        } else {
            ((low as u16 + high as u16) / 2) as u8
        };
        let row = rows
            .iter()
            .position(|(p, _)| *p <= middle)
            .unwrap_or(rows.len() / 2);
        let y = (row as f32 + 0.5) * row_h - body_height / 2.0;
        scroll = scroll.vertical_scroll_offset(y.max(0.0));
    }
    scroll.show(ui, |ui| {
        let total = vec2(
            (keys_w + length as f32 * PX_PER_BEAT + 40.0).max(ui.available_width()),
            rows.len() as f32 * row_h,
        );
        let (area, resp) = ui.allocate_exact_size(total, Sense::click_and_drag());
        let visible = ui.clip_rect();
        let origin = pos2(area.left() + keys_w, area.top());
        let keys_right = visible.left() + keys_w;
        let beat_at = |x: f32| ((x - origin.x) / PX_PER_BEAT) as f64;
        let row_at =
            |y: f32| (((y - area.top()) / row_h).floor().max(0.0) as usize).min(rows.len() - 1);
        let note_rect = |n: &Note, row: usize| {
            Rect::from_min_size(
                pos2(
                    origin.x + n.start_beat as f32 * PX_PER_BEAT,
                    area.top() + row as f32 * row_h,
                ),
                vec2((n.length_beats as f32 * PX_PER_BEAT).max(4.0), row_h),
            )
        };

        if ed.shown != Some(clip_id) {
            ed.shown = Some(clip_id);
            ed.selected = None;
            ed.drag = None;
        }

        let hit = |p: Pos2, notes: &[Note]| -> Option<(usize, bool)> {
            notes.iter().enumerate().rev().find_map(|(i, n)| {
                let r = note_rect(n, row_of(n.pitch)?);
                r.contains(p)
                    .then(|| (i, !drums && p.x > r.right() - RESIZE_GRIP))
            })
        };
        let new_note = |p: Pos2, grid: f64, len: f64| -> Option<Note> {
            let start = (beat_at(p.x) / grid).floor() * grid;
            if start < 0.0 || start >= length {
                return None;
            }
            Some(Note {
                pitch: rows[row_at(p.y)].0,
                velocity: NEW_VELOCITY,
                start_beat: start,
                length_beats: if drums { grid } else { len }.min(length - start),
            })
        };

        let press = ui.input(|i| i.pointer.press_origin());
        let pointer = resp.interact_pointer_pos();
        let on_keys = |p: Pos2| p.x < keys_right;

        // Keys column: hold to hear a note.
        let want = pointer
            .filter(|p| resp.is_pointer_button_down_on() && on_keys(*p) && ed.drag.is_none())
            .filter(|_| press.is_some_and(on_keys))
            .map(|p| rows[row_at(p.y)].0);
        if want != ed.key_held {
            if let Some(old) = ed.key_held.take() {
                shared.note_off(old);
            }
            if let Some(pitch) = want {
                shared.note_on(pitch, NEW_VELOCITY);
                ed.key_held = Some(pitch);
            }
        }

        let mut sound = None;
        if resp.secondary_clicked()
            && let Some(p) = pointer.filter(|p| !on_keys(*p))
            && let Some((i, _)) = hit(p, &clip.notes)
        {
            clip.notes.remove(i);
            ed.selected = None;
            changed = true;
        } else if resp.clicked()
            && let Some(p) = pointer.filter(|p| !on_keys(*p))
        {
            if let Some((i, _)) = hit(p, &clip.notes) {
                ed.selected = Some(i);
                sound = Some(clip.notes[i].pitch);
            } else if let Some(n) = new_note(p, ed.grid, ed.note_length) {
                sound = Some(n.pitch);
                clip.notes.push(n);
                ed.selected = Some(clip.notes.len() - 1);
                changed = true;
            }
        } else if resp.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = press.filter(|p| !on_keys(*p))
        {
            if let Some((i, resize)) = hit(p, &clip.notes) {
                let grab = beat_at(p.x) - clip.notes[i].start_beat;
                ed.drag = Some(Drag {
                    note: i,
                    resize,
                    grab,
                });
                ed.selected = Some(i);
                sound = Some(clip.notes[i].pitch);
            } else if let Some(n) = new_note(p, ed.grid, ed.grid) {
                // Drawing: press where the note starts, drag to set its length.
                sound = Some(n.pitch);
                clip.notes.push(n);
                let i = clip.notes.len() - 1;
                ed.selected = Some(i);
                ed.drag = Some(Drag {
                    note: i,
                    resize: !drums,
                    grab: 0.0,
                });
                changed = true;
            }
        }

        if resp.dragged()
            && let Some(drag) = &ed.drag
            && let Some(p) = pointer
            && let Some(n) = clip.notes.get_mut(drag.note)
        {
            let before = *n;
            if drag.resize {
                let end = snap(beat_at(p.x), ed.grid).min(length);
                n.length_beats = (end - n.start_beat).max(ed.grid.max(MIN_LENGTH));
            } else {
                let last = (length - n.length_beats).max(0.0);
                n.start_beat = snap(beat_at(p.x) - drag.grab, ed.grid).clamp(0.0, last);
                n.pitch = rows[row_at(p.y)].0;
                if n.pitch != before.pitch {
                    sound = Some(n.pitch);
                }
            }
            changed |= *n != before;
        }
        if resp.drag_stopped()
            && let Some(drag) = ed.drag.take()
            && drag.resize
            && let Some(n) = clip.notes.get(drag.note)
        {
            ed.note_length = n.length_beats;
        }
        if let Some(pitch) = sound {
            // Borrow dance: `studio` is busy lending the clip, so go through the
            // shared half directly.
            if let Some((old, _)) = ed.preview.take() {
                shared.note_off(old);
            }
            shared.note_on(pitch, NEW_VELOCITY);
            ed.preview = Some((pitch, Instant::now()));
            ui.ctx().request_repaint_after(PREVIEW);
        }

        // ---- Paint ----
        let painter = ui.painter_at(visible.intersect(area));
        painter.rect_filled(area, 0, BG);
        for (row, (pitch, _)) in rows.iter().enumerate() {
            let y = area.top() + row as f32 * row_h;
            if y > visible.bottom() || y + row_h < visible.top() {
                continue;
            }
            let r = Rect::from_min_size(pos2(visible.left(), y), vec2(visible.width(), row_h));
            if drums {
                if row % 2 == 0 {
                    painter.rect_filled(r, 0, Color32::from_white_alpha(4));
                }
            } else if !is_black(*pitch) {
                painter.rect_filled(r, 0, Color32::from_white_alpha(5));
            }
            if drums || pitch % 12 == 0 {
                painter.line_segment(
                    [r.left_bottom(), r.right_bottom()],
                    Stroke::new(1.0, Color32::from_white_alpha(10)),
                );
            }
        }
        // Beyond the end of the clip.
        let end_x = origin.x + length as f32 * PX_PER_BEAT;
        painter.rect_filled(
            Rect::from_min_max(pos2(end_x, area.top()), area.max),
            0,
            Color32::from_black_alpha(110),
        );
        let steps = (length / ed.grid).ceil() as usize;
        for step in 0..=steps {
            let beat = step as f64 * ed.grid;
            let x = origin.x + beat as f32 * PX_PER_BEAT;
            if x < visible.left() || x > visible.right() {
                continue;
            }
            let song_beat = clip_start + beat;
            let on = |every: f64| (song_beat / every - (song_beat / every).round()).abs() < 1e-6;
            let alpha = if on(4.0) {
                38
            } else if on(1.0) {
                18
            } else {
                7
            };
            painter.line_segment(
                [pos2(x, area.top()), pos2(x, area.bottom())],
                Stroke::new(1.0, Color32::from_white_alpha(alpha)),
            );
        }
        for (i, n) in clip.notes.iter().enumerate() {
            let Some(row) = row_of(n.pitch) else {
                continue;
            };
            let r = note_rect(n, row).shrink2(vec2(0.5, 1.0));
            let strength = 0.55 + 0.45 * n.velocity as f32 / 127.0;
            painter.rect_filled(r, 2, color.gamma_multiply(strength));
            if ed.selected == Some(i) {
                painter.rect_stroke(r, 2, Stroke::new(1.5, TEXT), StrokeKind::Outside);
            }
        }
        let playhead = origin.x + (position - clip_start) as f32 * PX_PER_BEAT;
        if position >= clip_start && position <= clip_start + length {
            painter.line_segment(
                [pos2(playhead, area.top()), pos2(playhead, area.bottom())],
                Stroke::new(1.5, TEXT),
            );
        }

        // Keys column, pinned to the left edge over the scrolling grid.
        let keys = Rect::from_min_max(
            pos2(visible.left(), area.top()),
            pos2(keys_right, area.bottom()),
        );
        painter.rect_filled(keys, 0, theme::SURFACE);
        for (row, (pitch, label)) in rows.iter().enumerate() {
            let y = area.top() + row as f32 * row_h;
            if y > visible.bottom() || y + row_h < visible.top() {
                continue;
            }
            let r = Rect::from_min_size(pos2(keys.left(), y), vec2(keys_w - 1.0, row_h - 1.0));
            let lit = shared.is_held(*pitch);
            let fill = if lit {
                ACCENT
            } else if drums {
                SURFACE_2
            } else if is_black(*pitch) {
                Color32::from_rgb(0x1a, 0x1b, 0x1f)
            } else {
                Color32::from_rgb(0xd6, 0xd8, 0xdd)
            };
            painter.rect_filled(r, 2, fill);
            if !label.is_empty() {
                let text = if drums || lit {
                    TEXT
                } else {
                    Color32::from_gray(70)
                };
                painter.text(
                    if drums {
                        r.left_center() + vec2(8.0, 0.0)
                    } else {
                        r.right_center() - vec2(22.0, 0.0)
                    },
                    Align2::LEFT_CENTER,
                    label,
                    font(if drums { 11.0 } else { 9.0 }),
                    text,
                );
            }
        }
    });

    if changed {
        studio.sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piano_rows_run_from_high_to_low_with_c_labels() {
        let r = rows(false);
        assert_eq!(r.first().unwrap().0, HIGHEST_PITCH);
        assert_eq!(r.last().unwrap().0, LOWEST_PITCH);
        assert_eq!(r.iter().find(|(p, _)| *p == 60).unwrap().1, "C4");
        assert_eq!(r.iter().find(|(p, _)| *p == 61).unwrap().1, "");
    }

    #[test]
    fn drum_rows_are_the_pads_with_the_kick_at_the_bottom() {
        let r = rows(true);
        assert_eq!(r.len(), PADS.len());
        assert_eq!(r.last().unwrap(), &(36, "Kick".to_string()));
    }

    #[test]
    fn deleting_the_picked_note_removes_it_from_the_clip() {
        let (ui, _engine) = crate::engine::commands::link();
        let mut studio = Studio::new(Studio::starter_project(), ui);
        let track = studio.selected();
        let clip = studio.project.add_empty_clip(track, 0.0, 4.0).unwrap();
        let note = |pitch| Note {
            pitch,
            velocity: 100,
            start_beat: 0.0,
            length_beats: 1.0,
        };
        studio.project.clip_mut(clip).unwrap().notes = vec![note(60), note(64)];
        studio.selected_clip = Some(clip);
        let mut ed = Editor {
            selected: Some(0),
            ..Editor::default()
        };
        assert!(ed.delete_selected(&mut studio));
        assert_eq!(studio.project.clip(clip).unwrap().1.notes, vec![note(64)]);
        assert!(!ed.delete_selected(&mut studio));
    }
}
