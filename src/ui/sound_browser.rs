//! The "New Track" / "Choose a Sound" dialog: built-in sounds, the user's library
//! (downloaded packs and imported files) and the online pack store.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::icons;
use super::studio::Studio;
use super::theme::{self, ACCENT, SURFACE_2, SURFACE_3, TEXT, TEXT_DIM, TEXT_FAINT, bold, font};
use crate::project::TrackKind;
use crate::sounds::essentials;
use crate::sounds::library::{self, PackManifest, SoundKind};
use crate::sounds::packs::{IndexState, PackManager};
use crate::sounds::{self, BUILTIN_INSTRUMENTS, BUILTIN_KITS, IMPORT_EXTENSIONS, Preset};

const LIST_HEIGHT: f32 = 340.0;

/// A SoundFont to pick a preset from: (file, (pack id, file in pack), display name).
type PresetRequest = (PathBuf, Option<(String, String)>, String);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tab {
    BuiltIn,
    Library,
    Store,
}

/// Where Add Track opens, and the sound it marks as the default (the first
/// Library row when Essentials is installed, the built-in synth otherwise).
fn new_track_defaults(drums: bool, essentials_installed: bool) -> (Tab, &'static str) {
    let tab = if essentials_installed {
        Tab::Library
    } else {
        Tab::BuiltIn
    };
    let sound = if drums {
        sounds::default_kit(essentials_installed)
    } else {
        sounds::default_instrument(essentials_installed)
    };
    (tab, sound)
}

#[derive(Clone, PartialEq)]
enum Page {
    TrackType,
    Sounds,
    /// Pick a preset from a SoundFont. `pack`: (pack id, file in pack) when it came from a pack.
    Presets {
        path: PathBuf,
        pack: Option<(String, String)>,
        name: String,
        presets: Vec<(String, Preset)>,
    },
}

/// What picking a sound does.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    NewTrack,
    /// Change the sound of an existing track.
    Track(u64),
}

pub struct Browser {
    pub open: bool,
    credits_open: bool,
    page: Page,
    target: Target,
    drums: bool,
    tab: Tab,
    filter: String,
    pub packs: PackManager,
    pub errors: Vec<String>,
}

impl Browser {
    pub fn new(packs: PackManager) -> Browser {
        Browser {
            open: false,
            credits_open: false,
            page: Page::TrackType,
            target: Target::NewTrack,
            drums: false,
            tab: Tab::BuiltIn,
            filter: String::new(),
            packs,
            errors: Vec::new(),
        }
    }

    pub fn open_new_track(&mut self) {
        self.open = true;
        self.page = Page::TrackType;
        self.target = Target::NewTrack;
        self.tab = new_track_defaults(false, self.essentials_installed()).0;
        self.filter.clear();
    }

    fn essentials_installed(&self) -> bool {
        self.packs.is_installed(essentials::ID).is_some()
    }

    /// Open the sound list for an existing track.
    pub fn open_for_track(&mut self, track: u64, drums: bool) {
        self.open = true;
        self.page = Page::Sounds;
        self.target = Target::Track(track);
        self.drums = drums;
        self.filter.clear();
    }

    fn kind(&self) -> SoundKind {
        if self.drums {
            SoundKind::Drums
        } else {
            SoundKind::Instrument
        }
    }

