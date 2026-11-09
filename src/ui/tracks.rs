//! The arrangement: ruler, track headers and clip lanes.

use egui::{Align2, Color32, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

use super::icons;
use std::collections::HashMap;
use std::sync::Arc;

use super::studio::{Studio, Waveform};
use super::theme::{
    self, ACCENT, BG, BORDER, MUTE, RECORD, SURFACE, SURFACE_2, TEXT, TEXT_DIM, TEXT_FAINT, bold,
    font, track_color,
};
use crate::project::{Track, TrackKind};

const ROW_HEIGHT: f32 = 64.0;
const HEADER_WIDTH: f32 = 230.0;
const RULER_HEIGHT: f32 = 28.0;
const PX_PER_BEAT: f32 = 30.0;
const MIN_BARS: f64 = 32.0;
const MONITOR: Color32 = Color32::from_rgb(0x34, 0xc3, 0x8f);

fn kind_icon(kind: &TrackKind) -> &'static str {
    match kind {
        TrackKind::Instrument { .. } => icons::PIANO,
        TrackKind::Audio => icons::MIC,
        TrackKind::Drums { .. } => icons::DRUM,
    }
}

/// Subtitle under the track name: the sound it plays, or "Voice".
fn kind_label(track: &Track) -> String {
    match track.sound() {
        Some(s) => crate::project::display_name(s),
        None => "Voice".into(),
    }
}

pub fn show(ui: &mut Ui, studio: &mut Studio, browser: &mut super::sound_browser::Browser) {
    if studio.project.tracks.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new("Click “Add Track” to start making music")
                    .font(font(15.0))
                    .color(TEXT_DIM),
            );
        });
        return;
    }
    let end_beat = studio
        .project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .map(|c| c.start_beat + c.length_beats)
        .fold(MIN_BARS * 4.0, f64::max)
        .max(studio.position + 16.0);
    let lane_width = (end_beat as f32) * PX_PER_BEAT;
    let rows = studio.project.tracks.len() as f32;

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let visible_h = ui.available_height();
            let total = vec2(
                HEADER_WIDTH + lane_width,
                (RULER_HEIGHT + ROW_HEIGHT * rows).max(visible_h),
            );
            let (area, _) = ui.allocate_exact_size(total, Sense::hover());
            let lane_x = area.left() + HEADER_WIDTH;
            let lanes = Rect::from_min_max(pos2(lane_x, area.top() + RULER_HEIGHT), area.max);

            grid(ui, lanes);
            ruler(
                ui,
                studio,
                Rect::from_min_size(pos2(lane_x, area.top()), vec2(lane_width, RULER_HEIGHT)),
            );
            // Header column background + ruler corner.
            ui.painter().rect_filled(
                Rect::from_min_size(area.min, vec2(HEADER_WIDTH, total.y)),
                0,
                SURFACE,
            );
            ui.painter().line_segment(
                [pos2(lane_x, area.top()), pos2(lane_x, area.bottom())],
                Stroke::new(1.0, BORDER),
            );

            let selected = studio.selected();
            let tempo = studio.project.tempo_bpm;
            let mut changed = false;
            let mut select = None;
            let mut pick_sound = None;
            for (i, track) in studio.project.tracks.iter_mut().enumerate() {
                let top = area.top() + RULER_HEIGHT + i as f32 * ROW_HEIGHT;
                let row = Rect::from_min_size(pos2(area.left(), top), vec2(total.x, ROW_HEIGHT));
                let color = track_color(i);
                let is_sel = track.id == selected;
                let resp = ui.interact(row, ui.scope_id().with(("row", track.id)), Sense::click());
                if resp.clicked() {
                    select = Some(track.id);
                }
                if is_sel {
                    ui.painter().rect_filled(
                        Rect::from_min_max(pos2(lane_x, row.top()), row.max),
                        0,
                        Color32::from_white_alpha(6),
                    );
                }
                ui.painter().line_segment(
                    [row.left_bottom(), row.right_bottom()],
                    Stroke::new(1.0, BORDER),
                );
                let header = Rect::from_min_size(row.min, vec2(HEADER_WIDTH, ROW_HEIGHT));
                changed |= track_header(ui, track, header, color, is_sel, &mut pick_sound);
                draw_clips(ui, track, lane_x, row, color, &studio.waveforms, tempo);
                if let Some((rec_track, from)) = studio.recording_from
                    && rec_track == track.id
                    && studio.recording
                {
                    let x0 = lane_x + from as f32 * PX_PER_BEAT;
                    let x1 = lane_x + (studio.position.max(from)) as f32 * PX_PER_BEAT;
                    let r = Rect::from_min_max(
                        pos2(x0 + 1.0, row.top() + 6.0),
                        pos2(x1.max(x0 + 3.0), row.bottom() - 6.0),
                    );
                    let p = ui.painter_at(Rect::from_min_max(pos2(lane_x, row.top()), row.max));
                    p.rect_filled(r, 6, RECORD.gamma_multiply(0.28));
                    p.rect_stroke(r, 6, Stroke::new(1.0, RECORD), StrokeKind::Inside);
                }
            }
            if let Some(id) = select {
                studio.select(id);
            }
            if let Some((id, drums)) = pick_sound {
                studio.select(id);
                browser.open_for_track(id, drums);
            }
            if changed {
                studio.sync();
            }

            // Playhead
            let x = lane_x + studio.position as f32 * PX_PER_BEAT;
            ui.painter().line_segment(
                [pos2(x, area.top() + RULER_HEIGHT), pos2(x, area.bottom())],
                Stroke::new(1.5, TEXT),
            );
            let t = area.top() + RULER_HEIGHT;
            ui.painter().add(Shape::convex_polygon(
                vec![pos2(x - 6.0, t - 10.0), pos2(x + 6.0, t - 10.0), pos2(x, t)],
                TEXT,
                Stroke::NONE,
            ));
            if studio.playing {
                ui.scroll_to_rect(
                    Rect::from_center_size(pos2(x, area.top()), vec2(240.0, 1.0)),
                    None,
                );
            }
        });
}

