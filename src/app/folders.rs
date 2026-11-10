//! Vue par dossiers : la bibliothèque telle qu'elle est rangée sur le disque. Plus parlante que
//! la vue Albums pour des fichiers mal tagués (« ## Personnel/Papa »…).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use eframe::egui::{self, pos2, vec2, FontId, Sense};

use super::lists::{self, ListOpts};
use super::{Action, SterMusic};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::{fold, plural, Library};
use crate::{theme, widgets};

/// Contenu d'un dossier.
#[derive(Default)]
pub struct Folder {
    /// Sous-dossiers (chemin complet), triés par nom.
    pub children: Vec<PathBuf>,
    /// Morceaux posés directement dans ce dossier, dans l'ordre de la bibliothèque.
    pub tracks: Vec<usize>,
    /// Nombre de morceaux, sous-dossiers compris.
    pub total: usize,
    /// Un morceau du dossier ou d'un sous-dossier : sa pochette illustre le dossier.
    pub first: Option<usize>,
}

#[derive(Default)]
pub struct FolderIndex {
    pub folders: HashMap<PathBuf, Folder>,
    pub roots: Vec<PathBuf>,
}

impl FolderIndex {
    pub fn build(lib: &Library, roots: &[PathBuf]) -> Self {
        let mut folders: HashMap<PathBuf, Folder> = HashMap::new();
        let mut children: HashMap<PathBuf, BTreeMap<String, PathBuf>> = HashMap::new();
        for (i, t) in lib.tracks.iter().enumerate() {
            let Some(root) = roots.iter().find(|r| t.path.starts_with(r)) else {
                continue;
            };
            let Some(dir) = t.path.parent() else { continue };
            folders.entry(dir.to_path_buf()).or_default().tracks.push(i);
            // Chaque dossier, jusqu'à la racine, compte le morceau et connaît son sous-dossier.
            let mut cur = dir;
            loop {
                let f = folders.entry(cur.to_path_buf()).or_default();
                f.total += 1;
                f.first.get_or_insert(i);
                if cur == root.as_path() {
                    break;
                }
                let Some(parent) = cur.parent() else { break };
                let name = cur
                    .file_name()
                    .map(|n| fold(&n.to_string_lossy()))
                    .unwrap_or_default();
                children
                    .entry(parent.to_path_buf())
                    .or_default()
                    .insert(format!("{name}\u{0}{}", cur.display()), cur.to_path_buf());
                cur = parent;
            }
        }
        for (dir, kids) in children {
            folders.entry(dir).or_default().children = kids.into_values().collect();
        }
        let roots = roots
            .iter()
            .filter(|r| folders.contains_key(*r))
            .cloned()
            .collect();
        Self { folders, roots }
    }
}

/// Nombre de morceaux et de sous-dossiers d'un dossier, et le morceau qui l'illustre.
type FolderInfo = (usize, usize, Option<usize>);

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

impl SterMusic {
    fn folder_index(&mut self) -> &FolderIndex {
        if self.folder_index.is_none() {
            self.folder_index = Some(FolderIndex::build(&self.lib, &self.settings.music_dirs));
        }
        self.folder_index.as_ref().unwrap()
    }

    /// Tous les morceaux d'un dossier, sous-dossiers compris, dans l'ordre de la bibliothèque.
    fn folder_tracks(&self, dir: &Path) -> Vec<usize> {
        (0..self.lib.tracks.len())
            .filter(|&i| self.lib.tracks[i].path.starts_with(dir))
            .collect()
    }

