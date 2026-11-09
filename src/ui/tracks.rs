//! The track rows: header (name, mute/solo/volume on hover) and clip lane.

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use super::studio::Studio;
use super::{ACCENT, CLIP, ROW, ROW_SELECTED};
use crate::project::{Track, TrackKind};

const ROW_HEIGHT: f32 = 56.0;
const HEADER_WIDTH: f32 = 200.0;
const RULER_HEIGHT: f32 = 22.0;
const PX_PER_BEAT: f32 = 28.0;
const MIN_BARS: f64 = 32.0;

fn icon(kind: &TrackKind) -> &'static str {
    match kind {
        TrackKind::Instrument { .. } => "🎹",
        TrackKind::Audio => "🎤",
        TrackKind::Drums => "🥁",
    }
}

pub fn show(ui: &mut egui::Ui, studio: &mut Studio) {
    let end_beat = studio
        .project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .map(|c| c.start_beat + c.length_beats)
        .fold(MIN_BARS * 4.0, f64::max)
        .max(studio.position + 16.0);
    let lane_width = (end_beat as f32) * PX_PER_BEAT;

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let total = vec2(
                HEADER_WIDTH + lane_width,
                RULER_HEIGHT + ROW_HEIGHT * studio.project.tracks.len().max(1) as f32,
            );
            let (area, _) = ui.allocate_exact_size(total, Sense::hover());
            let lane_x = area.left() + HEADER_WIDTH;

            ruler(
                ui,
                studio,
                Rect::from_min_size(pos2(lane_x, area.top()), vec2(lane_width, RULER_HEIGHT)),
            );

            let selected = studio.selected();
            let mut changed = false;
            let mut select = None;
            for (i, track) in studio.project.tracks.iter_mut().enumerate() {
                let top = area.top() + RULER_HEIGHT + i as f32 * ROW_HEIGHT;
                let row = Rect::from_min_size(pos2(area.left(), top), vec2(total.x, ROW_HEIGHT));
                let is_sel = track.id == selected;
                let resp = ui.interact(row, ui.scope_id().with(("row", track.id)), Sense::click());
                if resp.clicked() {
                    select = Some(track.id);
                }
                ui.painter()
                    .rect_filled(row, 0.0, if is_sel { ROW_SELECTED } else { ROW });
                ui.painter().line_segment(
                    [row.left_bottom(), row.right_bottom()],
                    Stroke::new(1.0, Color32::from_gray(35)),
                );
                let header = Rect::from_min_size(row.min, vec2(HEADER_WIDTH, ROW_HEIGHT));
                changed |= track_header(ui, track, header, resp.hovered() || is_sel);
                draw_clips(ui, track, lane_x, row);
            }
            if let Some(id) = select {
                studio.select(id);
            }
            if changed {
                studio.sync();
            }

            // Playhead
            let x = lane_x + studio.position as f32 * PX_PER_BEAT;
            ui.painter().line_segment(
                [pos2(x, area.top()), pos2(x, area.bottom())],
                Stroke::new(2.0, Color32::WHITE),
            );
            if studio.playing {
                ui.scroll_to_rect(
                    Rect::from_center_size(pos2(x, area.top()), vec2(200.0, 1.0)),
                    None,
                );
            }
        });
}

fn ruler(ui: &mut egui::Ui, studio: &mut Studio, rect: Rect) {
    let resp = ui.interact(rect, ui.scope_id().with("ruler"), Sense::click_and_drag());
    if (resp.clicked() || resp.dragged())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let beat = ((p.x - rect.left()) / PX_PER_BEAT).max(0.0) as f64;
        studio.locate(beat.floor());
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(28));
    let beats = (rect.width() / PX_PER_BEAT) as usize;
    for b in 0..=beats {
        let x = rect.left() + b as f32 * PX_PER_BEAT;
        let bar = b % 4 == 0;
        let h = if bar {
            rect.height()
        } else {
            rect.height() * 0.35
        };
        painter.line_segment(
            [pos2(x, rect.bottom() - h), pos2(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_gray(if bar { 90 } else { 55 })),
        );
        if bar {
            painter.text(
                pos2(x + 4.0, rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                b / 4 + 1,
                egui::FontId::proportional(11.0),
                Color32::from_gray(150),
            );
        }
    }
}

/// Returns true when the track's mix settings changed.
fn track_header(ui: &mut egui::Ui, track: &mut Track, rect: Rect, show_controls: bool) -> bool {
    let mut changed = false;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(10.0, 6.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.label(RichText::new(format!("{}  {}", icon(&track.kind), track.name)).size(15.0));
    if show_controls {
        child.horizontal(|ui| {
            let toggle =
                |ui: &mut egui::Ui, on: &mut bool, label: &str, color: Color32, tip: &str| {
                    let b = egui::Button::new(RichText::new(label).strong())
                        .fill(if *on {
                            color
                        } else {
                            ui.visuals().widgets.inactive.weak_bg_fill
                        })
                        .min_size(vec2(22.0, 18.0));
                    if ui.add(b).on_hover_text(tip).clicked() {
                        *on = !*on;
                        return true;
                    }
                    false
                };
            changed |= toggle(
                ui,
                &mut track.mute,
                "M",
                Color32::from_rgb(0xe0, 0x95, 0x4b),
                "Mute",
            );
            changed |= toggle(ui, &mut track.solo, "S", ACCENT, "Solo");
            ui.spacing_mut().slider_width = 90.0;
            changed |= ui
                .add(egui::Slider::new(&mut track.volume, 0.0..=1.5).show_value(false))
                .on_hover_text("Volume")
                .changed();
        });
    }
    changed
}

fn draw_clips(ui: &egui::Ui, track: &Track, lane_x: f32, row: Rect) {
    let painter = ui.painter_at(Rect::from_min_max(pos2(lane_x, row.top()), row.max));
    for clip in &track.clips {
        let x0 = lane_x + clip.start_beat as f32 * PX_PER_BEAT;
        let r = Rect::from_min_size(
            pos2(x0, row.top() + 5.0),
            vec2(clip.length_beats as f32 * PX_PER_BEAT, ROW_HEIGHT - 10.0),
        );
        painter.rect_filled(r, 5.0, CLIP.gamma_multiply(0.85));
        painter.rect_stroke(r, 5.0, Stroke::new(1.0, CLIP), StrokeKind::Inside);
        // Mini piano-roll preview of the notes.
        let (lo, hi) = clip.notes.iter().fold((127u8, 0u8), |(lo, hi), n| {
            (lo.min(n.pitch), hi.max(n.pitch))
        });
        let span = (hi.saturating_sub(lo)).max(12) as f32;
        let inner = r.shrink(5.0);
        for n in &clip.notes {
            let y = inner.bottom() - (n.pitch.saturating_sub(lo)) as f32 / span * inner.height();
            let a = Pos2::new(inner.left() + n.start_beat as f32 * PX_PER_BEAT, y);
            let b = Pos2::new(a.x + (n.length_beats as f32 * PX_PER_BEAT).max(2.0), y);
            painter.line_segment(
                [a, b],
                Stroke::new(2.5, Color32::from_rgb(0x1d, 0x4d, 0x33)),
            );
        }
    }
}
