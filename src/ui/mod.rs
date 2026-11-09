//! The Oxbeat window: Focus layout (top bar, tracks, keyboard strip).

mod keyboard_strip;
pub mod studio;
mod top_bar;
mod tracks;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Color32, Key};

use crate::audio::output::{self, AudioOutput};
use crate::midi::input::{MidiInputs, MidiMsg};
use crate::midi::keyboard::{MAX_OCTAVE, MIN_OCTAVE, key_to_pitch, stale_keys};
use crate::project::io;
use studio::Studio;

pub const BG: Color32 = Color32::from_rgb(0x16, 0x18, 0x1d);
pub const PANEL: Color32 = Color32::from_rgb(0x22, 0x25, 0x2c);
pub const ROW: Color32 = Color32::from_rgb(0x1e, 0x21, 0x28);
pub const ROW_SELECTED: Color32 = Color32::from_rgb(0x26, 0x2b, 0x36);
pub const ACCENT: Color32 = Color32::from_rgb(0x4b, 0x8b, 0xe0);
pub const RECORD: Color32 = Color32::from_rgb(0xd9, 0x43, 0x4f);
pub const CLIP: Color32 = Color32::from_rgb(0x3f, 0xae, 0x6f);
pub const LCD: Color32 = Color32::from_rgb(0x0e, 0x10, 0x13);
pub const LCD_TEXT: Color32 = Color32::from_rgb(0x7f, 0xe0, 0xa8);

const MIDI_RESCAN: Duration = Duration::from_secs(1);
const KEY_VELOCITY: u8 = 100;

/// Apply Oxbeat's dark look.
pub fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.global_style_mut(|style| {
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = PANEL;
        v.window_fill = PANEL;
        v.extreme_bg_color = BG;
        v.selection.bg_fill = ACCENT;
        v.hyperlink_color = ACCENT;
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
    });
}

pub struct OxbeatApp {
    studio: Studio,
    audio: Option<AudioOutput>,
    audio_error: Option<String>,
    midi: MidiInputs,
    last_rescan: Instant,
    keyboards: Vec<String>,
    octave: i8,
    bundle: Option<PathBuf>,
    error: Option<String>,
    add_track: top_bar::AddTrackDialog,
    /// Computer keys currently down -> pitch they started.
    computer_keys: Vec<(Key, u8)>,
    /// Pitch held with the mouse on the on-screen keyboard.
    mouse_note: Option<u8>,
}

