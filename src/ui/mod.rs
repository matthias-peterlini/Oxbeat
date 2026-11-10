//! The Oxbeat window: Focus layout (top bar, tracks, keyboard strip).

mod effects_panel;
mod history;
mod icons;
mod keyboard_strip;
mod loader;
mod mixer;
mod piano_roll;
mod sound_browser;
pub mod studio;
mod theme;
mod top_bar;
mod tracks;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use egui::{Color32, Key};

use crate::audio::input::{self, AudioInput};
use crate::audio::output::{self, AudioOutput};
use crate::engine::commands::link;
use crate::midi::input::{MidiInputs, MidiMsg};
use crate::midi::keyboard::{MAX_OCTAVE, MIN_OCTAVE, key_to_pad, key_to_pitch, stale_keys};
use crate::project::io;
use studio::Studio;

const MIDI_RESCAN: Duration = Duration::from_secs(1);
const KEY_VELOCITY: u8 = 100;
/// How often to look for a microphone while a voice track is waiting for one.
const INPUT_RETRY: Duration = Duration::from_secs(2);

pub use theme::apply as apply_theme;
use theme::{BG, BORDER, RECORD, SURFACE, TEXT_DIM};

/// What the panel along the bottom shows.
#[derive(Clone, Copy, PartialEq)]
enum Bottom {
    /// The keyboard, drum pads or microphone of the selected track.
    Play,
    /// The notes of the selected clip.
    Editor,
    Effects,
    Mixer,
}

pub struct OxbeatApp {
    studio: Studio,
    bottom: Bottom,
    editor: piano_roll::Editor,
    audio: Option<AudioOutput>,
    audio_error: Option<String>,
    input: Option<AudioInput>,
    last_input_try: Instant,
    midi: MidiInputs,
    last_rescan: Instant,
    keyboards: Vec<String>,
    octave: i8,
    bundle: Option<PathBuf>,
    error: Option<String>,
    /// An export in progress, and how far along it is.
    export: Option<(mpsc::Receiver<crate::export::Update>, f32)>,
    /// Good news to show (e.g. where the exported song went).
    notice: Option<String>,
    /// The instrument-library strip was closed (until the next launch).
    essentials_dismissed: bool,
    add_track: sound_browser::Browser,
    /// Computer keys currently down -> pitch they started.
    computer_keys: Vec<(Key, u8)>,
    /// Pitch held with the mouse on the on-screen keyboard.
    mouse_note: Option<u8>,
}