    fn pick(&mut self, studio: &mut Studio, sound: &str, name: &str) {
        match self.target {
            Target::NewTrack => {
                let id = if self.drums {
                    studio.project.add_drums_track(sound)
                } else {
                    studio.project.add_track(
                        name,
                        TrackKind::Instrument {
                            preset: sound.to_string(),
                        },
                    )
                };
                studio.sync();
                studio.select(id);
            }
            Target::Track(id) => {
                if let Some(t) = studio.project.track_mut(id) {
                    match &mut t.kind {
                        TrackKind::Instrument { preset } => {
                            *preset = sound.to_string();
                            t.name = name.to_string();
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

    fn show_presets(&mut self, path: PathBuf, pack: Option<(String, String)>, name: String) {
        match sounds::soundfont_presets(&path) {
            Ok(presets) => {
                self.filter.clear();
                self.page = Page::Presets {
                    path,
                    pack,
                    name,
                    presets,
                };
            }
            Err(e) => self
                .errors
                .push(format!("Could not open {}: {e}", path.display())),
        }
    }

    fn import(&mut self, studio: &mut Studio) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import a sound")
            .add_filter("Sounds (SF2, SFZ, WAV, FLAC, OGG, MP3)", &IMPORT_EXTENSIONS)
            .pick_file()
        else {
            return;
        };
        let stem = path
            .file_stem()
            .map_or("Sound".into(), |s| s.to_string_lossy().into_owned());
        let is_sf2 = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("sf2"));
        if is_sf2 {
            self.show_presets(path, None, stem);
            return;
        }
        // Check it loads before remembering it.
        match sounds::load(&sounds::file_sound(&path, None), studio.sample_rate) {
            Ok(_) => {
                let sound = library::import_sound(&path, &stem, None, self.kind());
                self.pick(studio, &sound, &stem);
            }
            Err(e) => self.errors.push(format!("Could not import {stem}: {e}")),
        }
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
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 8, SURFACE_3);
    } else if resp.hovered() {
        p.rect_filled(rect, 8, SURFACE_2);
    }
    let icon_rect = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(34.0, 34.0));
    p.rect_filled(icon_rect, 8, SURFACE_3);
    p.text(
        icon_rect.center(),
        Align2::CENTER_CENTER,
        icon,
        theme::icon(16.0),
        ACCENT,
    );
    let x = icon_rect.right() + 12.0;
    let has_caption = !caption.is_empty();
    p.text(
        pos2(x, rect.top() + if has_caption { 17.0 } else { 25.0 }),
        Align2::LEFT_CENTER,
        title,
        bold(13.5),
        TEXT,
    );
    if has_caption {
        p.text(
            pos2(x, rect.top() + 34.0),
            Align2::LEFT_CENTER,
            caption,
            font(11.5),
            TEXT_DIM,
        );
    }
    resp.clicked()
}

fn section_label(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .font(font(10.5))
            .color(TEXT_FAINT),
    );
    ui.add_space(2.0);
}

fn tab_bar(ui: &mut Ui, tab: &mut Tab) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (t, label) in [
            (Tab::BuiltIn, "Built-in"),
            (Tab::Library, "My Library"),
            (Tab::Store, "Get More"),
        ] {
            let on = *tab == t;
            let galley = ui.painter().layout_no_wrap(
                label.into(),
                bold(13.0),
                if on { TEXT } else { TEXT_DIM },
            );
            let (rect, resp) =
                ui.allocate_exact_size(vec2(galley.size().x + 24.0, 30.0), Sense::click());
            let fill = if on {
                SURFACE_3
            } else if resp.hovered() {
                SURFACE_2
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, 15, fill);
            ui.painter()
                .galley(rect.center() - galley.size() / 2.0, galley, TEXT);
            if resp.clicked() {
                *tab = t;
            }
        }
    });
}

fn human_size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else if mb >= 1.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

/// A small text button in the accent color.
fn link_button(ui: &mut Ui, icon: &str, label: &str) -> bool {
    let job = theme::icon_text(icon, label, 13.0, ACCENT);
    ui.add(egui::Button::new(job).fill(egui::Color32::TRANSPARENT))
        .clicked()
}

/// One line per credit; a pack's homepage follows its line.
fn credit_lines(packs: &[PackManifest]) -> Vec<String> {
    let mut lines = vec!["Built-in synths and kits — Oxbeat — GPL-3.0-or-later".to_string()];
    for p in packs {
        lines.push(format!("{} — {} — {}", p.name, p.author, p.license));
        if !p.homepage.is_empty() {
            lines.push(p.homepage.clone());
        }
    }
    lines
}

fn credits_window(ctx: &egui::Context, b: &mut Browser) {
    let lines = credit_lines(&b.packs.installed);
    egui::Window::new("Sound credits")
        .open(&mut b.credits_open)
        .collapsible(false)
        .show(ctx, |ui| {
            for line in &lines {
                if line.starts_with("http") {
                    ui.hyperlink(line);
                } else {
                    ui.label(line);
                }
            }
        });
}

