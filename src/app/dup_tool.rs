//! Doublons : les morceaux de la bibliothèque qui ont le même titre, le même artiste et la même
//! durée (à deux secondes près), donc très probablement le même enregistrement en plusieurs
//! exemplaires. On compare les exemplaires (format, taille, débit), on peut les écouter, puis
//! envoyer ceux qu'on coche à la corbeille : rien n'est supprimé définitivement.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use eframe::egui::{self, pos2, vec2, Align, Align2, CornerRadius, FontId, Layout, Rect, Sense};

use super::lib_tools;
use super::tag_editor::{file_name, size_text};
use super::{Action, SterMusic};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::library::{self, fmt_duration, fold, plural, Track};
use crate::trf;
use crate::{theme, widgets};

/// Écart de durée toléré entre deux exemplaires du même enregistrement.
const TOLERANCE_MS: u64 = 2000;

struct Dup {
    path: PathBuf,
    album: String,
    duration_ms: u64,
    size: u64,
    /// À envoyer à la corbeille.
    marked: bool,
}

impl Dup {
    fn extension(&self) -> String {
        self.path
            .extension()
            .map(|e| e.to_string_lossy().to_uppercase())
            .unwrap_or_default()
    }

    fn lossless(&self) -> bool {
        matches!(self.extension().as_str(), "FLAC" | "WAV" | "AIFF" | "AIF")
    }

    /// Débit moyen, en kbit/s.
    fn bitrate(&self) -> u64 {
        (self.size * 8).checked_div(self.duration_ms).unwrap_or(0)
    }

    /// Qualité, pour désigner le meilleur exemplaire : sans perte d'abord, puis le débit.
    fn quality(&self) -> (bool, u64, u64) {
        (self.lossless(), self.bitrate(), self.size)
    }
}

struct Group {
    title: String,
    artist: String,
    copies: Vec<Dup>,
}

impl Group {
    fn best(&self) -> usize {
        (0..self.copies.len())
            .max_by_key(|&i| self.copies[i].quality())
            .unwrap_or(0)
    }

    fn keep_best(&mut self) {
        let best = self.best();
        for (i, c) in self.copies.iter_mut().enumerate() {
            c.marked = i != best;
        }
    }
}

struct Trashing {
    rx: Receiver<(Vec<PathBuf>, Vec<String>)>,
    total: usize,
}

#[derive(Default)]
pub struct DupTool {
    scan: Option<Receiver<Vec<Group>>>,
    groups: Vec<Group>,
    /// Le bouton de mise à la corbeille attend sa confirmation.
    confirm: bool,
    trashing: Option<Trashing>,
}

impl DupTool {
    pub fn busy(&self) -> bool {
        self.scan.is_some() || self.trashing.is_some()
    }

    fn marked(&self) -> impl Iterator<Item = &Dup> {
        self.groups
            .iter()
            .flat_map(|g| g.copies.iter())
            .filter(|c| c.marked)
    }
}

/// Clé de comparaison : minuscules, sans accents, lettres et chiffres seulement.
fn key(text: &str) -> String {
    fold(text).chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Groupes de morceaux (par leurs indices) qui ont même titre, même artiste et même durée.
fn find_groups(tracks: &[Track]) -> Vec<Vec<usize>> {
    let mut by_name: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, t) in tracks.iter().enumerate() {
        let title = key(&t.title);
        if !title.is_empty() {
            by_name.entry((title, key(&t.artist))).or_default().push(i);
        }
    }
    let mut groups = Vec::new();
    for mut same in by_name.into_values().filter(|v| v.len() > 1) {
        // Triés par durée : deux voisins à moins de deux secondes d'écart vont ensemble.
        same.sort_by_key(|&i| tracks[i].duration_ms);
        let mut run = vec![same[0]];
        for pair in same.windows(2) {
            if tracks[pair[1]].duration_ms - tracks[pair[0]].duration_ms > TOLERANCE_MS {
                if run.len() > 1 {
                    groups.push(std::mem::take(&mut run));
                }
                run.clear();
            }
            run.push(pair[1]);
        }
        if run.len() > 1 {
            groups.push(run);
        }
    }
    groups.sort_by_cached_key(|g| {
        let t = &tracks[g[0]];
        (fold(&t.artist), fold(&t.title), t.duration_ms)
    });
    groups
}