    pub(super) fn folders_tab(&mut self, ui: &mut egui::Ui, query: &str) {
        // Un dossier qui n'existe plus (rescan, dossier retiré) : retour aux racines.
        let current = self
            .lib_folder
            .clone()
            .filter(|d| self.folder_index().folders.contains_key(d));
        self.lib_folder = current.clone();
        let index = self.folder_index();
        let (kids, tracks): (Vec<PathBuf>, Vec<usize>) = match &current {
            None => (index.roots.clone(), Vec::new()),
            Some(d) => {
                let f = &index.folders[d];
                (f.children.clone(), f.tracks.clone())
            }
        };
        // Pour chaque sous-dossier : nombre de morceaux, de sous-dossiers, et sa pochette.
        let info: Vec<FolderInfo> = kids
            .iter()
            .map(|k| {
                index
                    .folders
                    .get(k)
                    .map(|f| (f.total, f.children.len(), f.first))
                    .unwrap_or_default()
            })
            .collect();
        let roots = index.roots.clone();

        // Fil d'Ariane à gauche, actions du dossier à droite.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if widgets::link(ui, tr("Tous les dossiers")).clicked() {
                self.lib_folder = None;
            }
            if let Some(d) = &current {
                if let Some(root) = roots.iter().find(|r| d.starts_with(r)) {
                    let mut chain = vec![d.clone()];
                    let mut cur = d.as_path();
                    while cur != root.as_path() {
                        match cur.parent() {
                            Some(p) => {
                                chain.push(p.to_path_buf());
                                cur = p;
                            }
                            None => break,
                        }
                    }
                    for p in chain.into_iter().rev() {
                        if widgets::link(ui, &name_of(&p)).clicked() {
                            self.lib_folder = Some(p);
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add_space(12.0);
                    if widgets::icon_button(ui, Icon::FolderOpen, tr("Ouvrir le dossier"), 32.0, 16.0)
                        .clicked()
                    {
                        self.actions.push(Action::OpenFolder(d.clone()));
                    }
                    if widgets::icon_button(ui, Icon::Tag, tr("Modifier les infos…"), 32.0, 16.0).clicked()
                    {
                        self.actions.push(Action::EditTags(vec![d.clone()]));
                    }
                    if widgets::icon_button(ui, Icon::Shuffle, tr("Lecture aléatoire"), 32.0, 16.0).clicked()
                    {
                        self.actions.push(Action::PlayShuffled(self.folder_tracks(d)));
                    }
                    if widgets::icon_button(ui, Icon::Play, tr("Lire le dossier"), 32.0, 16.0).clicked() {
                        self.actions.push(Action::Play {
                            tracks: self.folder_tracks(d),
                            start: 0,
                        });
                    }
                });
            }
        });
        ui.add_space(8.0);

        let words: Vec<String> = fold(query).split_whitespace().map(str::to_owned).collect();
        let keep = |name: &str| words.iter().all(|w| fold(name).contains(w.as_str()));
        let folders: Vec<(PathBuf, FolderInfo)> = kids
            .into_iter()
            .zip(info)
            .filter(|(k, _)| keep(&name_of(k)))
            .collect();
        let tracks = if words.is_empty() {
            tracks
        } else {
            self.lib.filter_tracks(&tracks, query)
        };
        if folders.is_empty() && tracks.is_empty() {
            Self::no_match(ui);
            return;
        }

        // Une seule liste, comme un gestionnaire de fichiers : les sous-dossiers, puis les morceaux.
        let opts = ListOpts {
            cover: true,
            album: true,
            ..Default::default()
        };
        if !tracks.is_empty() {
            lists::header(ui, &opts);
        }
        let n_folders = folders.len();
        let mut open = None;
        let mut menu: Option<(PathBuf, &'static str)> = None;
        let names: Vec<String> = folders
            .iter()
            .map(|(d, _)| {
                if current.is_none() {
                    d.display().to_string()
                } else {
                    name_of(d)
                }
            })
            .collect();
        let thumbs: Vec<Option<PathBuf>> = folders
            .iter()
            .map(|(_, (_, _, first))| first.map(|i| self.lib.tracks[i].path.clone()))
            .collect();
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical()
            .id_salt(("folder_view", current.clone()))
            .auto_shrink(false)
            .show_rows(ui, lists::ROW_H, n_folders + tracks.len(), |ui, range| {
                for row in range {
                    if row >= n_folders {
                        let mut c = self.view_ctx();
                        lists::track_row(ui, &mut c, &tracks, row - n_folders, &opts);
                        continue;
                    }
                    let (dir, (total, subfolders, _)) = &folders[row];
                    let (r, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), lists::ROW_H), Sense::click());
                    if row % 2 == 1 {
                        ui.painter().rect_filled(r, 6, theme::white(0x03));
                    }
                    widgets::paint_interaction(ui, r, 6, &resp);
                    // La pochette d'un de ses morceaux, et un petit dossier en coin.
                    let art = egui::Rect::from_min_size(
                        pos2(r.left() + 12.0, r.center().y - 18.0),
                        vec2(36.0, 36.0),
                    );
                    let tex = thumbs[row].as_ref().and_then(|p| self.covers.thumb(p).cloned());
                    widgets::cover(ui.painter(), art, tex.as_ref(), 5, &names[row]);
                    let badge =
                        egui::Rect::from_center_size(art.right_bottom() - vec2(3.0, 3.0), vec2(18.0, 18.0));
                    ui.painter().rect_filled(badge, 5, theme::glass());
                    icons::paint_centered(
                        ui.painter(),
                        Icon::FolderOpen,
                        badge.center(),
                        12.0,
                        theme::accent_light(),
                    );
                    let x = r.left() + 60.0;
                    let g = widgets::galley(
                        ui,
                        &names[row],
                        theme::semibold(14.0),
                        theme::TEXT,
                        r.width() - 300.0,
                    );
                    ui.painter().galley(pos2(x, r.top() + 7.0), g, theme::TEXT);
                    let detail = if *subfolders > 0 {
                        format!(
                            "{} · {}",
                            plural(*subfolders, "dossier", "dossiers"),
                            plural(*total, "morceau", "morceaux")
                        )
                    } else {
                        plural(*total, "morceau", "morceaux")
                    };
                    let g = widgets::galley(
                        ui,
                        &detail,
                        FontId::proportional(12.5),
                        theme::TEXT_3,
                        r.width() - 300.0,
                    );
                    ui.painter().galley(pos2(x, r.top() + 27.0), g, theme::TEXT_3);
                    icons::paint_centered(
                        ui.painter(),
                        Icon::ChevronRight,
                        pos2(r.right() - 22.0, r.center().y),
                        16.0,
                        theme::TEXT_3,
                    );
                    if resp.clicked() {
                        open = Some(dir.clone());
                    }
                    resp.context_menu(|ui| {
                        for (icon, label, what) in [
                            (Icon::PlayOutline, tr("Lire"), "play"),
                            (Icon::Shuffle, tr("Lecture aléatoire"), "shuffle"),
                            (Icon::ListEnd, tr("Ajouter à la file d'attente"), "queue"),
                            (Icon::Tag, tr("Modifier les infos…"), "tags"),
                            (Icon::FolderOpen, tr("Ouvrir le dossier"), "open"),
                        ] {
                            if widgets::menu_item(ui, Some(icon), label).clicked() {
                                menu = Some((dir.clone(), what));
                            }
                        }
                    });
                }
                ui.add_space(16.0);
            });
        if let Some(dir) = open {
            self.lib_folder = Some(dir);
        }
        if let Some((dir, what)) = menu {
            let all = self.folder_tracks(&dir);
            self.actions.push(match what {
                "play" => Action::Play {
                    tracks: all,
                    start: 0,
                },
                "shuffle" => Action::PlayShuffled(all),
                "queue" => Action::Enqueue(all),
                "tags" => Action::EditTags(vec![dir]),
                _ => Action::OpenFolder(dir),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Track;

    #[test]
    fn index() {
        let t = |p: &str| Track {
            path: PathBuf::from(p),
            title: p.into(),
            artist: String::new(),
            album: String::new(),
            album_artist: String::new(),
            genre: String::new(),
            year: None,
            track_no: None,
            disc_no: None,
            duration_ms: 0,
            mtime: 0,
            composer: String::new(),
            schema: 0,
        };
        let lib = Library::new(vec![
            t("/m/## Personnel/Papa/a.mp3"),
            t("/m/## Personnel/Papa/b.mp3"),
            t("/m/## Personnel/c.mp3"),
            t("/m/Zèbre/d.mp3"),
            t("/m/Album/e.mp3"),
            t("/ailleurs/f.mp3"),
        ]);
        let idx = FolderIndex::build(&lib, &[PathBuf::from("/m")]);
        assert_eq!(idx.roots, vec![PathBuf::from("/m")]);
        let root = &idx.folders[Path::new("/m")];
        assert_eq!(root.total, 5);
        // Tri sans tenir compte des accents ni des majuscules.
        assert_eq!(
            root.children,
            vec![
                PathBuf::from("/m/## Personnel"),
                PathBuf::from("/m/Album"),
                PathBuf::from("/m/Zèbre")
            ]
        );
        let perso = &idx.folders[Path::new("/m/## Personnel")];
        assert_eq!(perso.total, 3);
        assert_eq!(perso.tracks.len(), 1);
        assert_eq!(perso.children, vec![PathBuf::from("/m/## Personnel/Papa")]);
        assert!(!idx.folders.contains_key(Path::new("/ailleurs")));
    }
}