fn grid(ui: &Ui, lanes: Rect) {
    let p = ui.painter_at(lanes);
    p.rect_filled(lanes, 0, BG);
    let beats = (lanes.width() / PX_PER_BEAT) as usize;
    for b in 0..=beats {
        let x = lanes.left() + b as f32 * PX_PER_BEAT;
        if b % 8 == 4 {
            // Shade every other bar very slightly.
            p.rect_filled(
                Rect::from_min_size(
                    pos2(x, lanes.top()),
                    vec2(PX_PER_BEAT * 4.0, lanes.height()),
                ),
                0,
                Color32::from_white_alpha(3),
            );
        }
        let alpha = if b % 4 == 0 { 14 } else { 5 };
        p.line_segment(
            [pos2(x, lanes.top()), pos2(x, lanes.bottom())],
            Stroke::new(1.0, Color32::from_white_alpha(alpha)),
        );
    }
}

fn ruler(ui: &mut Ui, studio: &mut Studio, rect: Rect) {
    let resp = ui.interact(rect, ui.scope_id().with("ruler"), Sense::click_and_drag());
    if (resp.clicked() || resp.dragged())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let beat = ((p.x - rect.left()) / PX_PER_BEAT).max(0.0) as f64;
        studio.locate(beat.floor());
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0, SURFACE);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, BORDER),
    );
    let beats = (rect.width() / PX_PER_BEAT) as usize;
    for b in 0..=beats {
        let x = rect.left() + b as f32 * PX_PER_BEAT;
        let bar = b % 4 == 0;
        let h = if bar { 9.0 } else { 4.0 };
        painter.line_segment(
            [pos2(x, rect.bottom() - h), pos2(x, rect.bottom())],
            Stroke::new(1.0, if bar { TEXT_FAINT } else { BORDER }),
        );
        if bar {
            painter.text(
                pos2(x + 5.0, rect.top() + 9.0),
                Align2::LEFT_CENTER,
                b / 4 + 1,
                font(11.0),
                TEXT_DIM,
            );
        }
    }
}