/// Envoie les fichiers à la corbeille du bureau ; renvoie ceux qui y sont, et les erreurs.
fn trash(paths: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<String>) {
    let (mut gone, mut errors) = (Vec::new(), Vec::new());
    for path in paths {
        let done = std::process::Command::new("gio")
            .arg("trash")
            .arg("--")
            .arg(&path)
            .output();
        match done {
            Ok(out) if out.status.success() => gone.push(path),
            Ok(out) => {
                let why = String::from_utf8_lossy(&out.stderr);
                let why = why.lines().last().unwrap_or(tr("refusé")).trim().to_owned();
                errors.push(format!("{} ({why})", file_name(&path)));
            }
            Err(_) => {
                errors.push(tr("la commande « gio » est introuvable (paquet libglib2.0-bin)").to_owned());
                break;
            }
        }
    }
    (gone, errors)
}

impl SterMusic {
    pub(super) fn start_dup_scan(&mut self, ctx: &egui::Context) {
        let tool = &mut self.lib_tools.dups;
        if tool.busy() {
            return;
        }
        tool.groups.clear();
        tool.confirm = false;
        // Doublons à mettre à la corbeille : de vrais fichiers seulement.
        let tracks: Vec<_> = self.lib.tracks.iter().filter(|t| t.is_plain_file()).cloned().collect();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let groups = find_groups(&tracks)
                .into_iter()
                .map(|g| {
                    let first = &tracks[g[0]];
                    Group {
                        title: first.title.clone(),
                        artist: first.artist_or_unknown().to_owned(),
                        copies: g
                            .iter()
                            .map(|&i| {
                                let t = &tracks[i];
                                Dup {
                                    path: t.path.clone(),
                                    album: t.album_or_unknown().to_owned(),
                                    duration_ms: t.duration_ms,
                                    size: std::fs::metadata(&t.path).map(|m| m.len()).unwrap_or(0),
                                    marked: false,
                                }
                            })
                            .collect(),
                    }
                })
                .collect();
            let _ = tx.send(groups);
            ctx.request_repaint();
        });
        tool.scan = Some(rx);
    }

    pub(super) fn poll_duplicates(&mut self, _ctx: &egui::Context) {
        let tool = &mut self.lib_tools.dups;
        if let Some(groups) = tool.scan.as_ref().and_then(|rx| rx.try_recv().ok()) {
            tool.groups = groups;
            tool.scan = None;
        }
        let Some((gone, errors)) = tool.trashing.as_ref().and_then(|t| t.rx.try_recv().ok()) else {
            return;
        };
        tool.trashing = None;
        let gone: HashSet<PathBuf> = gone.into_iter().collect();
        for g in &mut tool.groups {
            g.copies.retain(|c| !gone.contains(&c.path));
        }
        // Un exemplaire resté seul n'est plus un doublon.
        tool.groups.retain(|g| g.copies.len() > 1);
        if !gone.is_empty() {
            // La bibliothèque, l'éditeur de tags et le cache oublient les fichiers partis.
            let tracks: Vec<Track> = self
                .lib
                .tracks
                .iter()
                .filter(|t| !gone.contains(&t.path))
                .cloned()
                .collect();
            library::save_cache(&tracks);
            self.set_library(tracks);
            self.tag_editor.remove_paths(&gone);
            let gone: Vec<PathBuf> = gone.iter().cloned().collect();
            self.covers.forget(&gone);
        }
        let ok = plural(
            gone.len(),
            "fichier mis à la corbeille",
            "fichiers mis à la corbeille",
        );
        match errors.first() {
            None => self.toast(ok),
            Some(first) => self.toast(trf!(
                "{ok}, {0} : {first}",
                ok,
                plural(errors.len(), "échec", "échecs"),
                first
            )),
        }
    }

    pub(super) fn duplicates_dialog(&mut self, ctx: &egui::Context) {
        let lib = &self.lib;
        let actions = &mut self.actions;
        let tool = &mut self.lib_tools.dups;
        let (mut close, mut send) = (false, false);
        let width = (ctx.screen_rect().width() - 100.0).clamp(640.0, 860.0);
        let list_h = (ctx.screen_rect().height() - 270.0).clamp(160.0, 480.0);
        let modal = crate::widgets::modal(ctx, egui::Id::new("duplicates"))
            .frame(lib_tools::modal_frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(width);
                let copies: usize = tool.groups.iter().map(|g| g.copies.len()).sum();
                let detail = if tool.scan.is_some() || tool.groups.is_empty() {
                    String::new()
                } else {
                    format!(
                        "{} · {}",
                        plural(tool.groups.len(), "morceau en double", "morceaux en double"),
                        plural(copies, "fichier", "fichiers")
                    )
                };
                lib_tools::title(ui, Icon::WinRestore, tr("Doublons"), &detail);
                ui.add_space(12.0);
                if tool.scan.is_some() {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(16.0).color(theme::accent_light()));
                        ui.label(egui::RichText::new(tr("Comparaison des morceaux…")).color(theme::TEXT));
                    });
                } else if tool.groups.is_empty() {
                    ui.label(
                        egui::RichText::new(tr(
                            "Aucun doublon : aucun morceau n'apparaît deux fois avec le même titre, \
                             le même artiste et la même durée.",
                        ))
                        .color(theme::TEXT_2),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(tr(
                            "Même titre, même artiste, même durée à deux secondes près. Écoute et \
                             compare, puis coche les exemplaires à envoyer à la corbeille.",
                        ))
                        .color(theme::TEXT_2),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if widgets::link(ui, tr("Cocher tout sauf le meilleur exemplaire")).clicked() {
                            tool.groups.iter_mut().for_each(Group::keep_best);
                            tool.confirm = false;
                        }
                        if widgets::link(ui, tr("Tout décocher")).clicked() {
                            for c in tool.groups.iter_mut().flat_map(|g| g.copies.iter_mut()) {
                                c.marked = false;
                            }
                            tool.confirm = false;
                        }
                    });
                    ui.add_space(6.0);
                    if let Some(path) = group_list(ui, tool, width, list_h) {
                        if let Some(i) = lib.find(&path) {
                            actions.push(Action::Play {
                                tracks: vec![i],
                                start: 0,
                            });
                        }
                    }
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    let count = tool.marked().count();
                    let bytes: u64 = tool.marked().map(|c| c.size).sum();
                    if let Some(t) = &tool.trashing {
                        ui.add(egui::Spinner::new().size(16.0).color(theme::accent_light()));
                        ui.label(
                            egui::RichText::new(trf!(
                                "Mise à la corbeille de {0}…",
                                plural(t.total, "fichier", "fichiers")
                            ))
                            .color(theme::TEXT),
                        );
                    } else if tool.confirm {
                        ui.label(
                            egui::RichText::new(trf!(
                                "{0} ({1}) : ils resteront récupérables dans la corbeille.",
                                plural(
                                    count,
                                    "fichier part à la corbeille",
                                    "fichiers partent à la corbeille"
                                ),
                                size_text(bytes)
                            ))
                            .color(theme::TEXT),
                        );
                    } else if count > 0 {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} · {}",
                                plural(count, "fichier coché", "fichiers cochés"),
                                size_text(bytes)
                            ))
                            .color(theme::TEXT_2),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if tool.trashing.is_some() {
                            return;
                        }
                        if tool.confirm {
                            if widgets::primary_button(ui, Some(Icon::Trash), tr("Confirmer")).clicked() {
                                send = true;
                            }
                            if widgets::secondary_button(ui, None, tr("Annuler")).clicked() {
                                tool.confirm = false;
                            }
                            return;
                        }
                        ui.scope(|ui| {
                            if count == 0 {
                                ui.disable();
                                ui.multiply_opacity(0.5);
                            }
                            if widgets::primary_button(ui, Some(Icon::Trash), tr("Mettre à la corbeille…"))
                                .clicked()
                            {
                                tool.confirm = true;
                            }
                        });
                        if widgets::secondary_button(ui, None, tr("Fermer")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if send {
            let paths: Vec<PathBuf> = tool.marked().map(|c| c.path.clone()).collect();
            let (tx, rx) = channel();
            tool.trashing = Some(Trashing {
                rx,
                total: paths.len(),
            });
            tool.confirm = false;
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(trash(paths));
                ctx.request_repaint();
            });
        } else if close || modal.should_close() {
            tool.confirm = false;
            self.lib_tools.open = None;
        }
    }
}

