//! Calcul du ReplayGain : chaque morceau est décodé et mesuré (voir `loudness`), puis le gain du
//! morceau et celui de son album sont inscrits dans les tags. Le son des fichiers n'est pas
//! modifié ; le « nivellement du volume » du lecteur s'en sert pour tout jouer au même niveau.
//!
//! L'analyse tourne en arrière-plan, plusieurs morceaux à la fois, album par album : l'arrêter
//! garde les albums déjà terminés.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, RichText};

use super::lib_tools::{self, LibTool};
use super::tag_editor::file_name;
use super::views::segment;
use super::SterMusic;
use crate::i18n::tr;
use crate::icons::Icon;
use crate::library::plural;
use crate::loudness::{self, Analysis};
use crate::tags::{self, ReplayGain, WriteOpts};
use crate::trf;
use crate::{theme, widgets};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Les morceaux désignés depuis un menu (un album, une sélection).
    Custom,
    Editor,
    Library,
}

pub struct RgTool {
    scope: Scope,
    custom: Option<(Vec<PathBuf>, String)>,
    /// Ne pas refaire les albums dont tous les morceaux ont déjà leurs valeurs.
    skip_done: bool,
    job: Option<Job>,
}

impl Default for RgTool {
    fn default() -> Self {
        Self {
            scope: Scope::Library,
            custom: None,
            skip_done: true,
            job: None,
        }
    }
}

struct Job {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    total: usize,
    done: usize,
    current: String,
    /// Morceaux dont les tags ont été écrits, albums terminés, morceaux déjà à jour.
    written: usize,
    albums: usize,
    skipped: usize,
    errors: Vec<(PathBuf, String)>,
    started: Instant,
    /// Durée totale, une fois l'analyse terminée (ou arrêtée).
    finished: Option<Duration>,
    stopped: bool,
}

enum Msg {
    Album(String),
    Track,
    Skipped(usize),
    Written(usize),
    Failed(PathBuf, String),
    Done,
}

/// Un album à analyser : son nom (pour l'affichage) et ses fichiers.
type Album = (String, Vec<PathBuf>);

impl RgTool {
    pub fn running(&self) -> bool {
        self.job.as_ref().is_some_and(|j| j.finished.is_none())
    }

    /// La fenêtre s'ouvre : sur une analyse en cours ou finie si elle existe, sinon sur le choix.
    pub fn opened(&mut self, custom: Option<(Vec<PathBuf>, String)>) {
        if self.running() {
            return;
        }
        self.job = None;
        self.scope = if custom.is_some() {
            Scope::Custom
        } else {
            Scope::Library
        };
        self.custom = custom;
    }

    fn stop(&mut self) {
        if let Some(j) = &mut self.job {
            j.cancel.store(true, Ordering::Relaxed);
            j.stopped = true;
        }
    }

    /// À la fermeture de l'application : on attend que l'écriture en cours se termine.
    pub fn stop_and_wait(&mut self) {
        self.stop();
        if let Some(t) = self.job.as_mut().and_then(|j| j.thread.take()) {
            let _ = t.join();
        }
    }
}

/// Analyse les albums l'un après l'autre ; les morceaux d'un album sont décodés en parallèle.
fn run(
    albums: Vec<Album>,
    skip_done: bool,
    opts: WriteOpts,
    loaded: Arc<Mutex<Option<PathBuf>>>,
    cancel: Arc<AtomicBool>,
    tx: Sender<Msg>,
    ctx: egui::Context,
) {
    // Un cœur est laissé à l'interface et à la lecture.
    let workers = std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1))
        .unwrap_or(2)
        .clamp(1, 8);
    let send = |m: Msg| {
        let _ = tx.send(m);
        ctx.request_repaint();
    };
    for (name, paths) in albums {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        send(Msg::Album(name));
        if skip_done && paths.iter().all(|p| tags::read_replaygain(p).is_some()) {
            send(Msg::Skipped(paths.len()));
            continue;
        }
        let next = AtomicUsize::new(0);
        let slots: Vec<Mutex<Option<Result<Analysis, String>>>> =
            paths.iter().map(|_| Mutex::new(None)).collect();
        std::thread::scope(|s| {
            for _ in 0..workers.min(paths.len()) {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= paths.len() || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    *slots[i].lock().unwrap() = Some(loudness::analyze(&paths[i], &cancel));
                    send(Msg::Track);
                });
            }
        });
        // Arrêt en cours d'album : rien n'est écrit pour lui (son gain d'album serait faux).
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let mut measured: Vec<(&PathBuf, Analysis)> = Vec::new();
        for (path, slot) in paths.iter().zip(slots) {
            match slot.into_inner().unwrap() {
                Some(Ok(a)) => measured.push((path, a)),
                Some(Err(e)) => send(Msg::Failed(path.clone(), e)),
                None => {}
            }
        }
        let album_loudness = loudness::integrated(measured.iter().flat_map(|(_, a)| a.blocks.iter()));
        let album_peak = measured.iter().map(|(_, a)| a.peak).fold(0.0, f32::max);
        let mut written = 0;
        for (path, a) in &measured {
            // Un morceau (ou un album) entièrement silencieux n'a pas de gain : 0 dB.
            let track_loudness = loudness::integrated(a.blocks.iter());
            let track_gain = track_loudness.map(loudness::gain).unwrap_or(0.0);
            let rg = ReplayGain {
                track_gain,
                track_peak: a.peak,
                album_gain: album_loudness.map(loudness::gain).unwrap_or(track_gain),
                album_peak,
            };
            // Le morceau chargé par le lecteur est écrit via une copie.
            let o = WriteOpts {
                via_copy: loaded.lock().unwrap().as_ref() == Some(*path),
                ..opts
            };
            match tags::write_replaygain(path, &rg, o) {
                Ok(()) => written += 1,
                Err(e) => send(Msg::Failed((*path).clone(), e)),
            }
        }
        send(Msg::Written(written));
    }
    send(Msg::Done);
}