pub fn show(ctx: &egui::Context, studio: &mut Studio, b: &mut Browser) {
    if b.packs.poll() {
        studio.reload_missing();
    }
    b.errors.append(&mut b.packs.errors);
    if b.credits_open {
        credits_window(ctx, b);
    }
    if !b.open {
        return;
    }
    let current = match b.target {
        Target::Track(id) => studio
            .project
            .track(id)
            .and_then(|t| t.sound())
            .map(str::to_string),
        // A new track marks the default sound for its kind.
        Target::NewTrack => Some(
            new_track_defaults(b.drums, b.essentials_installed())
                .1
                .to_string(),
        ),
    };
    let r = egui::Modal::new(egui::Id::unique("sound-browser"))
        .frame(
            egui::Frame::window(&ctx.global_style())
                .inner_margin(24)
                .corner_radius(12),
        )
        .show(ctx, |ui| {
            ui.set_width(500.0);
            header(ui, b);
            ui.add_space(14.0);
            match b.page.clone() {
                Page::TrackType => track_type_page(ui, studio, b),
                Page::Sounds => sounds_page(ui, studio, b, current.as_deref()),
                Page::Presets {
                    path,
                    pack,
                    name,
                    presets,
                } => presets_page(ui, studio, b, &path, pack, &name, &presets),
            }
        });
    if r.should_close() {
        b.open = false;
    }
}

fn header(ui: &mut Ui, b: &mut Browser) {
    ui.horizontal(|ui| {
        let can_go_back = match b.page {
            Page::TrackType => false,
            Page::Sounds => b.target == Target::NewTrack,
            Page::Presets { .. } => true,
        };
        if can_go_back
            && theme::icon_button(ui, icons::CHEVRON_LEFT, 28.0, false)
                .on_hover_text("Back")
                .clicked()
        {
            b.page = match b.page {
                Page::Presets { .. } => Page::Sounds,
                _ => Page::TrackType,
            };
        }
        let title = match &b.page {
            Page::TrackType => "New Track".to_string(),
            Page::Sounds if b.drums => "Choose a Drum Kit".to_string(),
            Page::Sounds => "Choose a Sound".to_string(),
            Page::Presets { name, .. } => name.clone(),
        };
        ui.label(egui::RichText::new(title).font(bold(18.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_button(ui, icons::X, 28.0, false).clicked() {
                b.open = false;
            }
        });
    });
}

fn track_type_page(ui: &mut Ui, studio: &mut Studio, b: &mut Browser) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        if type_card(ui, icons::PIANO, "Instrument", "Play with a keyboard", true) {
            b.page = Page::Sounds;
            b.drums = false;
        }
        if type_card(ui, icons::MIC, "Voice", "Sing or record audio", true) {
            let id = studio.project.add_voice_track();
            studio.sync();
            studio.select(id);
            b.open = false;
        }
        if type_card(ui, icons::DRUM, "Drums", "Beats and grooves", true) {
            b.page = Page::Sounds;
            b.drums = true;
        }
    });
}

