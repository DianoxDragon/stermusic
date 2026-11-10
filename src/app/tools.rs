//! Page « Outils » : l'éditeur de tags et les outils d'entretien rangés au même endroit, plus
//! Convertir, Extraire un CD et Synchroniser vers un appareil.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, pos2, vec2, Color32, CornerRadius, FontId, Rect, RichText, Sense};

use super::lib_tools::LibTool;
use super::views::{heading, segment};
use super::{Action, Page, PlaylistId, SterMusic, View};
use crate::convert::{self, Format, Source};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::{plural, Track};
use crate::trf;
use crate::{theme, widgets};

/// Lecture d'un CD : ses pistes, et le disque reconnu par MusicBrainz.
type CdRead = Result<(Vec<(Duration, Duration)>, Option<crate::musicbrainz::Disc>), String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Convert,
    Cd,
    Sync,
}

/// Un fichier à produire : d'où, vers où, en quel format (`None` : simple copie).
struct Item {
    src: Source,
    out: PathBuf,
    format: Option<Format>,
    tags: Vec<(&'static str, String)>,
    label: String,
}

#[derive(Default)]
struct JobStatus {
    done: usize,
    total: usize,
    current: String,
    errors: Vec<String>,
    finished: bool,
    /// Message de fin (dossier de destination, appareil).
    summary: String,
}

struct Job {
    status: Arc<Mutex<JobStatus>>,
    stop: Arc<AtomicBool>,
}

/// Où synchroniser : un dossier, ou un téléphone en MTP (adresse KIO).
#[derive(Clone, Debug, PartialEq)]
enum Dest {
    Folder(PathBuf),
    Mtp(String),
}

#[derive(Default)]
pub struct ToolsState {
    pub open: Option<Tool>,
    job: Option<Job>,
    // Convertir.
    convert_tracks: Vec<Track>,
    format: Format,
    convert_dest: Option<PathBuf>,
    // Extraire un CD.
    cd_drive: Option<PathBuf>,
    cd_reading: Option<Receiver<CdRead>>,
    cd_tracks: Vec<(Duration, Duration)>,
    cd_titles: Vec<String>,
    cd_album: String,
    cd_artist: String,
    cd_year: String,
    cd_error: Option<String>,
    cd_dest: Option<PathBuf>,
    // Synchroniser.
    sync_lists: HashSet<String>,
    sync_dest: Option<Dest>,
    sync_convert: bool,
    devices: Vec<String>,
    devices_rx: Option<Receiver<Vec<String>>>,
}

impl ToolsState {
    pub fn busy(&self) -> bool {
        self.job
            .as_ref()
            .is_some_and(|j| !j.status.lock().unwrap().finished)
    }
}

/// Lance les conversions et copies l'une après l'autre ; `after` tourne à la fin (copie vers un
/// téléphone, playlists).
fn start_job(
    items: Vec<Item>,
    after: Option<Box<dyn FnOnce() -> Result<String, String> + Send>>,
    ctx: &egui::Context,
) -> Job {
    let status = Arc::new(Mutex::new(JobStatus {
        total: items.len(),
        ..Default::default()
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let (st, sp, ctx) = (status.clone(), stop.clone(), ctx.clone());
    std::thread::spawn(move || {
        for item in items {
            if sp.load(Ordering::Relaxed) {
                break;
            }
            st.lock().unwrap().current = item.label.clone();
            ctx.request_repaint();
            let result = match item.format {
                Some(f) => convert::run(&item.src, &item.out, f, &item.tags, &sp),
                None => item
                    .out
                    .parent()
                    .map(std::fs::create_dir_all)
                    .unwrap_or(Ok(()))
                    .and_then(|_| std::fs::copy(&item.src.input, &item.out).map(|_| ()))
                    .map_err(|e| e.to_string()),
            };
            let mut s = st.lock().unwrap();
            s.done += 1;
            if let Err(e) = result {
                s.errors.push(format!("{} — {e}", item.label));
            }
        }
        let summary = match after {
            Some(f) if !sp.load(Ordering::Relaxed) => f(),
            _ => Ok(String::new()),
        };
        let mut s = st.lock().unwrap();
        match summary {
            Ok(m) => s.summary = m,
            Err(e) => s.errors.push(e),
        }
        s.finished = true;
        s.current.clear();
        ctx.request_repaint();
    });
    Job { status, stop }
}

/// Appareils MTP (téléphones) vus par KDE, sous la forme `mtp:/Appareil/Stockage`.
fn mtp_devices() -> Vec<String> {
    let ls = |url: &str| -> Vec<String> {
        std::process::Command::new("kioclient")
            .args(["ls", url])
            .stderr(std::process::Stdio::null())
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|l| l.trim().trim_end_matches('/').to_owned())
                    .filter(|l| !l.is_empty() && l != "." && l != "..")
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for dev in ls("mtp:/") {
        for storage in ls(&format!("mtp:/{dev}")) {
            out.push(format!("mtp:/{dev}/{storage}"));
        }
    }
    out
}

fn music_dir() -> PathBuf {
    crate::library::default_music_dir()
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Musique"))
}

impl SterMusic {
    // ---------- Page ----------

    pub(super) fn tools_page(&mut self, ui: &mut egui::Ui) {
        heading(ui, tr("Outils"));
        ui.add_space(16.0);
        let cards: [(Icon, &str, &str, Action); 7] = [
            (
                Icon::Tag,
                tr("Éditeur de tags"),
                tr("Titres, artistes, pochettes, renommage, import MusicBrainz."),
                Action::Go(View::Page(Page::Tags)),
            ),
            (
                Icon::Image,
                tr("Pochettes manquantes"),
                tr("Trouve les albums sans pochette et les complète en une fois."),
                Action::LibTool(LibTool::Covers),
            ),
            (
                Icon::Volume2,
                tr("ReplayGain"),
                tr("Mesure le volume des morceaux pour les jouer tous au même niveau."),
                Action::LibTool(LibTool::ReplayGain),
            ),
            (
                Icon::Images,
                tr("Doublons"),
                tr("Repère les morceaux en double et met ceux en trop à la corbeille."),
                Action::LibTool(LibTool::Duplicates),
            ),
            (
                Icon::Refresh,
                tr("Convertir"),
                tr("En MP3, Opus, AAC, Vorbis ou FLAC, tags et pochette compris."),
                Action::OpenTool(Tool::Convert),
            ),
            (
                Icon::Disc,
                tr("Extraire un CD"),
                tr("Copie un CD audio, avec les titres trouvés sur MusicBrainz."),
                Action::OpenTool(Tool::Cd),
            ),
            (
                Icon::Upload,
                tr("Synchroniser"),
                tr("Copie des playlists vers une clé, une carte ou un téléphone."),
                Action::OpenTool(Tool::Sync),
            ),
        ];
        egui::ScrollArea::vertical()
            .id_salt("tools")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                let w = ui.available_width();
                let cols = ((w + 12.0) / 300.0).floor().max(1.0) as usize;
                let cw = (w - 12.0 * (cols - 1) as f32) / cols as f32;
                let mut cards = cards.into_iter().enumerate().peekable();
                while cards.peek().is_some() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        for _ in 0..cols {
                            let Some((k, (icon, title, desc, action))) = cards.next() else {
                                break;
                            };
                            let a = super::lists::stagger(ui.ctx(), k);
                            ui.scope(|ui| {
                                ui.set_opacity(a);
                                if tool_card(ui, icon, title, desc, cw) {
                                    self.actions.push(action);
                                }
                            });
                        }
                    });
                    ui.add_space(12.0);
                }
                if let Some(job) = &self.tools.job {
                    let s = job.status.lock().unwrap();
                    if !s.finished {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new(trf!("En cours : {0} / {1} — {2}", s.done, s.total, s.current))
                                    .color(theme::TEXT_2),
                            );
                        });
                    }
                }
            });
    }

    // ---------- Fenêtres ----------

    pub(super) fn open_tool(&mut self, ctx: &egui::Context, tool: Tool) {
        self.tools.open = Some(tool);
        match tool {
            Tool::Convert => {}
            Tool::Cd => self.read_cd(ctx),
            Tool::Sync => self.find_devices(ctx),
        }
    }

    /// « Convertir… » depuis un menu : ces morceaux.
    pub(super) fn convert_tracks(&mut self, tracks: Vec<usize>) {
        self.tools.convert_tracks = tracks.iter().map(|&i| self.lib.tracks[i].clone()).collect();
        self.tools.open = Some(Tool::Convert);
    }

    fn read_cd(&mut self, ctx: &egui::Context) {
        let t = &mut self.tools;
        t.cd_drive = convert::cd_drive();
        t.cd_error = None;
        let Some(drive) = t.cd_drive.clone() else {
            t.cd_error = Some(tr("Aucun lecteur de CD détecté").into());
            return;
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = convert::cd_tracks(&drive).map(|tracks| {
                let (offsets, leadout) = convert::toc(&tracks);
                let disc = crate::musicbrainz::disc_by_toc(&offsets, leadout).ok().flatten();
                (tracks, disc)
            });
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        t.cd_reading = Some(rx);
    }

    fn find_devices(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(mtp_devices());
            ctx.request_repaint();
        });
        self.tools.devices_rx = Some(rx);
    }

    pub(super) fn poll_tools(&mut self, ctx: &egui::Context) {
        let t = &mut self.tools;
        if let Some(rx) = &t.cd_reading {
            if let Ok(result) = rx.try_recv() {
                t.cd_reading = None;
                match result {
                    Ok((tracks, disc)) => {
                        let disc = disc.unwrap_or_default();
                        t.cd_titles = (0..tracks.len())
                            .map(|i| {
                                disc.titles
                                    .get(i)
                                    .map(|(title, _)| title.clone())
                                    .unwrap_or_else(|| format!("Piste {:02}", i + 1))
                            })
                            .collect();
                        t.cd_album = disc.album;
                        t.cd_artist = disc.artist;
                        t.cd_year = disc.year.map(|y| y.to_string()).unwrap_or_default();
                        t.cd_tracks = tracks;
                    }
                    Err(e) => t.cd_error = Some(e),
                }
            }
        }
        if let Some(rx) = &t.devices_rx {
            if let Ok(list) = rx.try_recv() {
                t.devices_rx = None;
                t.devices = list;
            }
        }
        let running = t.busy();
        let finished = t.job.as_ref().map(|j| {
            let s = j.status.lock().unwrap();
            (s.finished, s.errors.len(), s.summary.clone())
        });
        if let Some((true, errors, summary)) = finished {
            if t.open.is_none() {
                t.job = None;
                let msg = if errors > 0 {
                    trf!("Terminé, avec {0}", plural(errors, "erreur", "erreurs"))
                } else if summary.is_empty() {
                    tr("Terminé").to_owned()
                } else {
                    summary
                };
                self.toast(msg);
            }
        }
        if running || self.tools.cd_reading.is_some() || self.tools.devices_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    pub(super) fn tool_dialogs(&mut self, ctx: &egui::Context) {
        let Some(tool) = self.tools.open else { return };
        let title = match tool {
            Tool::Convert => tr("Convertir"),
            Tool::Cd => tr("Extraire un CD"),
            Tool::Sync => tr("Synchroniser"),
        };
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new(("tool", tool as u8)))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .inner_margin(24.0)
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                ui.set_width(600.0);
                ui.label(RichText::new(title).font(theme::semibold(20.0)));
                ui.add_space(12.0);
                if self.tools.job.is_some() {
                    close = self.job_view(ui);
                    return;
                }
                match tool {
                    Tool::Convert => self.convert_form(ui),
                    Tool::Cd => self.cd_form(ui),
                    Tool::Sync => self.sync_form(ui),
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (label, ready) = match tool {
                            Tool::Convert => (tr("Convertir"), !self.tools.convert_tracks.is_empty()),
                            Tool::Cd => (tr("Extraire"), !self.tools.cd_tracks.is_empty()),
                            Tool::Sync => (
                                tr("Synchroniser"),
                                self.tools.sync_dest.is_some() && !self.tools.sync_lists.is_empty(),
                            ),
                        };
                        let go = ui
                            .add_enabled_ui(ready, |ui| widgets::primary_button(ui, None, label))
                            .inner
                            .clicked();
                        if go {
                            match tool {
                                Tool::Convert => self.start_convert(ui.ctx()),
                                Tool::Cd => self.start_rip(ui.ctx()),
                                Tool::Sync => self.start_sync(ui.ctx()),
                            }
                        }
                        if widgets::secondary_button(ui, None, tr("Fermer")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if modal.should_close() && !self.tools.busy() {
            close = true;
        }
        if close {
            self.tools.open = None;
        }
    }

    /// Avancement d'un travail ; vrai quand on ferme la fenêtre.
    fn job_view(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(job) = &self.tools.job else { return true };
        let s = job.status.lock().unwrap();
        let frac = if s.total > 0 {
            s.done as f32 / s.total as f32
        } else {
            0.0
        };
        ui.add(
            egui::ProgressBar::new(frac)
                .desired_width(600.0)
                .show_percentage(),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(trf!("{0} / {1}", s.done, s.total)).color(theme::TEXT_2));
        if !s.current.is_empty() {
            ui.label(RichText::new(&s.current).small().color(theme::TEXT_3));
        }
        if !s.summary.is_empty() {
            ui.label(RichText::new(&s.summary).color(theme::accent_light()));
        }
        if !s.errors.is_empty() {
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                for e in &s.errors {
                    ui.label(
                        RichText::new(e)
                            .small()
                            .color(Color32::from_rgb(0xF0, 0x9A, 0x8A)),
                    );
                }
            });
        }
        ui.add_space(14.0);
        let finished = s.finished;
        drop(s);
        let stop = job.stop.clone();
        let mut close = false;
        ui.horizontal(|ui| {
            if finished {
                close = widgets::primary_button(ui, None, tr("Fermer")).clicked();
            } else if widgets::secondary_button(ui, Some(Icon::Stop), tr("Arrêter")).clicked() {
                stop.store(true, Ordering::Relaxed);
            }
        });
        if close {
            self.tools.job = None;
        }
        close
    }

    fn format_choice(ui: &mut egui::Ui, format: &mut Format) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for f in Format::ALL {
                if segment(ui, f.label(), *format == f).clicked() {
                    *format = f;
                }
            }
        });
    }

    fn dest_choice(ui: &mut egui::Ui, dest: &mut Option<PathBuf>, default: PathBuf) {
        let shown = dest.clone().unwrap_or(default);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Dossier")).color(theme::TEXT_2));
            let g = widgets::galley(
                ui,
                &shown.display().to_string(),
                FontId::proportional(13.0),
                theme::TEXT,
                380.0,
            );
            ui.label(g);
            if widgets::secondary_button(ui, Some(Icon::FolderOpen), tr("Changer…")).clicked() {
                if let Some(d) = rfd::FileDialog::new().set_directory(&shown).pick_folder() {
                    *dest = Some(d);
                }
            }
        });
    }

    fn convert_form(&mut self, ui: &mut egui::Ui) {
        let n = self.tools.convert_tracks.len();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if n == 0 {
                    tr("Aucun morceau choisi.").to_owned()
                } else {
                    plural(n, "morceau à convertir", "morceaux à convertir")
                })
                .color(theme::TEXT),
            );
            if widgets::secondary_button(ui, Some(Icon::Plus), tr("Choisir des fichiers…")).clicked() {
                if let Some(files) = rfd::FileDialog::new()
                    .add_filter(tr("Musique"), crate::library::AUDIO_EXTENSIONS)
                    .pick_files()
                {
                    self.tools.convert_tracks = files.iter().map(|f| crate::library::read_track(f)).collect();
                }
            }
        });
        ui.label(
            RichText::new(tr("Ou clic droit sur des morceaux, un album : « Convertir… »."))
                .small()
                .color(theme::TEXT_3),
        );
        ui.add_space(10.0);
        Self::format_choice(ui, &mut self.tools.format);
        ui.add_space(10.0);
        let default = music_dir().join(tr("Converti"));
        Self::dest_choice(ui, &mut self.tools.convert_dest, default);
        ui.label(
            RichText::new(tr(
                "Rangés en Artiste / Album / Morceau. Les originaux ne sont pas touchés.",
            ))
            .small()
            .color(theme::TEXT_3),
        );
    }

    fn start_convert(&mut self, ctx: &egui::Context) {
        let t = &self.tools;
        let dest = t
            .convert_dest
            .clone()
            .unwrap_or_else(|| music_dir().join(tr("Converti")));
        let format = t.format;
        let items = t
            .convert_tracks
            .iter()
            .map(|tr| Item {
                src: Source::of(tr),
                out: dest.join(convert::relative_path(tr, format.ext())),
                format: Some(format),
                tags: convert::tags_of(tr),
                label: format!("{} — {}", tr.artist_or_unknown(), tr.title),
            })
            .collect();
        let summary = trf!("Fichiers dans {0}", dest.display());
        self.tools.job = Some(start_job(items, Some(Box::new(move || Ok(summary))), ctx));
    }

    fn cd_form(&mut self, ui: &mut egui::Ui) {
        let t = &mut self.tools;
        if t.cd_reading.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(tr("Lecture du CD et recherche sur MusicBrainz…")).color(theme::TEXT_2),
                );
            });
            return;
        }
        if let Some(e) = &t.cd_error {
            ui.label(RichText::new(e).color(theme::TEXT_2));
            ui.add_space(6.0);
            if widgets::secondary_button(ui, Some(Icon::Refresh), tr("Réessayer")).clicked() {
                let ctx = ui.ctx().clone();
                self.read_cd(&ctx);
            }
            return;
        }
        if t.cd_tracks.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut t.cd_artist)
                    .desired_width(220.0)
                    .hint_text(tr("Artiste")),
            );
            ui.add(
                egui::TextEdit::singleline(&mut t.cd_album)
                    .desired_width(260.0)
                    .hint_text(tr("Album")),
            );
            ui.add(
                egui::TextEdit::singleline(&mut t.cd_year)
                    .desired_width(70.0)
                    .hint_text(tr("Année")),
            );
        });
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
            for (i, title) in t.cd_titles.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{:02}", i + 1)).color(theme::TEXT_3));
                    ui.add(egui::TextEdit::singleline(title).desired_width(440.0));
                    if let Some((a, b)) = t.cd_tracks.get(i) {
                        ui.label(
                            RichText::new(crate::library::fmt_duration(b.saturating_sub(*a)))
                                .small()
                                .color(theme::TEXT_3),
                        );
                    }
                });
            }
        });
        ui.add_space(8.0);
        Self::format_choice(ui, &mut t.format);
        ui.add_space(8.0);
        let default = self
            .settings
            .music_dirs
            .first()
            .cloned()
            .unwrap_or_else(music_dir);
        Self::dest_choice(ui, &mut self.tools.cd_dest, default);
    }

    fn start_rip(&mut self, ctx: &egui::Context) {
        let t = &self.tools;
        let Some(drive) = t.cd_drive.clone() else { return };
        let dest = t
            .cd_dest
            .clone()
            .or_else(|| self.settings.music_dirs.first().cloned())
            .unwrap_or_else(music_dir);
        let format = t.format;
        let items = t
            .cd_tracks
            .iter()
            .enumerate()
            .map(|(i, (start, end))| {
                let track = Track {
                    path: PathBuf::new(),
                    title: t.cd_titles.get(i).cloned().unwrap_or_default(),
                    artist: t.cd_artist.clone(),
                    album: t.cd_album.clone(),
                    album_artist: t.cd_artist.clone(),
                    genre: String::new(),
                    year: t.cd_year.trim().parse().ok(),
                    track_no: Some(i as u32 + 1),
                    disc_no: None,
                    duration_ms: 0,
                    mtime: 0,
                    composer: String::new(),
                    schema: 0,
                    cue: None,
                    rating: None,
                    remote: None,
                };
                Item {
                    src: Source {
                        input: drive.to_string_lossy().into_owned(),
                        format: Some("libcdio"),
                        start: Some(*start),
                        end: Some(*end),
                    },
                    out: dest.join(convert::relative_path(&track, format.ext())),
                    format: Some(format),
                    tags: convert::tags_of(&track),
                    label: format!("{:02} — {}", i + 1, track.title),
                }
            })
            .collect();
        let summary = trf!("CD copié dans {0}", dest.display());
        self.tools.job = Some(start_job(items, Some(Box::new(move || Ok(summary))), ctx));
    }

    fn sync_form(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(tr("Playlists à copier")).color(theme::TEXT_2));
        let mut ids = vec![PlaylistId::Favorites];
        ids.extend((0..self.settings.smart_playlists.len()).map(PlaylistId::Smart));
        ids.extend((0..self.settings.playlists.len()).map(PlaylistId::User));
        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
            for id in ids {
                let name = self.playlist_name(id);
                let n = self.playlist_tracks(id).len();
                let key = format!("{id:?}");
                let mut on = self.tools.sync_lists.contains(&key);
                if ui
                    .checkbox(&mut on, format!("{name} · {}", plural(n, "morceau", "morceaux")))
                    .changed()
                {
                    if on {
                        self.tools.sync_lists.insert(key);
                    } else {
                        self.tools.sync_lists.remove(&key);
                    }
                }
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new(tr("Vers")).color(theme::TEXT_2));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            if let Some(Dest::Folder(p)) = &self.tools.sync_dest {
                let _ = segment(ui, &p.display().to_string(), true);
            }
            if widgets::secondary_button(ui, Some(Icon::FolderOpen), tr("Un dossier (clé, carte)…")).clicked()
            {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.tools.sync_dest = Some(Dest::Folder(d));
                }
            }
            for dev in self.tools.devices.clone() {
                let label = dev.trim_start_matches("mtp:/").to_owned();
                if segment(ui, &label, self.tools.sync_dest == Some(Dest::Mtp(dev.clone()))).clicked() {
                    self.tools.sync_dest = Some(Dest::Mtp(dev));
                }
            }
        });
        if self.tools.devices_rx.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(tr("Recherche des téléphones…"))
                        .small()
                        .color(theme::TEXT_3),
                );
            });
        } else if self.tools.devices.is_empty() {
            ui.label(
                RichText::new(tr("Téléphone : branche-le en USB, mode « Transfert de fichiers », puis rouvre cette fenêtre."))
                    .small()
                    .color(theme::TEXT_3),
            );
        }
        ui.add_space(10.0);
        ui.checkbox(
            &mut self.tools.sync_convert,
            tr("Convertir pour gagner de la place"),
        );
        if self.tools.sync_convert {
            Self::format_choice(ui, &mut self.tools.format);
        }
        ui.label(
            RichText::new(tr(
                "Les morceaux déjà présents ne sont pas recopiés ; chaque playlist est aussi écrite en .m3u.",
            ))
            .small()
            .color(theme::TEXT_3),
        );
    }

    fn start_sync(&mut self, ctx: &egui::Context) {
        let Some(dest) = self.tools.sync_dest.clone() else {
            return;
        };
        let format = self.tools.sync_convert.then_some(self.tools.format);
        // Un téléphone (MTP) reçoit le tout en une copie, préparée dans un dossier de passage.
        let root = match &dest {
            Dest::Folder(p) => p.join("SterMusic"),
            Dest::Mtp(_) => crate::library::cache_dir().join("sync").join("SterMusic"),
        };
        let mut lists: Vec<(String, Vec<usize>)> = Vec::new();
        let mut ids = vec![PlaylistId::Favorites];
        ids.extend((0..self.settings.smart_playlists.len()).map(PlaylistId::Smart));
        ids.extend((0..self.settings.playlists.len()).map(PlaylistId::User));
        for id in ids {
            if self.tools.sync_lists.contains(&format!("{id:?}")) {
                lists.push((self.playlist_name(id), self.playlist_tracks(id)));
            }
        }
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        let mut m3u: Vec<(String, Vec<PathBuf>)> = Vec::new();
        for (name, tracks) in &lists {
            let mut entries = Vec::new();
            for &i in tracks {
                let t = &self.lib.tracks[i];
                // Copie telle quelle d'un vrai fichier ; sinon (CUE, serveur) conversion.
                let fmt = format.or((!t.is_plain_file()).then_some(Format::Mp3));
                let ext = match fmt {
                    Some(f) => f.ext().to_owned(),
                    None => t
                        .path
                        .extension()
                        .map(|e| e.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "mp3".into()),
                };
                let rel = convert::relative_path(t, &ext);
                entries.push(rel.clone());
                let out = root.join(&rel);
                if !seen.insert(rel) || out.exists() {
                    continue;
                }
                items.push(Item {
                    src: Source::of(t),
                    out,
                    format: fmt,
                    tags: convert::tags_of(t),
                    label: format!("{} — {}", t.artist_or_unknown(), t.title),
                });
            }
            m3u.push((name.clone(), entries));
        }
        let root2 = root.clone();
        let after = move || -> Result<String, String> {
            for (name, entries) in &m3u {
                let mut text = String::from("#EXTM3U\n");
                for e in entries {
                    text.push_str(&e.to_string_lossy());
                    text.push('\n');
                }
                let _ = std::fs::create_dir_all(&root2);
                std::fs::write(root2.join(format!("{}.m3u", convert::safe_name(name))), text)
                    .map_err(|e| e.to_string())?;
            }
            match dest {
                Dest::Folder(_) => Ok(trf!("Synchronisé dans {0}", root2.display())),
                Dest::Mtp(device) => {
                    let target = format!("{device}/Music/");
                    let ok = std::process::Command::new("kioclient")
                        .args(["--overwrite", "--noninteractive", "copy"])
                        .arg(&root2)
                        .arg(&target)
                        .status()
                        .is_ok_and(|s| s.success());
                    if ok {
                        Ok(trf!("Copié sur {0}", target))
                    } else {
                        Err(
                            crate::i18n::tr("La copie vers le téléphone a échoué (est-il déverrouillé ?)")
                                .into(),
                        )
                    }
                }
            }
        };
        self.tools.job = Some(start_job(items, Some(Box::new(after)), ctx));
    }
}