impl OxbeatApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);
        let (ui_end, engine_end) = link();
        let packs = crate::sounds::packs::PackManager::new({
            let ctx = cc.egui_ctx.clone();
            Arc::new(move || ctx.request_repaint())
        });
        let essentials_installed = packs.is_installed(crate::sounds::essentials::ID).is_some();
        let (audio, audio_error) = match output::start(engine_end) {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(e.to_string())),
        };
        // Built at the device's rate straight away, so the starter piano loads once
        // (in the background).
        let studio = Studio::with_sample_rate(
            Studio::starter_project(essentials_installed),
            ui_end,
            audio.as_ref().map_or(48_000, |a| a.sample_rate()),
        );
        {
            let ctx = cc.egui_ctx.clone();
            studio.set_repaint(move || ctx.request_repaint());
        }

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
            bottom: Bottom::Play,
            editor: piano_roll::Editor::default(),
            audio,
            audio_error,
            input: None,
            last_input_try: Instant::now() - INPUT_RETRY,
            midi,
            last_rescan: Instant::now() - MIDI_RESCAN,
            keyboards: Vec::new(),
            octave: 4,
            bundle: None,
            error: None,
            export: None,
            notice: None,
            essentials_dismissed: false,
            add_track: sound_browser::Browser::new(packs),
            computer_keys: Vec::new(),
            mouse_note: None,
        };
        app.rescan_midi();
        #[cfg(debug_assertions)]
        app.demo();
        app
    }

    /// Debug builds only: `OXBEAT_DEMO=effects` or `=mixer` opens on that panel with a
    /// few tracks, for checking the look without clicking through the app.
    #[cfg(debug_assertions)]
    fn demo(&mut self) {
        let Ok(what) = std::env::var("OXBEAT_DEMO") else {
            return;
        };
        let p = &mut self.studio.project;
        p.add_drums_track("Studio Kit");
        let voice = p.add_voice_track();
        if let Some(t) = p.track_mut(voice) {
            t.effects = crate::project::effects::preset("Tuned Pop");
            t.pan = 0.3;
        }
        self.studio.select(voice);
        self.studio.sync();
        self.bottom = match what.as_str() {
            "mixer" => Bottom::Mixer,
            "editor" | "beats" => {
                // A clip with a few notes, on the keys or the drums.
                let drums = what == "beats";
                let p = &mut self.studio.project;
                let track = p.tracks[if drums { 1 } else { 0 }].id;
                let pitches: [u8; 6] = if drums {
                    [36, 42, 38, 42, 36, 38]
                } else {
                    [60, 64, 67, 72, 67, 64]
                };
                if let Some(clip) = p.add_empty_clip(track, 0.0, 8.0) {
                    if let Some(c) = p.clip_mut(clip) {
                        c.notes = pitches
                            .iter()
                            .enumerate()
                            .map(|(i, pitch)| crate::project::Note {
                                pitch: *pitch,
                                velocity: 60 + 10 * i as u8,
                                start_beat: i as f64,
                                length_beats: if drums { 0.25 } else { 0.75 },
                            })
                            .collect();
                    }
                    self.studio.select_clip(clip);
                    self.studio.sync();
                }
                Bottom::Editor
            }
            "effects" => Bottom::Effects,
            _ => Bottom::Play,
        };
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
        // The microphone is reopened for the new engine (and its sample rate).
        self.input = None;
        let (ui_end, engine_end) = link();
        self.studio.reconnect(ui_end);
        match output::start(engine_end) {
            Ok(a) => {
                self.studio.set_sample_rate(a.sample_rate());
                self.audio = Some(a);
                self.audio_error = None;
            }
            Err(e) => self.audio_error = Some(e.to_string()),
        }
    }

    /// Keep the microphone open exactly while the project has a voice track.
    fn manage_input(&mut self) {
        if self.input.as_ref().is_some_and(|i| i.has_failed()) {
            self.input = None;
            self.studio.set_input(None);
        }
        let wanted = self.studio.needs_input() && self.audio.is_some();
        if !wanted {
            if self.input.take().is_some() {
                self.studio.set_input(None);
            }
            return;
        }
        if self.input.is_some() || self.last_input_try.elapsed() < INPUT_RETRY {
            return;
        }
        self.last_input_try = Instant::now();
        // Failing quietly: the voice strip says no microphone was found, and pressing
        // record explains it.
        if let Ok((mic, queue)) = input::start(self.studio.sample_rate) {
            self.studio.set_input(Some((mic.name().to_string(), queue)));
            self.input = Some(mic);
        }
    }

    fn new_project(&mut self) {
        self.bundle = None;
        let essentials = self
            .add_track
            .packs
            .is_installed(crate::sounds::essentials::ID)
            .is_some();
        self.studio.replace_project(
            Studio::starter_project(essentials),
            studio::unsaved_audio_dir(),
        );
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
                self.studio.replace_project(p, dir.join(io::AUDIO_DIR));
                self.studio.mark_on_disk();
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
        let saved = self
            .studio
            .move_audio_to(&dir.join(io::AUDIO_DIR))
            .and_then(|()| io::save(&self.studio.project, &dir).map_err(|e| e.to_string()));
        match saved {
            Ok(()) => {
                self.studio.mark_on_disk();
                self.bundle = Some(dir);
            }
            Err(e) => self.error = Some(format!("Could not save: {e}")),
        }
    }

    /// Ask where to save the song as audio, then render it in the background.
    fn export_song(&mut self, ctx: &egui::Context) {
        if self.export.is_some() {
            return;
        }
        let Some(mut path) = rfd::FileDialog::new()
            .set_title("Export song")
            .set_file_name(format!("{}.mp3", self.studio.project.name))
            .add_filter("MP3 (small, for sharing)", &["mp3"])
            .add_filter("WAV (full quality)", &["wav"])
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("mp3");
        }
        let ctx = ctx.clone();
        let updates = crate::export::start(
            self.studio.project.clone(),
            self.studio.audio_dir.clone(),
            path,
            move || ctx.request_repaint(),
        );
        self.export = Some((updates, 0.0));
    }

    /// Follow a running export and show its progress.
    fn export_dialog(&mut self, ctx: &egui::Context) {
        let Some((updates, progress)) = &mut self.export else {
            return;
        };
        let mut done = None;
        loop {
            match updates.try_recv() {
                Ok(crate::export::Update::Progress(p)) => *progress = p,
                Ok(crate::export::Update::Done(result)) => done = Some(result),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    done.get_or_insert(Err("The export stopped unexpectedly.".into()));
                    break;
                }
            }
        }
        let progress = *progress;
        if let Some(result) = done {
            self.export = None;
            match result {
                Ok(path) => {
                    self.notice = Some(format!("Your song was saved as\n{}", path.display()))
                }
                Err(e) => self.error = Some(format!("Could not export the song. {e}")),
            }
            return;
        }
        egui::Modal::new(egui::Id::unique("export"))
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .inner_margin(24)
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(egui::RichText::new("Exporting your song…").font(theme::bold(17.0)));
                ui.add_space(12.0);
                ui.add(egui::ProgressBar::new(progress).desired_height(8.0));
                ui.add_space(4.0);
            });
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let command = |i: &egui::InputState, k: Key| i.modifiers.command && i.key_pressed(k);
        let (save, open, new) =
            ctx.input(|i| (command(i, Key::S), command(i, Key::O), command(i, Key::N)));
        if ctx.input(|i| command(i, Key::E)) {
            self.export_song(ctx);
        }
        if save {
            self.save_project();
        }
        if open {
            self.open_project();
        }
        if new {
            self.new_project();
        }
        if !ctx.egui_wants_keyboard_input() && !self.add_track.open && self.error.is_none() {
            let (undo, redo, duplicate, split) = ctx.input(|i| {
                (
                    command(i, Key::Z) && !i.modifiers.shift,
                    (command(i, Key::Z) && i.modifiers.shift) || command(i, Key::Y),
                    command(i, Key::D),
                    command(i, Key::T),
                )
            });
            if undo {
                self.studio.undo();
            }
            if redo {
                self.studio.redo();
            }
            if duplicate {
                self.studio.duplicate_selected_clip();
            }
            if split {
                self.studio.split_selected_clip();
            }
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

        if ctx.egui_wants_keyboard_input()
            || self.add_track.open
            || self.error.is_some()
            || self.notice.is_some()
            || self.export.is_some()
        {
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
                    Key::Delete | Key::Backspace => {
                        // A picked note goes first; otherwise the picked clip.
                        let note = self.bottom == Bottom::Editor
                            && self.editor.delete_selected(&mut self.studio);
                        if !note {
                            self.studio.delete_selected_clip();
                        }
                    }
                    Key::Z => self.octave = (self.octave - 1).max(MIN_OCTAVE),
                    Key::X => self.octave = (self.octave + 1).min(MAX_OCTAVE),
                    _ => {
                        let drums = self
                            .studio
                            .project
                            .track(self.studio.selected())
                            .is_some_and(|t| t.is_drums());
                        let pitch = if drums {
                            key_to_pad(key).map(|i| crate::engine::drums::PADS[i].0)
                        } else {
                            key_to_pitch(key, self.octave)
                        };
                        if let Some(p) = pitch
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
        let (title, msg) = match (&self.error, &self.notice) {
            (Some(e), _) => ("Something went wrong", e.clone()),
            (None, Some(n)) => ("All done", n.clone()),
            (None, None) => return,
        };
        let r = egui::Modal::new(egui::Id::unique("error"))
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .inner_margin(24)
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(egui::RichText::new(title).font(theme::bold(17.0)));
                ui.add_space(8.0);
                ui.label(egui::RichText::new(msg).color(theme::TEXT_DIM));
                ui.add_space(18.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::primary_button(ui, None, "OK").clicked()
                })
                .inner
            });
        if r.inner || r.should_close() {
            // The error shows first; closing it reveals any notice behind it.
            if self.error.take().is_none() {
                self.notice = None;
            }
        }
    }

    /// A slim, dismissible strip while the instrument library downloads or fails.
    fn essentials_strip(&mut self, ui: &mut egui::Ui) {
        use crate::sounds::packs::EssentialsStatus;
        if self.essentials_dismissed {
            return;
        }
        let text = match self.add_track.packs.essentials_status() {
            EssentialsStatus::Hidden => return,
            EssentialsStatus::Downloading { permille, size } => {
                let mut t = format!("Downloading the instrument library… {}%", permille / 10);
                if size > 0 {
                    let mb = |b: f64| b / 1_048_576.0;
                    t += &format!(
                        " ({:.0} of {:.0} MB)",
                        mb(size as f64 * permille as f64 / 1000.0),
                        mb(size as f64)
                    );
                }
                t
            }
            EssentialsStatus::Failed(e) => format!(
                "Couldn't download the instrument library: {e} Oxbeat will try again next time."
            ),
        };
        egui::Panel::top("essentials-strip")
            .frame(
                egui::Frame::NONE
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(16, 6))
                    .stroke(egui::Stroke::new(1.0, BORDER)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(text).color(TEXT_DIM));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button(ui, icons::X, 22.0, false)
                            .on_hover_text("Hide")
                            .clicked()
                        {
                            self.essentials_dismissed = true;
                        }
                    });
                });
            });
    }
}