impl SterMusic {
    /// Les albums à analyser d'après le choix de la fenêtre.
    fn rg_albums(&self) -> Vec<Album> {
        let lib = &self.lib;
        let lib_album = |a: usize| -> Album {
            let al = &lib.albums[a];
            (
                format!("{} — {}", al.artist, al.title),
                // Seuls les vrais fichiers reçoivent un gain (ni morceau CUE, ni morceau distant).
                al.tracks
                    .iter()
                    .filter(|&&t| lib.tracks[t].is_plain_file())
                    .map(|&t| lib.tracks[t].path.clone())
                    .collect(),
            )
        };
        match self.lib_tools.rg.scope {
            Scope::Library => (0..lib.albums.len()).map(lib_album).collect(),
            Scope::Custom => {
                let paths = self.lib_tools.rg.custom.as_ref().map(|c| c.0.as_slice());
                let mut albums: Vec<Album> = Vec::new();
                let mut index: HashMap<usize, usize> = HashMap::new();
                for p in paths.unwrap_or_default() {
                    match lib.find(p) {
                        // Les morceaux d'un même album de la bibliothèque sont mesurés ensemble.
                        Some(t) => {
                            let a = lib.track_album[t];
                            let slot = *index.entry(a).or_insert_with(|| {
                                albums.push((lib_album(a).0, Vec::new()));
                                albums.len() - 1
                            });
                            albums[slot].1.push(p.clone());
                        }
                        None => albums.push((file_name(p), vec![p.clone()])),
                    }
                }
                albums
            }
            Scope::Editor => {
                use crate::tags::Field;
                let ed = &self.tag_editor;
                let mut albums: Vec<Album> = Vec::new();
                let mut index: HashMap<(String, String), usize> = HashMap::new();
                for k in ed.all_readable() {
                    let v = &ed.values[k];
                    let path = ed.files[k].path.clone();
                    let album = v[Field::Album.index()].trim();
                    let artist = match v[Field::AlbumArtist.index()].trim() {
                        "" => v[Field::Artist.index()].trim(),
                        a => a,
                    };
                    // Sans nom d'album, le morceau est mesuré seul.
                    if album.is_empty() {
                        albums.push((file_name(&path), vec![path]));
                        continue;
                    }
                    let key = (artist.to_lowercase(), album.to_lowercase());
                    let slot = *index.entry(key).or_insert_with(|| {
                        albums.push((format!("{artist} — {album}"), Vec::new()));
                        albums.len() - 1
                    });
                    albums[slot].1.push(path);
                }
                albums
            }
        }
    }

    fn start_replaygain(&mut self, ctx: &egui::Context) {
        if self.tag_editor.saving() {
            self.toast(tr(
                "Enregistrement des tags en cours : réessaie quand il sera terminé",
            ));
            return;
        }
        let albums = self.rg_albums();
        let total: usize = albums.iter().map(|a| a.1.len()).sum();
        if total == 0 {
            self.toast(tr("Aucun morceau à analyser"));
            return;
        }
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let thread = {
            let (opts, loaded, cancel, ctx) = (
                self.write_opts(),
                self.now_loaded.clone(),
                cancel.clone(),
                ctx.clone(),
            );
            let skip_done = self.lib_tools.rg.skip_done;
            std::thread::Builder::new()
                .name("replaygain".into())
                .spawn(move || run(albums, skip_done, opts, loaded, cancel, tx, ctx))
                .ok()
        };
        self.lib_tools.rg.job = Some(Job {
            rx,
            cancel,
            thread,
            total,
            done: 0,
            current: String::new(),
            written: 0,
            albums: 0,
            skipped: 0,
            errors: Vec::new(),
            started: Instant::now(),
            finished: None,
            stopped: false,
        });
    }