/// Une ligne de la liste : le titre d'un groupe, ou l'un de ses exemplaires.
#[derive(Clone, Copy)]
enum Row {
    Head(usize),
    Copy(usize, usize),
}

/// Dossier d'un fichier, raccourci à ses deux derniers niveaux.
fn short_dir(path: &Path) -> String {
    let dir = path.parent().unwrap_or(path);
    let parts: Vec<String> = dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let tail = &parts[parts.len().saturating_sub(2)..];
    if parts.len() > 2 {
        format!("…/{}", tail.join("/"))
    } else {
        dir.display().to_string()
    }
}

/// Liste des groupes ; renvoie l'exemplaire qu'on demande à écouter.
fn group_list(ui: &mut egui::Ui, tool: &mut DupTool, width: f32, height: f32) -> Option<PathBuf> {
    const ROW: f32 = 44.0;
    let mut play = None;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        theme::white(0x05),
        egui::Stroke::new(1.0_f32, theme::white(0x0E)),
        egui::StrokeKind::Inside,
    );
    let rows: Vec<Row> = tool
        .groups
        .iter()
        .enumerate()
        .flat_map(|(g, group)| {
            std::iter::once(Row::Head(g)).chain((0..group.copies.len()).map(move |c| Row::Copy(g, c)))
        })
        .collect();
    let mut changed = false;
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(4.0)));
    inner.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt("duplicates")
        .auto_shrink(false)
        .show_rows(&mut inner, ROW, rows.len(), |ui, range| {
            for i in range {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                match rows[i] {
                    Row::Head(g) => {
                        let group = &mut tool.groups[g];
                        let y = r.bottom() - 14.0;
                        let text = format!("{} — {}", group.title, group.artist);
                        let galley =
                            widgets::galley(ui, &text, theme::semibold(14.0), theme::TEXT, r.width() - 230.0);
                        ui.painter().galley(
                            pos2(r.left() + 10.0, y - galley.size().y / 2.0),
                            galley,
                            theme::TEXT,
                        );
                        let link = Rect::from_min_max(
                            pos2(r.right() - 200.0, y - 13.0),
                            pos2(r.right() - 4.0, y + 13.0),
                        );
                        let mut link_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(link)
                                .layout(Layout::right_to_left(Align::Center)),
                        );
                        if widgets::link(&mut link_ui, tr("Garder le meilleur")).clicked() {
                            group.keep_best();
                            changed = true;
                        }
                    }
                    Row::Copy(g, c) => {
                        let best = tool.groups[g].best() == c;
                        let copy = &mut tool.groups[g].copies[c];
                        ui.painter().rect_filled(
                            r.shrink2(vec2(0.0, 1.0)),
                            CornerRadius::same(6),
                            if copy.marked {
                                theme::alpha(theme::DANGER, 0x26)
                            } else {
                                theme::white(0x05)
                            },
                        );
                        let check =
                            Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(22.0, 22.0));
                        if widgets::place(ui, check, |ui| {
                            ui.add(egui::Checkbox::without_text(&mut copy.marked))
                        })
                        .on_hover_text(tr("À envoyer à la corbeille"))
                        .changed()
                        {
                            changed = true;
                        }
                        let button =
                            Rect::from_center_size(pos2(r.left() + 52.0, r.center().y), vec2(30.0, 30.0));
                        let resp = widgets::place(ui, button, |ui| {
                            widgets::icon_button(
                                ui,
                                Icon::PlayOutline,
                                tr("Écouter cet exemplaire"),
                                30.0,
                                15.0,
                            )
                        });
                        if resp.clicked() {
                            play = Some(copy.path.clone());
                        }
                        let x = r.left() + 76.0;
                        let info = format!(
                            "{} · {} · {} kb/s · {}",
                            copy.extension(),
                            size_text(copy.size),
                            copy.bitrate(),
                            fmt_duration(Duration::from_millis(copy.duration_ms))
                        );
                        let info_w = 250.0;
                        let tw = r.width() - 76.0 - info_w - 12.0;
                        let name = file_name(&copy.path);
                        let galley = widgets::galley(ui, &name, FontId::proportional(13.0), theme::TEXT, tw);
                        ui.painter().galley(pos2(x, r.top() + 6.0), galley, theme::TEXT);
                        let place = format!("{} · {}", short_dir(&copy.path), copy.album);
                        let galley =
                            widgets::galley(ui, &place, FontId::proportional(11.5), theme::TEXT_3, tw);
                        ui.painter()
                            .galley(pos2(x, r.top() + 24.0), galley, theme::TEXT_3);
                        ui.painter().text(
                            pos2(r.right() - 10.0, r.top() + 14.0),
                            Align2::RIGHT_CENTER,
                            info,
                            FontId::proportional(12.5),
                            theme::TEXT_2,
                        );
                        if best {
                            ui.painter().text(
                                pos2(r.right() - 10.0, r.top() + 31.0),
                                Align2::RIGHT_CENTER,
                                tr("meilleure qualité"),
                                FontId::proportional(11.5),
                                theme::accent_light(),
                            );
                        }
                        // Le chemin complet au survol de la ligne.
                        let hover = Rect::from_min_max(pos2(x, r.top()), r.max);
                        ui.interact(hover, ui.id().with(("dup_row", i)), Sense::hover())
                            .on_hover_text(copy.path.display().to_string());
                    }
                }
            }
        });
    if changed {
        tool.confirm = false;
    }
    play
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(path: &str, title: &str, artist: &str, secs: u64) -> Track {
        Track {
            path: PathBuf::from(path),
            title: title.into(),
            artist: artist.into(),
            album: String::new(),
            album_artist: String::new(),
            genre: String::new(),
            year: None,
            track_no: None,
            disc_no: None,
            duration_ms: secs * 1000,
            mtime: 0,
            composer: String::new(),
            schema: 0,
            cue: None,
            rating: None,
            remote: None,
        }
    }

    #[test]
    fn groups() {
        let tracks = vec![
            track("/a/1.mp3", "Été indien", "Joe Dassin", 270),
            track("/b/1.flac", "ete indien!", "JOE DASSIN", 271),
            // Même titre, autre durée : une autre version, pas un doublon.
            track("/c/live.mp3", "Été indien", "Joe Dassin", 330),
            // Même titre, autre artiste.
            track("/d/1.mp3", "Été indien", "Reprise", 270),
            track("/e/1.mp3", "Seul", "Quelqu'un", 100),
            // Trois exemplaires qui se suivent à moins de deux secondes.
            track("/f/1.mp3", "Trio", "X", 100),
            track("/f/2.mp3", "Trio", "X", 101),
            track("/f/3.mp3", "Trio", "X", 103),
        ];
        let found = find_groups(&tracks);
        assert_eq!(found, vec![vec![0, 1], vec![5, 6, 7]]);
    }

    #[test]
    fn best_copy() {
        let copy = |path: &str, size: u64| Dup {
            path: PathBuf::from(path),
            album: String::new(),
            duration_ms: 200_000,
            size,
            marked: false,
        };
        let mut g = Group {
            title: String::new(),
            artist: String::new(),
            copies: vec![
                copy("a.mp3", 8_000_000),
                copy("b.flac", 6_000_000),
                copy("c.mp3", 3_000_000),
            ],
        };
        // Le FLAC l'emporte même plus petit ; entre deux MP3, le plus gros débit.
        assert_eq!(g.best(), 1);
        g.keep_best();
        assert_eq!(
            g.copies.iter().map(|c| c.marked).collect::<Vec<_>>(),
            [true, false, true]
        );
        g.copies.remove(1);
        assert_eq!(g.best(), 0);
        assert_eq!(
            short_dir(Path::new("/run/media/x/Musique/Album/t.mp3")),
            "…/Musique/Album"
        );
    }
}
