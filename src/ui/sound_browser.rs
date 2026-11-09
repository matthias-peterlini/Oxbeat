//! The "New Track" / "Choose a Sound" dialog.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::icons;
use super::studio::Studio;
use super::theme::{self, ACCENT, SURFACE_2, SURFACE_3, TEXT, TEXT_DIM, bold, font};
use crate::project::{TrackKind, display_name};
use crate::sounds::{BUILTIN_INSTRUMENTS, BUILTIN_KITS};

#[derive(Default, Clone, Copy, PartialEq)]
enum Page {
    #[default]
    TrackType,
    Instruments,
    Kits,
}

/// What picking a sound does.
#[derive(Default, Clone, Copy, PartialEq)]
enum Target {
    #[default]
    NewTrack,
    /// Change the sound of an existing track.
    Track(u64),
}

#[derive(Default)]
pub struct Browser {
    pub open: bool,
    page: Page,
    target: Target,
}

impl Browser {
    pub fn open_new_track(&mut self) {
        *self = Browser {
            open: true,
            ..Default::default()
        };
    }

    /// Open the sound list for an existing track.
    pub fn open_for_track(&mut self, track: u64, drums: bool) {
        *self = Browser {
            open: true,
            page: if drums { Page::Kits } else { Page::Instruments },
            target: Target::Track(track),
        };
    }

    fn pick(&mut self, studio: &mut Studio, sound: &str) {
        match self.target {
            Target::NewTrack => {
                let id = if self.page == Page::Kits {
                    studio.project.add_drums_track(sound)
                } else {
                    studio.project.add_instrument_track(sound)
                };
                studio.sync();
                studio.select(id);
            }
            Target::Track(id) => {
                if let Some(t) = studio.project.track_mut(id) {
                    match &mut t.kind {
                        TrackKind::Instrument { preset } => {
                            // Keep a name the user typed; follow the sound otherwise.
                            if t.name == display_name(preset) {
                                t.name = display_name(sound);
                            }
                            *preset = sound.to_string();
                        }
                        TrackKind::Drums { kit } => *kit = sound.to_string(),
                        TrackKind::Audio => {}
                    }
                }
                studio.sync();
            }
        }
        self.open = false;
    }
}

/// A selectable card with a large icon, title and caption.
fn type_card(ui: &mut Ui, icon: &str, title: &str, caption: &str, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(146.0, 132.0), Sense::click());
    let hovered = enabled && resp.hovered();
    let p = ui.painter();
    p.rect_filled(rect, 10, if hovered { SURFACE_3 } else { SURFACE_2 });
    if hovered {
        p.rect_stroke(rect, 10, Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
    }
    let dim = if enabled { 1.0 } else { 0.4 };
    p.text(
        rect.center_top() + vec2(0.0, 44.0),
        Align2::CENTER_CENTER,
        icon,
        theme::icon(30.0),
        TEXT.gamma_multiply(dim),
    );
    p.text(
        rect.center_top() + vec2(0.0, 84.0),
        Align2::CENTER_CENTER,
        title,
        bold(14.0),
        TEXT.gamma_multiply(dim),
    );
    p.text(
        rect.center_top() + vec2(0.0, 105.0),
        Align2::CENTER_CENTER,
        caption,
        font(11.5),
        TEXT_DIM.gamma_multiply(dim),
    );
    enabled && resp.clicked()
}

/// One row in a sound list. Returns true when clicked.
pub fn sound_row(ui: &mut Ui, icon: &str, title: &str, caption: &str, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 8, SURFACE_3);
    } else if resp.hovered() {
        p.rect_filled(rect, 8, SURFACE_2);
    }
    let icon_rect = Rect::from_min_size(rect.min + vec2(10.0, 9.0), vec2(36.0, 36.0));
    p.rect_filled(icon_rect, 8, SURFACE_3);
    p.text(
        icon_rect.center(),
        Align2::CENTER_CENTER,
        icon,
        theme::icon(17.0),
        ACCENT,
    );
    let x = icon_rect.right() + 14.0;
    p.text(
        pos2(x, rect.top() + 18.0),
        Align2::LEFT_CENTER,
        title,
        bold(14.0),
        TEXT,
    );
    p.text(
        pos2(x, rect.top() + 36.0),
        Align2::LEFT_CENTER,
        caption,
        font(12.0),
        TEXT_DIM,
    );
    resp.clicked()
}

pub fn show(ctx: &egui::Context, studio: &mut Studio, b: &mut Browser) {
    if !b.open {
        return;
    }
    let current = match b.target {
        Target::Track(id) => studio
            .project
            .track(id)
            .and_then(|t| t.sound())
            .map(str::to_string),
        Target::NewTrack => None,
    };
    let r = egui::Modal::new(egui::Id::unique("sound-browser"))
        .frame(
            egui::Frame::window(&ctx.global_style())
                .inner_margin(24)
                .corner_radius(12),
        )
        .show(ctx, |ui| {
            ui.set_width(462.0);
            ui.horizontal(|ui| {
                if b.page != Page::TrackType
                    && b.target == Target::NewTrack
                    && theme::icon_button(ui, icons::CHEVRON_LEFT, 28.0, false)
                        .on_hover_text("Back")
                        .clicked()
                {
                    b.page = Page::TrackType;
                }
                let title = match b.page {
                    Page::TrackType => "New Track",
                    Page::Instruments => "Choose a Sound",
                    Page::Kits => "Choose a Drum Kit",
                };
                ui.label(egui::RichText::new(title).font(bold(18.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::icon_button(ui, icons::X, 28.0, false).clicked() {
                        b.open = false;
                    }
                });
            });
            ui.add_space(16.0);

            match b.page {
                Page::TrackType => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        if type_card(ui, icons::PIANO, "Instrument", "Play with a keyboard", true) {
                            b.page = Page::Instruments;
                        }
                        type_card(ui, icons::MIC, "Voice", "Coming soon", false);
                        if type_card(ui, icons::DRUM, "Drums", "Beats and grooves", true) {
                            b.page = Page::Kits;
                        }
                    });
                }
                Page::Instruments => {
                    for (id, caption) in BUILTIN_INSTRUMENTS {
                        let sel = current.as_deref() == Some(id);
                        if sound_row(ui, icons::AUDIO_LINES, id, caption, sel) {
                            b.pick(studio, id);
                        }
                    }
                }
                Page::Kits => {
                    for (id, caption) in BUILTIN_KITS {
                        let sel = current.as_deref() == Some(id);
                        if sound_row(ui, icons::DRUM, id, caption, sel) {
                            b.pick(studio, id);
                        }
                    }
                }
            }
        });
    if r.should_close() {
        b.open = false;
    }
}