fn sounds_page(ui: &mut Ui, studio: &mut Studio, b: &mut Browser, current: Option<&str>) {
    tab_bar(ui, &mut b.tab);
    ui.add_space(10.0);
    let icon = if b.drums {
        icons::DRUM
    } else {
        icons::AUDIO_LINES
    };
    let mut picked: Option<(String, String)> = None;
    let mut presets_for: Option<PresetRequest> = None;
    let mut import = false;
    egui::ScrollArea::vertical()
        .max_height(LIST_HEIGHT)
        .min_scrolled_height(LIST_HEIGHT)
        .auto_shrink([false, false])
        .show(ui, |ui| match b.tab {
            Tab::BuiltIn => {
                let list: &[(&str, &str)] = if b.drums {
                    &BUILTIN_KITS
                } else {
                    &BUILTIN_INSTRUMENTS
                };
                for (id, caption) in list {
                    if sound_row(ui, icon, id, caption, current == Some(*id)) {
                        picked = Some((id.to_string(), id.to_string()));
                    }
                }
            }
            Tab::Library => {
                if link_button(
                    ui,
                    icons::FOLDER_OPEN,
                    "Import a file…  (SF2, SFZ, WAV, FLAC, OGG, MP3)",
                ) {
                    import = true;
                }
                if b.packs.essentials_missing()
                    && !b.packs.downloads.contains_key(essentials::ID)
                    && link_button(
                        ui,
                        icons::DOWNLOAD,
                        "Install instruments  (the free Oxbeat sound library)",
                    )
                {
                    b.packs.request_essentials();
                }
                let kind = b.kind();
                let imports: Vec<_> = library::imports()
                    .into_iter()
                    .filter(|i| i.kind == kind)
                    .collect();
                let mut any = false;
                for category in library::CATEGORIES.iter().copied().chain(["Other"]) {
                    let sounds: Vec<_> = b
                        .packs
                        .installed
                        .iter()
                        .flat_map(|p| p.sounds.iter().map(move |s| (p, s)))
                        .filter(|(p, s)| s.kind == kind && library::category_of(p, s) == category)
                        .collect();
                    if sounds.is_empty() {
                        continue;
                    }
                    any = true;
                    section_label(ui, category);
                    for (pack, s) in sounds {
                        let id = s.sound_id(&pack.id);
                        let sel = id.is_some() && current == id.as_deref();
                        let caption = if s.caption.is_empty() {
                            pack.name.clone()
                        } else {
                            format!("{} · {}", s.caption, pack.name)
                        };
                        if sound_row(ui, icon, &s.name, &caption, sel) {
                            match id {
                                Some(id) => picked = Some((id, s.name.clone())),
                                None => {
                                    presets_for = Some((
                                        s.path(&pack.id),
                                        Some((pack.id.clone(), s.file.clone())),
                                        s.name.clone(),
                                    ))
                                }
                            }
                        }
                    }
                }
                if !imports.is_empty() {
                    any = true;
                    section_label(ui, "Imported");
                    for i in imports {
                        if sound_row(
                            ui,
                            icons::FOLDER_OPEN,
                            &i.name,
                            "",
                            current == Some(&i.sound),
                        ) {
                            picked = Some((i.sound.clone(), i.name.clone()));
                        }
                    }
                }
                if !any {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("Your library is empty")
                                .font(bold(14.0))
                                .color(TEXT_DIM),
                        );
                        ui.label(
                            egui::RichText::new(
                                "Download free packs from “Get More”, or import your own files.",
                            )
                            .color(TEXT_FAINT),
                        );
                    });
                }
                ui.add_space(8.0);
                if link_button(ui, icons::HEADPHONES, "Sound credits") {
                    b.credits_open = true;
                }
            }
            Tab::Store => store(ui, b),
        });
    if import {
        b.import(studio);
    }
    if let Some((path, pack, name)) = presets_for {
        b.show_presets(path, pack, name);
    }
    if let Some((sound, name)) = picked {
        b.pick(studio, &sound, &name);
    }
}