    pub(super) fn poll_replaygain(&mut self) {
        let Some(j) = &mut self.lib_tools.rg.job else {
            return;
        };
        if j.finished.is_some() {
            return;
        }
        while let Ok(msg) = j.rx.try_recv() {
            match msg {
                Msg::Album(name) => j.current = name,
                Msg::Track => j.done += 1,
                Msg::Skipped(n) => {
                    j.done += n;
                    j.skipped += n;
                }
                Msg::Written(n) => {
                    j.written += n;
                    j.albums += (n > 0) as usize;
                }
                Msg::Failed(path, why) => j.errors.push((path, why)),
                Msg::Done => j.finished = Some(j.started.elapsed()),
            }
        }
        if j.finished.is_none() {
            return;
        }
        let summary = if j.stopped {
            trf!(
                "Analyse ReplayGain arrêtée : {0}",
                plural(j.written, "morceau mesuré", "morceaux mesurés")
            )
        } else {
            trf!(
                "ReplayGain : {0}",
                plural(j.written, "morceau mesuré", "morceaux mesurés")
            )
        };
        // Fenêtre fermée : on prévient que c'est fini.
        if self.lib_tools.open != Some(LibTool::ReplayGain) {
            self.toast(summary);
        }
    }

    pub(super) fn replaygain_dialog(&mut self, ctx: &egui::Context) {
        let editor_files = self.tag_editor.all_readable().len();
        let library_tracks = self.lib.tracks.len();
        let leveling = self.settings.leveling;
        let rg = &mut self.lib_tools.rg;
        // « Liste de l'éditeur » n'a de sens que si elle contient des fichiers.
        if rg.scope == Scope::Editor && editor_files == 0 {
            rg.scope = Scope::Library;
        }
        let (mut close, mut start, mut stop, mut enable_leveling) = (false, false, false, false);
        let modal = egui::Modal::new(egui::Id::new("replaygain"))
            .frame(lib_tools::modal_frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(560.0);
                lib_tools::title(ui, Icon::Volume2, tr("ReplayGain"), "");
                ui.add_space(12.0);
                match &rg.job {
                    None => {
                        ui.label(
                            RichText::new(tr(
                                "Mesure le volume perçu de chaque morceau et de chaque album (norme \
                                 EBU R 128), puis l'inscrit dans les tags. Le son des fichiers n'est pas \
                                 modifié : c'est le lecteur qui s'en sert pour tout jouer au même niveau.",
                            ))
                            .color(theme::TEXT_2),
                        );
                        ui.add_space(14.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                            ui.label(RichText::new(tr("Analyser :")).color(theme::TEXT_2));
                            if let Some((paths, label)) = &rg.custom {
                                let text = format!("{label} ({})", paths.len());
                                if segment(ui, &text, rg.scope == Scope::Custom).clicked() {
                                    rg.scope = Scope::Custom;
                                }
                            }
                            if editor_files > 0
                                && segment(
                                    ui,
                                    &trf!("Liste de l'éditeur ({editor_files})", editor_files),
                                    rg.scope == Scope::Editor,
                                )
                                .clicked()
                            {
                                rg.scope = Scope::Editor;
                            }
                            if library_tracks > 0
                                && segment(
                                    ui,
                                    &trf!("Toute la bibliothèque ({library_tracks})", library_tracks),
                                    rg.scope == Scope::Library,
                                )
                                .clicked()
                            {
                                rg.scope = Scope::Library;
                            }
                        });
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            widgets::switch(ui, &mut rg.skip_done);
                            ui.label(RichText::new(tr("Passer les albums déjà mesurés")).color(theme::TEXT));
                        });
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(tr(
                                "Compte une à deux secondes par morceau. L'analyse continue si tu fermes \
                                 cette fenêtre, et peut être arrêtée : les albums terminés sont gardés.",
                            ))
                            .small()
                            .color(theme::TEXT_3),
                        );
                    }
                    Some(j) => progress(ui, j, leveling, &mut enable_leveling),
                }
                ui.add_space(18.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &rg.job {
                    None => {
                        if widgets::primary_button(ui, None, tr("Analyser")).clicked() {
                            start = true;
                        }
                        if widgets::secondary_button(ui, None, tr("Annuler")).clicked() {
                            close = true;
                        }
                    }
                    Some(j) if j.finished.is_none() => {
                        if widgets::secondary_button(ui, None, tr("Fermer"))
                            .on_hover_text(tr("L'analyse continue en arrière-plan"))
                            .clicked()
                        {
                            close = true;
                        }
                        if widgets::secondary_button(ui, Some(Icon::Stop), tr("Arrêter")).clicked() {
                            stop = true;
                        }
                    }
                    Some(_) => {
                        if widgets::primary_button(ui, None, tr("Fermer")).clicked() {
                            close = true;
                        }
                    }
                });
            });
        if stop {
            rg.stop();
        }
        if enable_leveling {
            self.settings.leveling = true;
            self.push_audio();
        }
        if start {
            self.start_replaygain(ctx);
        }
        if close || modal.should_close() {
            self.lib_tools.open = None;
            // Une analyse terminée ne se rouvre pas : la prochaine fois, on repart du choix.
            if !self.lib_tools.rg.running() {
                self.lib_tools.rg.job = None;
            }
        }
    }
}