impl OxbeatApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);
        let ((cmd_tx, cmd_rx), (evt_tx, evt_rx)) = studio::queues();
        let studio = Studio::new(Studio::starter_project(), cmd_tx, evt_rx);
        let (audio, audio_error) = match output::start(cmd_rx, evt_tx) {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(e.to_string())),
        };
        studio.sync();

        let shared = studio.shared.clone();
        let ctx = cc.egui_ctx.clone();
        let midi = MidiInputs::new(Arc::new(move |msg| {
            match msg {
                MidiMsg::NoteOn { pitch, velocity } => shared.note_on(pitch, velocity),
                MidiMsg::NoteOff { pitch } => shared.note_off(pitch),
            }
            ctx.request_repaint();
        }));
        let mut app = Self {
            studio,
            audio,
            audio_error,
            midi,
            last_rescan: Instant::now() - MIDI_RESCAN,
            keyboards: Vec::new(),
            octave: 4,
            bundle: None,
            error: None,
            add_track: Default::default(),
            computer_keys: Vec::new(),
            mouse_note: None,
        };
        app.rescan_midi();
        app
    }

    fn rescan_midi(&mut self) {
        self.last_rescan = Instant::now();
        let r = self.midi.rescan();
        if !r.removed.is_empty() {
            // A keyboard vanished: make sure nothing it was holding keeps ringing.
            self.studio.shared.release_all();
        }
        self.keyboards = self.midi.connected();
    }

    fn retry_audio(&mut self) {
        self.audio = None;
        let ((cmd_tx, cmd_rx), (evt_tx, evt_rx)) = studio::queues();
        self.studio.reconnect(cmd_tx, evt_rx);
        match output::start(cmd_rx, evt_tx) {
            Ok(a) => {
                self.audio = Some(a);
                self.audio_error = None;
                self.studio.sync();
            }
            Err(e) => self.audio_error = Some(e.to_string()),
        }
    }

    fn new_project(&mut self) {
        self.bundle = None;
        self.studio.replace_project(Studio::starter_project());
    }

    fn open_project(&mut self) {
        let Some(dir) = rfd::FileDialog::new()
            .set_title("Open an Oxbeat project (.oxbeat folder)")
            .pick_folder()
        else {
            return;
        };
        match io::load(&dir) {
            Ok(p) => {
                self.studio.replace_project(p);
                self.bundle = Some(dir);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn save_project(&mut self) {
        let dir = match &self.bundle {
            Some(d) => d.clone(),
            None => {
                let Some(mut path) = rfd::FileDialog::new()
                    .set_title("Save project")
                    .set_file_name(format!("{}.oxbeat", self.studio.project.name))
                    .save_file()
                else {
                    return;
                };
                if path.extension().is_none_or(|e| e != "oxbeat") {
                    path.set_extension("oxbeat");
                }
                if let Some(stem) = path.file_stem() {
                    self.studio.project.name = stem.to_string_lossy().into_owned();
                }
                path
            }
        };
        match io::save(&self.studio.project, &dir) {
            Ok(()) => self.bundle = Some(dir),
            Err(e) => self.error = Some(format!("Could not save: {e}")),
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let command = |i: &egui::InputState, k: Key| i.modifiers.command && i.key_pressed(k);
        let (save, open, new) =
            ctx.input(|i| (command(i, Key::S), command(i, Key::O), command(i, Key::N)));
        if save {
            self.save_project();
        }
        if open {
            self.open_project();
        }
        if new {
            self.new_project();
        }
        // Releases always count, whatever else is going on, so notes never get stuck.
        let (events, focused) =
            ctx.input(|i| (i.events.clone(), i.viewport().focused.unwrap_or(true)));
        for e in &events {
            if let egui::Event::Key {
                key,
                pressed: false,
                ..
            } = e
                && let Some(i) = self.computer_keys.iter().position(|(k, _)| k == key)
            {
                let (_, p) = self.computer_keys.remove(i);
                self.studio.shared.note_off(p);
            }
        }
        // Key-ups can go missing (focus loss, file dialogs): release anything not down.
        let stale = ctx.input(|i| stale_keys(&self.computer_keys, |k| i.key_down(k), focused));
        for i in stale.into_iter().rev() {
            let (_, p) = self.computer_keys.remove(i);
            self.studio.shared.note_off(p);
        }

        if ctx.egui_wants_keyboard_input() || self.add_track.open || self.error.is_some() {
            return;
        }
        for e in events {
            let egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = e
            else {
                continue;
            };
            if modifiers.command || modifiers.alt {
                continue;
            }
            if pressed && !repeat {
                match key {
                    Key::Space => self.studio.play_stop(),
                    Key::R => self.studio.record(),
                    Key::Enter => self.studio.rewind(),
                    Key::Z => self.octave = (self.octave - 1).max(MIN_OCTAVE),
                    Key::X => self.octave = (self.octave + 1).min(MAX_OCTAVE),
                    _ => {
                        if let Some(p) = key_to_pitch(key, self.octave)
                            && !self.computer_keys.iter().any(|(k, _)| *k == key)
                        {
                            self.computer_keys.push((key, p));
                            self.studio.shared.note_on(p, KEY_VELOCITY);
                        }
                    }
                }
            }
        }
    }

    fn error_dialog(&mut self, ctx: &egui::Context) {
        let Some(msg) = self.error.clone() else {
            return;
        };
        let r = egui::Modal::new(egui::Id::unique("error")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("Something went wrong");
            ui.add_space(8.0);
            ui.label(msg);
            ui.add_space(12.0);
            ui.button("OK").clicked()
        });
        if r.inner || r.should_close() {
            self.error = None;
        }
    }
}

impl eframe::App for OxbeatApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.studio.drain_events();
        if self.last_rescan.elapsed() >= MIDI_RESCAN {
            self.rescan_midi();
        }
        if self.audio.as_ref().is_some_and(|a| a.has_failed()) {
            self.audio = None;
            self.audio_error = Some("No audio device found".into());
        }
        // Keep the playhead moving and keep checking for keyboards.
        let next = if self.studio.playing {
            Duration::from_millis(16)
        } else {
            MIDI_RESCAN
        };
        ctx.request_repaint_after(next);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_keys(&ctx);

        if let Some(err) = self.audio_error.clone() {
            egui::Panel::top("audio-banner")
                .frame(egui::Frame::NONE.fill(RECORD).inner_margin(6.0))
                .show(ui, |ui| {
                    let text = format!("{err} — click to retry");
                    if ui
                        .add(
                            egui::Label::new(egui::RichText::new(text).color(Color32::WHITE))
                                .sense(egui::Sense::click()),
                        )
                        .clicked()
                    {
                        self.retry_audio();
                    }
                });
        }

        let mut action = top_bar::Action::None;
        egui::Panel::top("top-bar")
            .frame(egui::Frame::NONE.fill(PANEL).inner_margin(8.0))
            .show(ui, |ui| {
                action = top_bar::show(ui, &mut self.studio, &mut self.add_track);
            });
        match action {
            top_bar::Action::New => self.new_project(),
            top_bar::Action::Open => self.open_project(),
            top_bar::Action::Save => self.save_project(),
            top_bar::Action::None => {}
        }

        egui::Panel::bottom("keyboard-strip")
            .frame(egui::Frame::NONE.fill(PANEL).inner_margin(8.0))
            .show(ui, |ui| {
                keyboard_strip::show(
                    ui,
                    &self.studio,
                    &mut self.octave,
                    &mut self.mouse_note,
                    &self.keyboards,
                );
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ui, |ui| tracks::show(ui, &mut self.studio));

        top_bar::add_track_dialog(&ctx, &mut self.studio, &mut self.add_track);
        self.error_dialog(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_is_dark() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        assert!(ctx.global_style().visuals.dark_mode);
    }
}