/// Returns true when the track's mix settings changed.
fn track_header(
    ui: &mut Ui,
    track: &mut Track,
    rect: Rect,
    color: Color32,
    selected: bool,
    pick_sound: &mut Option<(u64, bool)>,
) -> bool {
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 0, SURFACE_2);
    }
    // Color tab
    p.rect_filled(
        Rect::from_min_size(rect.min, vec2(4.0, rect.height())),
        0,
        if selected {
            color
        } else {
            color.gamma_multiply(0.6)
        },
    );
    // Icon tile: click to change the sound.
    let tile = Rect::from_min_size(rect.min + vec2(16.0, 14.0), vec2(36.0, 36.0));
    let tile_resp = ui.interact(tile, ui.scope_id().with(("tile", track.id)), Sense::click());
    if track.sound().is_some()
        && tile_resp
            .on_hover_text("Change sound")
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    {
        *pick_sound = Some((track.id, track.is_drums()));
    }
    let p = ui.painter();
    p.rect_filled(tile, 8, color.gamma_multiply(0.18));
    p.text(
        tile.center(),
        Align2::CENTER_CENTER,
        kind_icon(&track.kind),
        theme::icon(17.0),
        color,
    );
    // Name + subtitle
    let x = tile.right() + 12.0;
    p.text(
        pos2(x, rect.top() + 22.0),
        Align2::LEFT_CENTER,
        &track.name,
        bold(13.5),
        TEXT,
    );
    p.text(
        pos2(x, rect.top() + 41.0),
        Align2::LEFT_CENTER,
        kind_label(track),
        font(11.5),
        TEXT_DIM,
    );

    // Mute / solo / volume on the right.
    let mut changed = false;
    let voice = track.is_audio();
    let controls = Rect::from_min_max(
        pos2(
            rect.right() - if voice { 104.0 } else { 76.0 },
            rect.top() + 11.0,
        ),
        pos2(rect.right() - 12.0, rect.bottom() - 10.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(controls), |ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.horizontal(|ui| {
            if voice {
                changed |= theme::icon_chip(ui, icons::HEADPHONES, &mut track.monitor, MONITOR)
                    .on_hover_text("Hear yourself while this track is selected (use headphones)")
                    .clicked();
            }
            changed |= theme::toggle_chip(ui, "M", &mut track.mute, MUTE)
                .on_hover_text("Mute")
                .clicked();
            changed |= theme::toggle_chip(ui, "S", &mut track.solo, ACCENT)
                .on_hover_text("Solo")
                .clicked();
        });
        ui.spacing_mut().slider_width = if voice { 92.0 } else { 64.0 };
        ui.spacing_mut().interact_size.y = 14.0;
        changed |= ui
            .add(egui::Slider::new(&mut track.volume, 0.0..=1.5).show_value(false))
            .on_hover_text(format!("Volume {:.0}%", track.volume * 100.0))
            .changed();
    });
    changed
}

fn draw_clips(
    ui: &Ui,
    track: &Track,
    lane_x: f32,
    row: Rect,
    color: Color32,
    waveforms: &HashMap<u64, Arc<Waveform>>,
    tempo: f64,
) {
    let painter = ui.painter_at(Rect::from_min_max(pos2(lane_x, row.top()), row.max));
    let dim = if track.mute { 0.35 } else { 1.0 };
    for clip in &track.clips {
        let x0 = lane_x + clip.start_beat as f32 * PX_PER_BEAT;
        let r = Rect::from_min_size(
            pos2(x0 + 1.0, row.top() + 6.0),
            vec2(
                clip.length_beats as f32 * PX_PER_BEAT - 2.0,
                ROW_HEIGHT - 12.0,
            ),
        );
        painter.rect_filled(r, 6, color.gamma_multiply(0.22 * dim));
        painter.rect_stroke(
            r,
            6,
            Stroke::new(1.0, color.gamma_multiply(0.7 * dim)),
            StrokeKind::Inside,
        );
        if clip.is_audio() {
            if let Some(wave) = waveforms.get(&clip.id) {
                draw_waveform(&painter, wave, r, tempo, color.gamma_multiply(dim));
            }
            continue;
        }
        // Note preview.
        let (lo, hi) = clip.notes.iter().fold((127u8, 0u8), |(lo, hi), n| {
            (lo.min(n.pitch), hi.max(n.pitch))
        });
        let span = (hi.saturating_sub(lo)).max(12) as f32;
        let inner = r.shrink2(vec2(4.0, 8.0));
        for n in &clip.notes {
            let y = inner.bottom() - (n.pitch.saturating_sub(lo)) as f32 / span * inner.height();
            let x = inner.left() + n.start_beat as f32 * PX_PER_BEAT;
            let w = (n.length_beats as f32 * PX_PER_BEAT).max(3.0);
            painter.rect_filled(
                Rect::from_min_size(pos2(x, y - 1.5), vec2(w, 3.0)),
                1,
                color.gamma_multiply(dim),
            );
        }
    }
}

/// One vertical line per pixel, as tall as the loudest moment it covers.
fn draw_waveform(painter: &egui::Painter, wave: &Waveform, r: Rect, tempo: f64, color: Color32) {
    let visible = r.intersect(painter.clip_rect());
    if visible.width() <= 0.0 {
        return;
    }
    let seconds_per_px = 60.0 / tempo.max(1.0) / PX_PER_BEAT as f64;
    let mid = r.center().y;
    let half = r.height() / 2.0 - 5.0;
    let mut x = visible.left().floor();
    while x < visible.right() {
        let t = (x - r.left()).max(0.0) as f64 * seconds_per_px;
        let h = (wave.peak_between(t, t + seconds_per_px) * half).max(0.5);
        painter.line_segment(
            [pos2(x, mid - h), pos2(x, mid + h)],
            Stroke::new(1.0, color),
        );
        x += 1.0;
    }
}