fn store(ui: &mut Ui, b: &mut Browser) {
    b.packs.load_index();
    let entries = match &b.packs.index {
        IndexState::NotLoaded | IndexState::Loading => {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new("Loading the sound library…").color(TEXT_DIM));
            });
            return;
        }
        IndexState::Failed(e) => {
            let e = e.clone();
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(e).color(TEXT_DIM));
                ui.add_space(8.0);
                if theme::primary_button(ui, None, "Try again").clicked() {
                    b.packs.retry_index();
                }
            });
            return;
        }
        IndexState::Ready(entries) => entries.clone(),
    };
    for e in entries {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect.shrink2(vec2(0.0, 3.0)), 8, SURFACE_2);
        let left = rect.left() + 14.0;
        p.text(
            pos2(left, rect.top() + 20.0),
            Align2::LEFT_CENTER,
            &e.name,
            bold(13.5),
            TEXT,
        );
        let meta = [e.category.as_str(), &human_size(e.size), e.license.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("  ·  ");
        p.text(
            pos2(left, rect.top() + 38.0),
            Align2::LEFT_CENTER,
            &e.description,
            font(11.5),
            TEXT_DIM,
        );
        p.text(
            pos2(left, rect.top() + 52.0),
            Align2::LEFT_CENTER,
            meta,
            font(10.5),
            TEXT_FAINT,
        );
        let btn = Rect::from_min_size(
            pos2(rect.right() - 112.0, rect.center().y - 15.0),
            vec2(100.0, 30.0),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(btn)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        if let Some(progress) = b.packs.downloads.get(&e.id) {
            let f = progress.load(Ordering::Relaxed) as f32 / 1000.0;
            child.add(
                egui::ProgressBar::new(f)
                    .desired_width(100.0)
                    .corner_radius(4),
            );
        } else if let Some(installed) = b.packs.is_installed(&e.id) {
            let outdated = !e.version.is_empty() && installed.version != e.version;
            if outdated {
                if theme::primary_button(&mut child, None, "Update").clicked() {
                    b.packs.install(e.clone());
                }
            } else if child
                .add(egui::Button::new(
                    egui::RichText::new("Remove").color(TEXT_DIM),
                ))
                .on_hover_text("Delete this pack from your computer")
                .clicked()
            {
                b.packs.uninstall(&e.id);
            }
        } else if theme::primary_button(&mut child, None, "Download").clicked() {
            b.packs.install(e.clone());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn presets_page(
    ui: &mut Ui,
    studio: &mut Studio,
    b: &mut Browser,
    path: &std::path::Path,
    pack: Option<(String, String)>,
    name: &str,
    presets: &[(String, Preset)],
) {
    ui.add(
        egui::TextEdit::singleline(&mut b.filter)
            .hint_text("Search sounds")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(8.0);
    let filter = b.filter.to_lowercase();
    let drums = b.drums;
    let mut picked = None;
    egui::ScrollArea::vertical()
        .max_height(LIST_HEIGHT)
        .min_scrolled_height(LIST_HEIGHT)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut shown = 0;
            for (pname, preset) in presets {
                // Drum kits live in bank 128; show those for drum tracks, the rest otherwise.
                if preset.is_drums() != drums {
                    continue;
                }
                if !filter.is_empty() && !pname.to_lowercase().contains(&filter) {
                    continue;
                }
                shown += 1;
                let caption = format!("Bank {} · Program {}", preset.bank, preset.program + 1);
                let icon = if drums {
                    icons::DRUM
                } else {
                    icons::AUDIO_LINES
                };
                if sound_row(ui, icon, pname, &caption, false) {
                    picked = Some((pname.clone(), *preset));
                }
            }
            if shown == 0 {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    let msg = if drums {
                        "This SoundFont has no drum kits."
                    } else {
                        "No matching sounds."
                    };
                    ui.label(egui::RichText::new(msg).color(TEXT_DIM));
                });
            }
        });
    if let Some((pname, preset)) = picked {
        let label = if pname.is_empty() {
            name.to_string()
        } else {
            pname
        };
        let sound = match &pack {
            Some((id, file)) => format!("pack:{id}/{file}#{}:{}", preset.bank, preset.program),
            None => library::import_sound(path, &label, Some(preset), b.kind()),
        };
        b.pick(studio, &sound, &label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sounds::library::PackManifest;

    fn pack(name: &str, homepage: &str) -> PackManifest {
        serde_json::from_str(&format!(
            r#"{{"id":"p","name":"{name}","author":"Ann","license":"CC0","homepage":"{homepage}","sounds":[]}}"#
        ))
        .unwrap()
    }

    #[test]
    fn new_tracks_default_to_essentials_when_installed() {
        assert_eq!(
            new_track_defaults(false, true),
            (Tab::Library, "pack:essentials/Grand Piano.sfz")
        );
        assert_eq!(
            new_track_defaults(true, true),
            (Tab::Library, "pack:essentials/Drums.sfz")
        );
        assert_eq!(new_track_defaults(false, false), (Tab::BuiltIn, "Keys"));
        assert_eq!(
            new_track_defaults(true, false),
            (Tab::BuiltIn, "Studio Kit")
        );
    }

    #[test]
    fn credit_lines_list_every_installed_pack() {
        let lines = credit_lines(&[pack("A", "https://a.example"), pack("B", "")]);
        assert_eq!(
            lines,
            [
                "Built-in synths and kits — Oxbeat — GPL-3.0-or-later",
                "A — Ann — CC0",
                "https://a.example",
                "B — Ann — CC0",
            ]
        );
    }
}