/// Carte d'un outil ; vrai si on a cliqué dessus.
fn tool_card(ui: &mut egui::Ui, icon: Icon, title: &str, desc: &str, w: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 96.0), Sense::click());
    let h = widgets::hover_t(ui, resp.id, resp.hovered());
    let r = rect.translate(vec2(0.0, -2.0 * h));
    ui.painter().rect_filled(
        r,
        CornerRadius::same(12),
        theme::white((0x0A as f32 + 8.0 * h) as u8),
    );
    ui.painter().rect_stroke(
        r,
        CornerRadius::same(12),
        egui::Stroke::new(1.0_f32, theme::white(0x12)),
        egui::StrokeKind::Inside,
    );
    let badge = Rect::from_min_size(r.min + vec2(16.0, 18.0), vec2(40.0, 40.0));
    theme::brand_fill(ui.painter(), badge, CornerRadius::same(12), 0.85 + 0.15 * h);
    icons::paint_centered(ui.painter(), icon, badge.center(), 20.0, Color32::WHITE);
    let x = badge.right() + 14.0;
    let tw = r.right() - x - 14.0;
    let g = widgets::galley(ui, title, theme::semibold(15.0), theme::TEXT, tw);
    ui.painter().galley(pos2(x, r.top() + 18.0), g, theme::TEXT);
    let job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: tw,
            max_rows: 2,
            break_anywhere: false,
            overflow_character: Some('…'),
        },
        ..egui::text::LayoutJob::single_section(
            desc.to_owned(),
            egui::TextFormat::simple(FontId::proportional(12.5), theme::TEXT_2),
        )
    };
    let g = ui.fonts(|f| f.layout_job(job));
    ui.painter().galley(pos2(x, r.top() + 42.0), g, theme::TEXT_2);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