impl eframe::App for OxbeatApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.studio.drain_events();
        self.manage_input();
        self.studio.update_meter();
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
        } else if (self.input.is_some() && self.studio.voice_selected())
            || (self.bottom == Bottom::Mixer && self.studio.meters_moving())
        {
            // Keep the level meters alive.
            Duration::from_millis(33)
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
                .frame(
                    egui::Frame::NONE
                        .fill(RECORD)
                        .inner_margin(egui::Margin::symmetric(16, 8)),
                )
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

        self.add_track.packs.ensure_essentials();
        self.essentials_strip(ui);

        let mut action = top_bar::Action::None;
        egui::Panel::top("top-bar")
            .exact_size(top_bar::HEIGHT + 1.0)
            .frame(
                egui::Frame::NONE
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(12, 0))
                    .stroke(egui::Stroke::new(1.0, BORDER)),
            )
            .show(ui, |ui| {
                action = top_bar::show(ui, &mut self.studio, &mut self.add_track);
            });
        match action {
            top_bar::Action::New => self.new_project(),
            top_bar::Action::Open => self.open_project(),
            top_bar::Action::Save => self.save_project(),
            top_bar::Action::Export => self.export_song(&ctx),
            top_bar::Action::None => {}
        }

        egui::Panel::bottom("keyboard-strip")
            .frame(
                egui::Frame::NONE
                    .fill(SURFACE)
                    .inner_margin(egui::Margin {
                        left: 16,
                        right: 16,
                        top: 10,
                        bottom: 14,
                    })
                    .stroke(egui::Stroke::new(1.0, BORDER)),
            )
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for (tab, glyph, tip) in [
                            (Bottom::Play, icons::KEYBOARD_MUSIC, "Play"),
                            (Bottom::Editor, icons::PENCIL, "Note editor"),
                            (Bottom::Effects, icons::SLIDERS, "Effects"),
                            (Bottom::Mixer, icons::MIXER, "Mixer"),
                        ] {
                            if theme::icon_button(ui, glyph, 30.0, self.bottom == tab)
                                .on_hover_text(tip)
                                .clicked()
                            {
                                self.bottom = tab;
                            }
                        }
                    });
                    ui.vertical(|ui| match self.bottom {
                        Bottom::Play => keyboard_strip::show(
                            ui,
                            &mut self.studio,
                            &mut self.octave,
                            &mut self.mouse_note,
                            &self.keyboards,
                        ),
                        Bottom::Editor => piano_roll::show(ui, &mut self.studio, &mut self.editor),
                        Bottom::Effects => effects_panel::show(ui, &mut self.studio),
                        Bottom::Mixer => mixer::show(ui, &mut self.studio),
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ui, |ui| {
                if tracks::show(ui, &mut self.studio, &mut self.add_track) {
                    self.bottom = Bottom::Editor;
                }
            });

        sound_browser::show(&ctx, &mut self.studio, &mut self.add_track);
        // `show` polls the pack manager; a fresh Essentials install upgrades an untouched
        // starter project to the grand piano.
        for id in std::mem::take(&mut self.add_track.packs.just_installed) {
            if id == crate::sounds::essentials::ID && self.studio.upgrade_starter_track() {
                self.studio.sync();
            }
        }
        self.studio.load_errors.append(&mut self.add_track.errors);
        if self.error.is_none() && !self.studio.load_errors.is_empty() {
            self.error = Some(std::mem::take(&mut self.studio.load_errors).join(
                "
",
            ));
        }
        self.export_dialog(&ctx);
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