/// Avancement, puis bilan, d'une analyse.
fn progress(ui: &mut egui::Ui, j: &Job, leveling: bool, enable_leveling: &mut bool) {
    let fraction = j.done as f32 / j.total.max(1) as f32;
    match j.finished {
        None => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(16.0).color(theme::accent_light()));
                let text = if j.stopped {
                    tr("Arrêt après le morceau en cours…").to_owned()
                } else {
                    trf!("Analyse… {0} / {1}", j.done, j.total)
                };
                ui.label(RichText::new(text).color(theme::TEXT));
                // Estimation une fois le rythme connu.
                let elapsed = j.started.elapsed().as_secs_f32();
                if !j.stopped && j.done >= 8 && elapsed > 4.0 {
                    let left = elapsed * (j.total - j.done) as f32 / j.done as f32;
                    let text = if left >= 90.0 {
                        trf!("environ {0} min restantes", (left / 60.0).round() as u32)
                    } else {
                        tr("moins de 2 min restantes").to_owned()
                    };
                    ui.label(RichText::new(text).color(theme::TEXT_3));
                }
            });
            ui.add_space(6.0);
            ui.add(egui::ProgressBar::new(fraction).desired_height(6.0));
            ui.add_space(6.0);
            let g = widgets::galley(
                ui,
                &j.current,
                egui::FontId::proportional(13.0),
                theme::TEXT_2,
                ui.available_width(),
            );
            ui.label(g);
        }
        Some(elapsed) => {
            let head = if j.stopped {
                tr("Analyse arrêtée")
            } else {
                tr("Analyse terminée")
            };
            ui.label(RichText::new(head).font(theme::semibold(15.0)).color(theme::TEXT));
            ui.add_space(4.0);
            let mut parts = vec![trf!(
                "{0} dans {1}",
                plural(j.written, "morceau mesuré", "morceaux mesurés"),
                plural(j.albums, "album", "albums")
            )];
            if j.skipped > 0 {
                parts.push(trf!("{0} déjà à jour", j.skipped));
            }
            match elapsed.as_secs() {
                0 => {}
                secs if secs >= 60 => parts.push(format!("{} min {:02} s", secs / 60, secs % 60)),
                secs => parts.push(format!("{secs} s")),
            }
            ui.label(RichText::new(parts.join(" · ")).color(theme::TEXT_2));
            if !leveling && j.written + j.skipped > 0 {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr(
                            "Le nivellement du volume est désactivé : le lecteur n'en tient pas compte.",
                        ))
                        .color(theme::TEXT_2),
                    );
                    if widgets::secondary_button(ui, None, tr("Activer")).clicked() {
                        *enable_leveling = true;
                    }
                });
            }
        }
    }
    if !j.errors.is_empty() {
        ui.add_space(10.0);
        ui.label(
            RichText::new(plural(
                j.errors.len(),
                "morceau n'a pas pu être traité :",
                "morceaux n'ont pas pu être traités :",
            ))
            .color(egui::Color32::from_rgb(0xFF, 0xC0, 0x70)),
        );
        egui::ScrollArea::vertical()
            .id_salt("rg_errors")
            .max_height(110.0)
            .show(ui, |ui| {
                for (path, why) in &j.errors {
                    let text = format!("{} ({why})", file_name(path));
                    let g = widgets::galley(
                        ui,
                        &text,
                        egui::FontId::proportional(12.5),
                        theme::TEXT_3,
                        ui.available_width() - 8.0,
                    );
                    ui.label(g).on_hover_text(path.display().to_string());
                }
            });
    }
}
