//! Éditeur de tags, à la Mp3tag, en page dédiée : une liste de fichiers (de la bibliothèque ou
//! d'ailleurs) à droite, les champs de la sélection et sa pochette à gauche.
//!
//! Les champs qui diffèrent d'un fichier sélectionné à l'autre restent vides (« valeurs
//! multiples ») et sont conservés tant qu'on n'y touche pas ; taper une valeur l'applique à
//! toute la sélection. Rien n'est écrit avant « Enregistrer ».

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, UiBuilder,
};

use serde::{Deserialize, Serialize};

use super::mb_import::MbImport;
use super::tag_tools::{Tool, ToolKind};
use super::{Page, SterMusic, View};
use crate::covers::Covers;
use crate::icons::{self, Icon};
use crate::library::{self, fold, plural, Track};
use crate::tags::{self, Cover, CoverChange, Field, FileTags, Values, FIELDS};
use crate::{theme, widgets};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Column {
    File,
    Tag,
    Title,
    Artist,
    Album,
    Track,
    Year,
    Genre,
}

const COLUMNS: [(Column, &str, f32); 8] = [
    (Column::File, "Fichier", 0.21),
    (Column::Tag, "Tag", 0.08),
    (Column::Title, "Titre", 0.20),
    (Column::Artist, "Artiste", 0.14),
    (Column::Album, "Album", 0.16),
    (Column::Track, "Piste", 0.065),
    (Column::Year, "Année", 0.065),
    (Column::Genre, "Genre", 0.08),
];

/// Largeur minimale d'une colonne (part de la largeur du tableau).
const MIN_COL: f32 = 0.04;

/// Réglages de l'éditeur gardés d'un lancement à l'autre.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Largeur des colonnes (parts de la largeur du tableau, somme 1).
    pub cols: [f32; 8],
    /// Derniers motifs de « Renommer les fichiers » et de « Tags ← nom de fichier ».
    pub rename: String,
    pub parse: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            cols: COLUMNS.map(|c| c.2),
            rename: "%track% - %title%".into(),
            parse: "%track% - %title%".into(),
        }
    }
}

impl Prefs {
    /// Des largeurs relues d'un fichier de réglages abîmé reviennent à celles d'origine.
    pub fn sanitized(mut self) -> Self {
        let sum: f32 = self.cols.iter().sum();
        if self.cols.iter().any(|c| !c.is_finite() || *c < MIN_COL * 0.9) || (sum - 1.0).abs() > 0.02 {
            self.cols = COLUMNS.map(|c| c.2);
        }
        self
    }
}

pub(super) type CoverSlot = Option<(u64, Option<egui::TextureHandle>, [u32; 2])>;

#[derive(Default)]
pub struct TagEditor {
    pub(super) files: Vec<FileTags>,
    /// Valeurs en cours d'édition, une copie par fichier.
    pub(super) values: Vec<Values>,
    /// Pochette à écrire, par fichier (`None` : inchangée).
    pub(super) covers: Vec<Option<CoverChange>>,
    selected: Vec<bool>,
    anchor: Option<usize>,
    /// Ordre d'affichage des fichiers (tri par colonne).
    order: Vec<usize>,
    sort: Option<(Column, bool)>,
    filter: String,
    /// Chemins des fichiers de la liste (un fichier n'y entre qu'une fois).
    pub(super) known: HashSet<PathBuf>,
    loading: Vec<Load>,
    saving: Option<Saving>,
    cover_tex: CoverSlot,
    /// Vignettes des pochettes de la liste, pour « Reprendre une pochette de la liste ».
    cover_thumbs: HashMap<u64, Option<egui::TextureHandle>>,
    pub notice: Option<String>,
    pub(super) mb: Option<MbImport>,
    /// Outil ouvert (rechercher-remplacer, renommer, tags ← nom de fichier).
    pub(super) tool: Option<Tool>,
    confirm_clear: bool,
    pub(super) prefs: Prefs,
}

/// Une lecture de tags en cours (un dossier ou un lot de fichiers ajoutés).
struct Load {
    rx: Receiver<LoadMsg>,
    cancel: Arc<AtomicBool>,
    /// Nombre de fichiers à lire ; `None` tant que les dossiers sont parcourus.
    total: Option<usize>,
    done: usize,
}

enum LoadMsg {
    Total(usize),
    Files(Vec<FileTags>),
}

struct Saving {
    rx: Receiver<SaveMsg>,
    done: usize,
    total: usize,
}

enum SaveMsg {
    Progress,
    Done {
        refreshed: Vec<FileTags>,
        errors: Vec<(PathBuf, String)>,
        tracks: Vec<Track>,
    },
}

/// Un fichier à écrire : ses champs modifiés et sa nouvelle pochette.
type Job = (PathBuf, Vec<(Field, String)>, Option<CoverChange>);

impl TagEditor {
    pub fn new(prefs: Prefs) -> Self {
        Self {
            prefs,
            ..Self::default()
        }
    }

    /// Ajoute des fichiers ou des dossiers à la liste. Tout se passe en arrière-plan (parcours
    /// des dossiers compris : un gros dossier sur un disque externe peut prendre du temps), et
    /// l'import peut être arrêté ; les nouveaux fichiers sont sélectionnés quand ils arrivent.
    fn load(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        if paths.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (ctx, stop, known) = (ctx.clone(), cancel.clone(), self.known.clone());
        std::thread::spawn(move || {
            let files: Vec<PathBuf> = tags::expand_until(paths, &stop)
                .into_iter()
                .filter(|p| !known.contains(p))
                .collect();
            if tx.send(LoadMsg::Total(files.len())).is_err() {
                return;
            }
            ctx.request_repaint();
            // Les morceaux d'un album portent la même pochette : une seule copie en mémoire.
            let mut covers: HashMap<u64, Cover> = HashMap::new();
            // Par paquets : la liste se remplit au fur et à mesure.
            let mut batch = Vec::new();
            let mut sent = Instant::now();
            for p in &files {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let mut f = tags::read(p);
                if let Some(c) = f.cover.take() {
                    f.cover = Some(covers.entry(c.hash).or_insert(c).clone());
                }
                batch.push(f);
                if batch.len() >= 64 || sent.elapsed() > Duration::from_millis(120) {
                    if tx.send(LoadMsg::Files(std::mem::take(&mut batch))).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                    sent = Instant::now();
                }
            }
            let _ = tx.send(LoadMsg::Files(batch));
            ctx.request_repaint();
        });
        self.loading.push(Load {
            rx,
            cancel,
            total: None,
            done: 0,
        });
    }

    /// Arrête les lectures en cours ; les fichiers déjà lus restent dans la liste.
    fn stop_loading(&mut self) {
        for l in self.loading.drain(..) {
            l.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn loading(&self) -> bool {
        !self.loading.is_empty()
    }

    /// Avancement des lectures : (fichiers lus, fichiers à lire) ; `None` pendant le parcours
    /// des dossiers.
    fn load_progress(&self) -> Option<(usize, usize)> {
        let mut total = 0;
        for l in &self.loading {
            total += l.total?;
        }
        Some((self.loading.iter().map(|l| l.done).sum(), total))
    }

    /// Vide la liste (et arrête les lectures) ; les réglages de l'éditeur sont gardés.
    fn clear(&mut self) {
        self.stop_loading();
        *self = Self {
            prefs: std::mem::take(&mut self.prefs),
            ..Self::default()
        };
    }

    pub(super) fn busy(&self) -> bool {
        !self.loading.is_empty() || self.saving.is_some()
    }

    pub(super) fn saving(&self) -> bool {
        self.saving.is_some()
    }

    /// Une fenêtre de l'éditeur est ouverte : les raccourcis du lecteur attendent.
    pub(super) fn modal_open(&self) -> bool {
        self.mb.is_some() || self.tool.is_some() || self.confirm_clear
    }

    pub(super) fn file_dirty(&self, k: usize) -> bool {
        self.values[k] != self.files[k].values || self.covers[k].is_some()
    }

    pub fn dirty_count(&self) -> usize {
        (0..self.files.len()).filter(|&k| self.file_dirty(k)).count()
    }

    fn dirty(&self) -> bool {
        (0..self.files.len()).any(|k| self.file_dirty(k))
    }

    /// Fichiers sélectionnés (lisibles), dans l'ordre d'affichage.
    pub(super) fn scope(&self) -> Vec<usize> {
        self.order
            .iter()
            .copied()
            .filter(|&k| self.selected[k] && self.files[k].error.is_none())
            .collect()
    }

    /// Tous les fichiers lisibles, dans l'ordre d'affichage.
    pub(super) fn all_readable(&self) -> Vec<usize> {
        self.order
            .iter()
            .copied()
            .filter(|&k| self.files[k].error.is_none())
            .collect()
    }

    /// Fichiers sélectionnés, illisibles compris (pour les retirer de la liste).
    fn selection(&self) -> Vec<usize> {
        (0..self.files.len()).filter(|&k| self.selected[k]).collect()
    }

    /// Fichiers sélectionnés qui ont des modifications à enregistrer.
    fn dirty_selected(&self) -> usize {
        (0..self.files.len())
            .filter(|&k| self.selected[k] && self.file_dirty(k))
            .count()
    }

    fn field_dirty(&self, scope: &[usize], i: usize) -> bool {
        scope
            .iter()
            .any(|&k| self.values[k][i] != self.files[k].values[i])
    }

    fn effective_cover(&self, k: usize) -> Option<&Cover> {
        match &self.covers[k] {
            Some(change) => change.as_ref(),
            None => self.files[k].cover.as_ref(),
        }
    }

    /// Change la pochette de la sélection (revenir à l'originale annule le changement).
    pub fn set_cover(&mut self, cover: CoverChange) {
        let scope = self.scope();
        if scope.is_empty() {
            self.notice = Some("Sélectionne d'abord des fichiers".into());
            return;
        }
        for k in scope {
            self.set_file_cover(k, cover.clone());
        }
        self.notice = None;
    }

    pub(super) fn set_file_cover(&mut self, k: usize, cover: CoverChange) {
        let original = self.files[k].cover.as_ref().map(|c| c.hash);
        self.covers[k] = (cover.as_ref().map(|c| c.hash) != original).then_some(cover);
    }

    fn fill(&mut self, field: Field, value: impl Fn(usize, &Path) -> String) {
        let scope = self.scope();
        for (n, &k) in scope.iter().enumerate() {
            self.values[k][field.index()] = value(n, &self.files[k].path);
        }
    }

    fn revert(&mut self, ks: &[usize]) {
        for &k in ks {
            self.values[k] = self.files[k].values.clone();
            self.covers[k] = None;
        }
    }

    fn remove(&mut self, ks: &[usize]) {
        let drop: HashSet<usize> = ks.iter().copied().collect();
        for &k in &drop {
            self.known.remove(&self.files[k].path);
        }
        fn keep<T>(v: &mut Vec<T>, drop: &HashSet<usize>) {
            let mut i = 0;
            v.retain(|_| {
                i += 1;
                !drop.contains(&(i - 1))
            });
        }
        keep(&mut self.files, &drop);
        keep(&mut self.values, &drop);
        keep(&mut self.covers, &drop);
        keep(&mut self.selected, &drop);
        self.anchor = None;
        self.resort();
    }

    fn cell(&self, k: usize, col: Column) -> String {
        let v = &self.values[k];
        let get = |f: Field| v[f.index()].clone();
        match col {
            Column::File => file_name(&self.files[k].path),
            Column::Tag => self.files[k].tags.clone(),
            Column::Title => get(Field::Title),
            Column::Artist => get(Field::Artist),
            Column::Album => get(Field::Album),
            Column::Year => get(Field::Year),
            Column::Genre => get(Field::Genre),
            Column::Track => match (get(Field::Track), get(Field::TrackTotal)) {
                (t, total) if total.is_empty() => t,
                (t, total) => format!("{t}/{total}"),
            },
        }
    }

    fn cell_dirty(&self, k: usize, col: Column) -> bool {
        let changed = |f: Field| self.values[k][f.index()] != self.files[k].values[f.index()];
        match col {
            Column::File | Column::Tag => false,
            Column::Title => changed(Field::Title),
            Column::Artist => changed(Field::Artist),
            Column::Album => changed(Field::Album),
            Column::Year => changed(Field::Year),
            Column::Genre => changed(Field::Genre),
            Column::Track => changed(Field::Track) || changed(Field::TrackTotal),
        }
    }

    /// Recalcule l'ordre d'affichage (pas à chaque frappe : les lignes ne sautent pas).
    pub(super) fn resort(&mut self) {
        let mut order: Vec<usize> = (0..self.files.len()).collect();
        if let Some((col, asc)) = self.sort {
            let num = |s: &str| s.parse::<u32>().unwrap_or(u32::MAX);
            order.sort_by_cached_key(|&k| {
                let v = &self.values[k];
                let key = match col {
                    Column::Track => format!(
                        "{:010}{:010}",
                        num(&v[Field::Disc.index()]),
                        num(&v[Field::Track.index()])
                    ),
                    Column::Year => format!("{:010}", num(&v[Field::Year.index()])),
                    c => fold(&self.cell(k, c)),
                };
                (key, fold(&file_name(&self.files[k].path)))
            });
            if !asc {
                order.reverse();
            }
        }
        self.order = order;
    }

    /// Lignes affichées : l'ordre de tri, réduit par le filtre.
    fn visible(&self) -> Vec<usize> {
        let words: Vec<String> = fold(&self.filter).split_whitespace().map(str::to_owned).collect();
        if words.is_empty() {
            return self.order.clone();
        }
        self.order
            .iter()
            .copied()
            .filter(|&k| {
                let text = fold(&format!(
                    "{} {}",
                    file_name(&self.files[k].path),
                    self.values[k].join(" ")
                ));
                words.iter().all(|w| text.contains(w.as_str()))
            })
            .collect()
    }

    fn click(&mut self, rows: &[usize], i: usize, mods: egui::Modifiers) {
        let k = rows[i];
        if mods.shift {
            let from = self
                .anchor
                .and_then(|a| rows.iter().position(|&r| r == a))
                .unwrap_or(i);
            if !mods.command {
                self.selected.iter_mut().for_each(|s| *s = false);
            }
            for &r in &rows[from.min(i)..=from.max(i)] {
                self.selected[r] = true;
            }
        } else if mods.command {
            self.selected[k] = !self.selected[k];
            self.anchor = Some(k);
        } else {
            self.selected.iter_mut().for_each(|s| *s = false);
            self.selected[k] = true;
            self.anchor = Some(k);
        }
    }

    pub fn select_all(&mut self) {
        for k in self.visible() {
            self.selected[k] = true;
        }
    }

    /// Applique une pochette à tous les fichiers de la liste.
    fn set_cover_everywhere(&mut self, cover: &Cover) {
        for k in self.all_readable() {
            self.set_file_cover(k, Some(cover.clone()));
        }
    }

    /// Pochettes différentes présentes dans la liste (modifications en attente comprises), les
    /// plus fréquentes d'abord, avec le nombre de fichiers qui les portent.
    fn distinct_covers(&self) -> Vec<(Cover, usize)> {
        let mut found: Vec<(Cover, usize)> = Vec::new();
        let mut index: HashMap<u64, usize> = HashMap::new();
        for k in 0..self.files.len() {
            if let Some(c) = self.effective_cover(k) {
                match index.get(&c.hash) {
                    Some(&i) => found[i].1 += 1,
                    None => {
                        index.insert(c.hash, found.len());
                        found.push((c.clone(), 1));
                    }
                }
            }
        }
        found.sort_by_key(|c| std::cmp::Reverse(c.1));
        found
    }
}

/// Pochette lue depuis une image : JPEG et PNG sont gardés tels quels (sauf s'ils sont très
/// grands), les autres formats sont convertis en JPEG.
pub fn cover_from_bytes(data: Vec<u8>) -> Result<Cover, String> {
    let img = image::load_from_memory(&data).map_err(|_| "Image illisible".to_owned())?;
    let known = matches!(
        data.get(..4),
        Some([0x89, b'P', b'N', b'G']) | Some([0xFF, 0xD8, ..])
    );
    // Au-delà de 1600 px, la pochette alourdit chaque fichier pour rien : on la ramène à 1200 px.
    if known && img.width().max(img.height()) <= 1600 {
        return Ok(Cover::new(data));
    }
    let img = if img.width().max(img.height()) > 1600 {
        img.resize(1200, 1200, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92)
        .encode_image(&img.to_rgb8())
        .map_err(|e| e.to_string())?;
    Ok(Cover::new(out))
}

pub fn cover_from_file(path: &Path) -> Result<Cover, String> {
    cover_from_bytes(std::fs::read(path).map_err(|e| e.to_string())?)
}

fn is_image(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "bmp" | "gif"
        )
    })
}

pub(super) fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Taille lisible : « 312 Ko », « 9,6 Mo ».
fn size_text(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} Mo", bytes as f64 / (1u64 << 20) as f64).replace('.', ",")
    } else {
        format!("{} Ko", bytes.div_ceil(1024))
    }
}

/// Dessine une pochette (texture gardée tant que la même image est affichée) ; renvoie sa taille.
pub(super) fn paint_cover_texture(ui: &egui::Ui, slot: &mut CoverSlot, c: &Cover, rect: Rect) -> [u32; 2] {
    if slot.as_ref().map(|t| t.0) != Some(c.hash) {
        let (tex, size) = match image::load_from_memory(&c.data) {
            Ok(img) => {
                let size = [img.width(), img.height()];
                let img = img.thumbnail(512, 512).to_rgba8();
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [img.width() as usize, img.height() as usize],
                    img.as_raw(),
                );
                let tex = ui.ctx().load_texture(
                    format!("tag_cover_{}", c.hash),
                    color,
                    egui::TextureOptions::LINEAR,
                );
                (Some(tex), size)
            }
            Err(_) => (None, [0, 0]),
        };
        *slot = Some((c.hash, tex, size));
    }
    let (_, tex, size) = slot.as_ref().unwrap();
    widgets::cover(ui.painter(), rect, tex.as_ref(), 6, "");
    *size
}

/// Ce que le panneau de gauche demande à la page.
enum PanelAction {
    PickCover,
    Tool(ToolKind),
}

/// Ce que le tableau demande à la page.
enum TableAction {
    SaveSelection,
}

impl SterMusic {
    /// Ouvre la page de l'éditeur sur ces fichiers ou dossiers. Sans modification en attente,
    /// la liste repart de zéro ; sinon ils s'y ajoutent (rien n'est perdu).
    pub(super) fn open_tag_editor(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if self.tag_editor.saving.is_some() {
            self.toast("Enregistrement en cours…");
            return;
        }
        if self.tag_editor.dirty() {
            self.toast("Ajoutés à la liste : les modifications en attente sont gardées");
        } else {
            self.tag_editor.clear();
        }
        let ed = &mut self.tag_editor;
        ed.selected.iter_mut().for_each(|s| *s = false);
        ed.load(paths, ctx);
        self.go(View::Page(Page::Tags));
    }

    /// Fichiers déposés sur la fenêtre. Sur la page de l'éditeur : morceaux et dossiers
    /// rejoignent la liste, une image devient la pochette de la sélection. Ailleurs : un dossier
    /// rejoint la bibliothèque, des morceaux ouvrent l'éditeur.
    pub(super) fn handle_drop(&mut self, ctx: &egui::Context, dropped: Vec<PathBuf>) {
        let (images, others): (Vec<PathBuf>, Vec<PathBuf>) = dropped.into_iter().partition(|p| is_image(p));
        if self.view == View::Page(Page::Tags) {
            if let Some(img) = images.first() {
                let ed = &mut self.tag_editor;
                match cover_from_file(img) {
                    Ok(c) => ed.set_cover(Some(c)),
                    Err(e) => ed.notice = Some(e),
                }
            }
            let ed = &mut self.tag_editor;
            if !others.is_empty() {
                ed.selected.iter_mut().for_each(|s| *s = false);
                ed.load(others, ctx);
            }
            return;
        }
        let (dirs, files): (Vec<PathBuf>, Vec<PathBuf>) = others.into_iter().partition(|p| p.is_dir());
        for d in dirs {
            self.actions.push(super::Action::UseFolder(d));
        }
        if !files.is_empty() {
            self.open_tag_editor(ctx, files);
        }
    }

    pub(super) fn poll_tag_editor(&mut self, ctx: &egui::Context) {
        let ed = &mut self.tag_editor;
        let mut added = false;
        let mut nothing_found = false;
        for mut load in std::mem::take(&mut ed.loading) {
            let mut alive = true;
            loop {
                match load.rx.try_recv() {
                    Ok(LoadMsg::Total(n)) => load.total = Some(n),
                    Ok(LoadMsg::Files(files)) => {
                        load.done += files.len();
                        for f in files {
                            // Le même fichier a pu arriver par deux ajouts lancés en même temps.
                            if !ed.known.insert(f.path.clone()) {
                                continue;
                            }
                            ed.values.push(f.values.clone());
                            ed.covers.push(None);
                            ed.selected.push(true);
                            ed.files.push(f);
                            added = true;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        alive = false;
                        break;
                    }
                }
            }
            if alive {
                ed.loading.push(load);
            } else if load.total == Some(0) {
                nothing_found = true;
            }
        }
        if added {
            ed.resort();
        }
        if let Some(mb) = &mut ed.mb {
            mb.poll();
        }
        let mut finished = None;
        if let Some(s) = &mut ed.saving {
            while let Ok(msg) = s.rx.try_recv() {
                match msg {
                    SaveMsg::Progress => s.done += 1,
                    SaveMsg::Done {
                        refreshed,
                        errors,
                        tracks,
                    } => finished = Some((refreshed, errors, tracks)),
                }
            }
        }
        if ed.busy() || ed.mb.as_ref().is_some_and(MbImport::busy) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if nothing_found {
            self.toast("Aucun nouveau fichier audio à ajouter");
        }
        let ed = &mut self.tag_editor;
        let Some((refreshed, errors, tracks)) = finished else {
            return;
        };
        ed.saving = None;
        // Les fichiers écrits sont relus : la liste montre ce qui est vraiment dans les tags.
        let written: Vec<PathBuf> = refreshed.iter().map(|f| f.path.clone()).collect();
        let index: HashMap<&Path, usize> = ed
            .files
            .iter()
            .enumerate()
            .map(|(k, f)| (f.path.as_path(), k))
            .collect();
        let slots: Vec<Option<usize>> = refreshed
            .iter()
            .map(|f| index.get(f.path.as_path()).copied())
            .collect();
        for (f, k) in refreshed.into_iter().zip(slots) {
            if let Some(k) = k {
                ed.values[k] = f.values.clone();
                ed.covers[k] = None;
                ed.files[k] = f;
            }
        }
        ed.cover_tex = None;

        // Les morceaux de la bibliothèque prennent leurs nouveaux tags (et le cache aussi).
        let mut all = self.lib.tracks.clone();
        let mut changed = false;
        for t in tracks {
            if let Some(i) = self.lib.find(&t.path) {
                all[i] = t;
                changed = true;
            }
        }
        if changed {
            library::save_cache(&all);
            self.set_library(all);
        }
        self.covers.forget(&written);
        let ok = plural(written.len(), "fichier modifié", "fichiers modifiés");
        match errors.first() {
            None => self.toast(format!("Tags enregistrés : {ok}")),
            Some((path, e)) => self.toast(format!(
                "{ok}, {} : {} ({e})",
                plural(errors.len(), "échec", "échecs"),
                file_name(path)
            )),
        }
    }

    /// Écrit les modifications dans les fichiers : toutes, ou celles de la sélection seulement.
    pub(super) fn save_tags(&mut self, ctx: &egui::Context, only_selection: bool) {
        // Le morceau chargé est écrit via une copie : son décodeur lit encore le fichier.
        let loaded = self
            .current()
            .filter(|_| self.player.as_ref().is_some_and(|p| p.has_track()))
            .map(|i| self.lib.tracks[i].path.clone());
        let opts = tags::WriteOpts {
            id3v23: self.settings.tag_id3v23,
            keep_mtime: self.settings.tag_keep_mtime,
            via_copy: false,
        };
        let ed = &mut self.tag_editor;
        if ed.saving.is_some() {
            return;
        }
        let jobs: Vec<Job> = (0..ed.files.len())
            .filter(|&k| ed.files[k].error.is_none() && ed.file_dirty(k))
            .filter(|&k| !only_selection || ed.selected[k])
            .map(|k| {
                let fields = FIELDS
                    .iter()
                    .filter(|f| ed.values[k][f.index()] != ed.files[k].values[f.index()])
                    .map(|&f| (f, ed.values[k][f.index()].clone()))
                    .collect();
                (ed.files[k].path.clone(), fields, ed.covers[k].clone())
            })
            .collect();
        if jobs.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        ed.saving = Some(Saving {
            rx,
            done: 0,
            total: jobs.len(),
        });
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let (mut written, mut errors) = (Vec::new(), Vec::new());
            for (path, fields, cover) in jobs {
                let o = tags::WriteOpts {
                    via_copy: loaded.as_ref() == Some(&path),
                    ..opts
                };
                match tags::write(&path, &fields, cover.as_ref(), o) {
                    Ok(()) => written.push(path),
                    Err(e) => errors.push((path, e)),
                }
                let _ = tx.send(SaveMsg::Progress);
                ctx.request_repaint();
            }
            let refreshed = written.iter().map(|p| tags::read(p)).collect();
            let tracks = written.iter().map(|p| library::read_track(p)).collect();
            let _ = tx.send(SaveMsg::Done {
                refreshed,
                errors,
                tracks,
            });
            ctx.request_repaint();
        });
    }

    fn pick_cover(&mut self) {
        if let Some(file) = rfd::FileDialog::new()
            .set_title("Choisir une pochette")
            .add_filter("Images", &["jpg", "jpeg", "png", "webp", "bmp", "gif"])
            .pick_file()
        {
            match cover_from_file(&file) {
                Ok(c) => self.tag_editor.set_cover(Some(c)),
                Err(e) => self.tag_editor.notice = Some(e),
            }
        }
    }

    fn add_tag_folder(&mut self, ctx: &egui::Context) {
        if let Some(dir) = rfd::FileDialog::new()
            .set_title("Dossier dont modifier les tags")
            .pick_folder()
        {
            let ed = &mut self.tag_editor;
            ed.selected.iter_mut().for_each(|s| *s = false);
            ed.load(vec![dir], ctx);
        }
    }

    fn add_tag_files(&mut self, ctx: &egui::Context) {
        if let Some(files) = rfd::FileDialog::new()
            .set_title("Fichiers dont modifier les tags")
            .add_filter("Musique", library::AUDIO_EXTENSIONS)
            .pick_files()
        {
            let ed = &mut self.tag_editor;
            ed.selected.iter_mut().for_each(|s| *s = false);
            ed.load(files, ctx);
        }
    }

    // ---------- Page ----------

    pub(super) fn tags_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !ctx.wants_keyboard_input() && !self.tag_editor.modal_open() {
            let save_selection = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
            if ctx.input_mut(|i| i.consume_key(save_selection, egui::Key::S)) {
                self.save_tags(&ctx, true);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
                self.save_tags(&ctx, false);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
                self.tag_editor.select_all();
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
                let ks = self.tag_editor.selection();
                self.tag_editor.remove(&ks);
            }
        }

        // En-tête : titre et actions (de droite à gauche).
        let mut action = None;
        ui.horizontal(|ui| {
            super::views::heading(ui, "Éditeur de tags");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                let ed = &self.tag_editor;
                let n = ed.dirty_count();
                let in_selection = ed.dirty_selected();
                let partial = in_selection > 0 && in_selection < n;
                if let Some(s) = &ed.saving {
                    ui.add(
                        egui::ProgressBar::new(s.done as f32 / s.total.max(1) as f32)
                            .desired_width(200.0)
                            .text(format!("Enregistrement… {}/{}", s.done, s.total)),
                    );
                } else {
                    ui.scope(|ui| {
                        if n == 0 {
                            ui.disable();
                            ui.set_opacity(0.5);
                        }
                        let label = match (n, partial) {
                            (0, _) => "Enregistrer".to_owned(),
                            (_, true) => format!("Tout enregistrer ({n})"),
                            (_, false) => format!("Enregistrer ({n})"),
                        };
                        if widgets::primary_button(ui, None, &label)
                            .on_hover_text("Écrit toutes les modifications dans les fichiers (Ctrl+S)")
                            .clicked()
                        {
                            action = Some("save");
                        }
                    });
                    if partial
                        && widgets::secondary_button(
                            ui,
                            Some(Icon::Save),
                            &format!("Sélection ({in_selection})"),
                        )
                        .on_hover_text("Enregistre seulement les fichiers sélectionnés (Ctrl+Maj+S)")
                        .clicked()
                    {
                        action = Some("save_selection");
                    }
                    if n > 0
                        && widgets::icon_button(
                            ui,
                            Icon::Undo,
                            "Annuler toutes les modifications",
                            34.0,
                            16.0,
                        )
                        .clicked()
                    {
                        action = Some("revert");
                    }
                }
                if !ed.files.is_empty()
                    && widgets::secondary_button(ui, Some(Icon::Globe), "MusicBrainz")
                        .on_hover_text(
                            "Chercher l'album de la sélection en ligne : titres, numéros, pochette…",
                        )
                        .clicked()
                {
                    action = Some("mb");
                }
                if widgets::icon_button(ui, Icon::FolderPlus, "Ajouter un dossier", 34.0, 16.0).clicked() {
                    action = Some("folder");
                }
                if widgets::icon_button(ui, Icon::Plus, "Ajouter des fichiers", 34.0, 16.0).clicked() {
                    action = Some("files");
                }
                if (!ed.files.is_empty() || ed.loading())
                    && ed.saving.is_none()
                    && widgets::icon_button(ui, Icon::Trash, "Vider la liste", 34.0, 16.0).clicked()
                {
                    action = Some("clear");
                }
            });
        });
        match action {
            Some("save") => self.save_tags(&ctx, false),
            Some("save_selection") => self.save_tags(&ctx, true),
            Some("revert") => {
                let all: Vec<usize> = (0..self.tag_editor.files.len()).collect();
                self.tag_editor.revert(&all);
            }
            Some("mb") => {
                let ed = &mut self.tag_editor;
                if ed.scope().is_empty() {
                    ed.select_all();
                }
                let scope = ed.scope();
                ed.mb = Some(MbImport::new(ed, scope));
            }
            Some("folder") => self.add_tag_folder(&ctx),
            Some("files") => self.add_tag_files(&ctx),
            Some("clear") => {
                // Des modifications en attente : on demande avant de tout perdre.
                if self.tag_editor.dirty() {
                    self.tag_editor.confirm_clear = true;
                } else {
                    self.tag_editor.clear();
                }
            }
            _ => {}
        }
        if loading_bar(ui, &self.tag_editor) {
            self.tag_editor.stop_loading();
            let n = self.tag_editor.files.len();
            self.toast(format!(
                "Import arrêté : {} dans la liste",
                plural(n, "fichier", "fichiers")
            ));
        }
        ui.add_space(14.0);

        if self.tag_editor.files.is_empty() {
            if !self.tag_editor.loading() {
                widgets::empty_state(
                    ui,
                    Icon::Tag,
                    "Aucun fichier à modifier",
                    "Clic droit sur un morceau ou un album → « Modifier les infos », ou glisse des fichiers ou un dossier ici.",
                );
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 380.0).max(0.0) / 2.0);
                    if widgets::secondary_button(ui, Some(Icon::Plus), "Ajouter des fichiers").clicked() {
                        self.add_tag_files(&ctx);
                    }
                    if widgets::secondary_button(ui, Some(Icon::FolderPlus), "Ajouter un dossier").clicked() {
                        self.add_tag_folder(&ctx);
                    }
                });
            }
            drop_hint(ui, ui.available_rect_before_wrap().shrink2(vec2(16.0, 12.0)));
            self.clear_dialog(&ctx);
            return;
        }

        let body = ui.available_rect_before_wrap();
        let body = Rect::from_min_max(body.min, body.max - vec2(16.0, 12.0));
        let panel = Rect::from_min_size(body.min, vec2(330.0, body.height()));
        let table = Rect::from_min_max(pos2(panel.right() + 16.0, body.top()), body.max);
        let mut panel_action = None;
        ui.scope_builder(UiBuilder::new().max_rect(panel), |ui| {
            panel_action = side_panel(ui, &mut self.tag_editor, &mut self.covers);
        });
        let mut table_action = None;
        ui.scope_builder(UiBuilder::new().max_rect(table), |ui| {
            table_action = file_table(ui, &mut self.tag_editor);
        });
        drop_hint(ui, table);
        match panel_action {
            Some(PanelAction::PickCover) => self.pick_cover(),
            Some(PanelAction::Tool(kind)) => {
                let ed = &mut self.tag_editor;
                ed.tool = Some(Tool::new(kind, &ed.prefs));
            }
            None => {}
        }
        match table_action {
            Some(TableAction::SaveSelection) => self.save_tags(&ctx, true),
            None => {}
        }
        if self.tag_editor.mb.is_some() {
            self.mb_dialog(&ctx);
        }
        if self.tag_editor.tool.is_some() {
            self.tool_dialog(&ctx);
        }
        self.clear_dialog(&ctx);
    }

    /// « Vider la liste ? » quand des modifications ne sont pas enregistrées.
    fn clear_dialog(&mut self, ctx: &egui::Context) {
        if !self.tag_editor.confirm_clear {
            return;
        }
        let n = self.tag_editor.dirty_count();
        let (mut close, mut confirm) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("tag_clear"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .inner_margin(24.0)
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(RichText::new("Vider la liste ?").font(theme::semibold(20.0)));
                ui.add_space(12.0);
                ui.label(
                    RichText::new(format!(
                        "{} non enregistrées : elles seront perdues. Les fichiers, eux, ne sont pas touchés.",
                        plural(n, "fichier a des modifications", "fichiers ont des modifications"),
                    ))
                    .color(theme::TEXT_2),
                );
                ui.add_space(20.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::primary_button(ui, None, "Vider la liste").clicked() {
                        confirm = true;
                    }
                    if widgets::secondary_button(ui, None, "Annuler").clicked() {
                        close = true;
                    }
                });
            });
        if confirm {
            self.tag_editor.clear();
        } else if close || modal.should_close() {
            self.tag_editor.confirm_clear = false;
        }
    }
}

/// Bandeau d'avancement des lectures de tags, avec « Arrêter » ; vrai si on le demande.
fn loading_bar(ui: &mut egui::Ui, ed: &TagEditor) -> bool {
    let t = crate::anim::toggle(
        ui.ctx(),
        egui::Id::new("tag_loading"),
        ed.loading(),
        crate::anim::NORMAL,
    );
    if t <= 0.0 {
        return false;
    }
    let mut stop = false;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width() - 16.0, 46.0 * t), Sense::hover());
    if !ed.loading() || t < 0.6 {
        return false;
    }
    let bar = Rect::from_min_max(rect.min + vec2(0.0, 8.0), rect.max);
    ui.painter().rect(
        bar,
        CornerRadius::same(8),
        theme::alpha(theme::accent(), 0x1C),
        egui::Stroke::new(1.0_f32, theme::alpha(theme::accent_light(), 0x50)),
        egui::StrokeKind::Inside,
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(bar.shrink2(vec2(12.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.set_opacity(((t - 0.6) / 0.4).clamp(0.0, 1.0));
            ui.add(egui::Spinner::new().size(16.0).color(theme::accent_light()));
            ui.add_space(4.0);
            match ed.load_progress() {
                Some((done, total)) => {
                    ui.label(RichText::new(format!("Lecture des tags… {done} / {total}")).color(theme::TEXT));
                    ui.add_space(8.0);
                    ui.add(
                        egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                            .desired_width((ui.available_width() - 150.0).clamp(60.0, 360.0))
                            .desired_height(6.0),
                    );
                }
                None => {
                    ui.label(RichText::new("Parcours des dossiers…").color(theme::TEXT));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::secondary_button(ui, Some(Icon::Stop), "Arrêter")
                    .on_hover_text("Arrête l'import ; les fichiers déjà lus restent dans la liste")
                    .clicked()
                {
                    stop = true;
                }
            });
        },
    );
    stop
}

/// Pendant qu'on survole la fenêtre avec des fichiers : la zone où les lâcher.
fn drop_hint(ui: &egui::Ui, rect: Rect) {
    let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
    let t = crate::anim::toggle(
        ui.ctx(),
        egui::Id::new("tag_drop_hint"),
        hovering,
        crate::anim::FAST,
    );
    if t <= 0.0 {
        return;
    }
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("tag_drop_hint"),
    ));
    painter.rect(
        rect,
        CornerRadius::same(8),
        theme::alpha(theme::accent(), (0x30 as f32 * t) as u8),
        egui::Stroke::new(2.0_f32, theme::accent_light().gamma_multiply(t)),
        egui::StrokeKind::Inside,
    );
    let center = rect.center();
    icons::paint_centered(
        &painter,
        Icon::Download,
        center - vec2(0.0, 18.0),
        30.0,
        theme::TEXT.gamma_multiply(t),
    );
    painter.text(
        center + vec2(0.0, 16.0),
        Align2::CENTER_CENTER,
        "Lâche ici : morceaux et dossiers rejoignent la liste, une image devient la pochette",
        FontId::proportional(14.0),
        theme::TEXT.gamma_multiply(t),
    );
}

const GAP: f32 = 12.0;

/// Un champ : libellé (avec « annuler » s'il a changé) et zone de saisie.
fn field(ui: &mut egui::Ui, ed: &mut TagEditor, scope: &[usize], f: Field, width: f32, rows: usize) {
    let i = f.index();
    let first = ed.values[scope[0]][i].clone();
    let same = scope.iter().all(|&k| ed.values[k][i] == first);
    let dirty = ed.field_dirty(scope, i);

    let (r, _) = ui.allocate_exact_size(vec2(width, 17.0), Sense::hover());
    let color = if dirty {
        theme::accent_light()
    } else {
        theme::TEXT_2
    };
    ui.painter().text(
        r.left_center(),
        Align2::LEFT_CENTER,
        f.label(),
        FontId::proportional(12.0),
        color,
    );
    if dirty {
        let b = Rect::from_center_size(pos2(r.right() - 9.0, r.center().y), vec2(18.0, 18.0));
        let resp = ui.interact(b, ui.id().with(("undo", i)), Sense::click());
        widgets::paint_interaction(ui, b, 4, &resp);
        icons::paint_centered(
            ui.painter(),
            Icon::Undo,
            b.center(),
            13.0,
            if resp.hovered() {
                theme::TEXT
            } else {
                theme::TEXT_2
            },
        );
        if resp.on_hover_text("Annuler cette modification").clicked() {
            for &k in scope {
                ed.values[k][i] = ed.files[k].values[i].clone();
            }
        }
    }
    ui.add_space(2.0);

    let mut text = if same { first } else { String::new() };
    let hint = match (same, width < 160.0) {
        (true, _) => "",
        (false, true) => "Multiples",
        (false, false) => "Valeurs multiples (conservées)",
    };
    let resp = widgets::text_field(ui, egui::Id::new(("tag_field", i)), &mut text, hint, width, rows);
    let resp = if same {
        resp
    } else {
        // Calculé au survol seulement : avec des milliers de fichiers sélectionnés, lister les
        // valeurs à chaque image figerait l'interface.
        resp.on_hover_ui(|ui| {
            let mut seen: HashSet<&str> = HashSet::new();
            let mut tip: Vec<String> = Vec::new();
            for &k in scope {
                let v = ed.values[k][i].as_str();
                if seen.insert(v) && seen.len() <= 6 {
                    tip.push(if v.is_empty() {
                        "(vide)".to_owned()
                    } else {
                        v.to_owned()
                    });
                }
            }
            if seen.len() > 6 {
                tip.push(format!("… et {} autres", seen.len() - 6));
            }
            ui.label(tip.join("\n"));
        })
    };
    if resp.changed() {
        if f.numeric() {
            text.retain(|c| c.is_ascii_digit());
        }
        for &k in scope {
            ed.values[k][i] = text.clone();
        }
    }
    ui.add_space(8.0);
}

/// Champs, pochette et outils de la sélection ; renvoie ce que la page doit faire ensuite.
fn side_panel(ui: &mut egui::Ui, ed: &mut TagEditor, covers: &mut Covers) -> Option<PanelAction> {
    let mut action = None;
    egui::Frame::NONE
        .fill(theme::white(0x08))
        .stroke(egui::Stroke::new(1.0_f32, theme::white(0x0E)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            egui::ScrollArea::vertical()
                .id_salt("tag_panel")
                .auto_shrink(false)
                .show(ui, |ui| {
                    let w = ui.available_width() - 8.0;
                    ui.set_width(w);
                    let scope = ed.scope();
                    let summary = match scope.len() {
                        0 => "Aucune sélection".to_owned(),
                        1 => file_name(&ed.files[scope[0]].path),
                        n => format!("{n} fichiers sélectionnés"),
                    };
                    let g = widgets::galley(ui, &summary, theme::semibold(14.0), theme::TEXT, w);
                    ui.label(g);
                    ui.add_space(10.0);
                    if scope.is_empty() {
                        ui.label(
                            RichText::new(
                                "Choisis des fichiers dans la liste (clic, Ctrl+clic, Maj+clic, Ctrl+A) \
                                 pour modifier leurs tags.",
                            )
                            .color(theme::TEXT_3),
                        );
                        return;
                    }
                    ui.spacing_mut().item_spacing = vec2(GAP, 0.0);
                    for f in [Field::Title, Field::Artist, Field::Album, Field::AlbumArtist] {
                        field(ui, ed, &scope, f, w, 1);
                    }
                    let row = |ui: &mut egui::Ui, ed: &mut TagEditor, cells: &[(Field, f32)]| {
                        ui.horizontal(|ui| {
                            for &(f, cw) in cells {
                                ui.vertical(|ui| {
                                    ui.set_width(cw);
                                    field(ui, ed, &scope, f, cw, 1);
                                });
                            }
                        });
                    };
                    row(ui, ed, &[(Field::Year, 70.0), (Field::Genre, w - 70.0 - GAP)]);
                    let q = (w - 3.0 * GAP) / 4.0;
                    row(
                        ui,
                        ed,
                        &[
                            (Field::Track, q),
                            (Field::TrackTotal, q),
                            (Field::Disc, q),
                            (Field::DiscTotal, q),
                        ],
                    );
                    field(ui, ed, &scope, Field::Composer, w, 1);
                    field(ui, ed, &scope, Field::Comment, w, 2);
                    ui.add_space(4.0);
                    if cover_section(ui, ed, &scope, covers, w) {
                        action = Some(PanelAction::PickCover);
                    }
                    ui.add_space(14.0);
                    if let Some(kind) = tools(ui, ed, &scope) {
                        action = Some(PanelAction::Tool(kind));
                    }
                    ui.add_space(8.0);
                });
        });
    action
}

fn cover_section(
    ui: &mut egui::Ui,
    ed: &mut TagEditor,
    scope: &[usize],
    covers: &mut Covers,
    w: f32,
) -> bool {
    ui.label(
        RichText::new("Pochette")
            .font(theme::semibold(13.0))
            .color(theme::TEXT_2),
    );
    ui.add_space(6.0);
    let side = 140.0;
    let (row, _) = ui.allocate_exact_size(vec2(w, side), Sense::hover());
    let rect = Rect::from_min_size(row.min, vec2(side, side));
    let first = scope.first().and_then(|&k| ed.effective_cover(k)).cloned();
    let same = scope
        .iter()
        .all(|&k| ed.effective_cover(k).map(|c| c.hash) == first.as_ref().map(|c| c.hash));
    let changed = scope.iter().any(|&k| ed.covers[k].is_some());
    let info: Vec<String> = match (&first, same) {
        (Some(c), true) => {
            let size = paint_cover_texture(ui, &mut ed.cover_tex, c, rect);
            let mime = match c.data.get(..4) {
                Some([0x89, b'P', b'N', b'G']) => "image/png",
                Some([0xFF, 0xD8, ..]) => "image/jpeg",
                _ => "image",
            };
            vec![
                mime.to_owned(),
                format!("{}×{}", size[0], size[1]),
                size_text(c.data.len() as u64),
                "Pochette avant".to_owned(),
            ]
        }
        (None, true) => {
            // Pas de pochette intégrée : celle du dossier, si elle existe, est montrée pour info.
            let folder = if changed {
                None
            } else {
                scope
                    .first()
                    .and_then(|&k| covers.thumb(&ed.files[k].path).cloned())
            };
            widgets::cover(ui.painter(), rect, folder.as_ref(), 6, "");
            if folder.is_some() {
                vec!["Image du dossier".into(), "non intégrée".into()]
            } else {
                vec!["Aucune pochette".into()]
            }
        }
        (_, false) => {
            widgets::cover(ui.painter(), rect, None, 6, "");
            vec!["Pochettes".into(), "différentes".into()]
        }
    };
    let mut y = rect.top() + 2.0;
    for (n, line) in info.iter().enumerate() {
        ui.painter().text(
            pos2(rect.right() + 12.0, y),
            Align2::LEFT_TOP,
            line,
            FontId::proportional(12.5),
            if n == 0 { theme::TEXT } else { theme::TEXT_2 },
        );
        y += 18.0;
    }
    if changed {
        ui.painter().text(
            pos2(rect.right() + 12.0, y + 6.0),
            Align2::LEFT_TOP,
            "● modifiée",
            FontId::proportional(12.0),
            theme::accent_light(),
        );
    }
    if ui.ctx().input(|i| !i.raw.hovered_files.is_empty()) {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(6),
            egui::Stroke::new(2.0_f32, theme::accent_light()),
            egui::StrokeKind::Inside,
        );
    }
    ui.add_space(8.0);
    let mut pick = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if widgets::secondary_button(ui, Some(Icon::Image), "Remplacer…").clicked() {
            pick = true;
        }
        cover_picker(ui, ed);
        let any = scope.iter().any(|&k| ed.effective_cover(k).is_some());
        if any && widgets::icon_button(ui, Icon::Trash, "Retirer la pochette", 34.0, 16.0).clicked() {
            ed.set_cover(None);
        }
        if changed
            && widgets::icon_button(ui, Icon::Undo, "Garder la pochette d'origine", 34.0, 16.0).clicked()
        {
            for &k in scope {
                ed.covers[k] = None;
            }
        }
    });
    // La pochette affichée peut être donnée à tous les fichiers de la liste.
    if let (Some(c), true) = (&first, same) {
        let everywhere = ed
            .all_readable()
            .iter()
            .all(|&k| ed.effective_cover(k).map(|x| x.hash) == Some(c.hash));
        if !everywhere {
            ui.add_space(6.0);
            if widgets::secondary_button(ui, Some(Icon::Images), "Appliquer à toute la liste")
                .on_hover_text(format!(
                    "Donne cette pochette aux {} de la liste",
                    plural(ed.files.len(), "fichier", "fichiers")
                ))
                .clicked()
            {
                ed.set_cover_everywhere(c);
            }
        }
    }
    ui.add_space(4.0);
    let note = ed
        .notice
        .clone()
        .unwrap_or_else(|| "ou glisse une image sur la fenêtre".to_owned());
    ui.label(RichText::new(note).small().color(if ed.notice.is_some() {
        Color32::from_rgb(0xFF, 0x99, 0x99)
    } else {
        theme::TEXT_3
    }));
    pick
}

/// Pochettes déjà présentes dans la liste : un clic donne celle-là à la sélection.
fn cover_picker(ui: &mut egui::Ui, ed: &mut TagEditor) {
    let popup = ui.make_persistent_id("tag_cover_picker");
    let button = widgets::icon_button(
        ui,
        Icon::Images,
        "Reprendre une pochette déjà présente dans la liste",
        34.0,
        16.0,
    );
    if button.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup));
    }
    let mut chosen = None;
    egui::popup_below_widget(
        ui,
        popup,
        &button,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            const SIDE: f32 = 64.0;
            const PER_ROW: usize = 4;
            let found = ed.distinct_covers();
            ui.set_width(PER_ROW as f32 * (SIDE + 6.0) + 2.0);
            if found.is_empty() {
                ui.label(RichText::new("Aucun fichier de la liste n'a de pochette.").color(theme::TEXT_2));
                return;
            }
            ui.label(
                RichText::new(format!(
                    "{} dans la liste : clique pour la donner à la sélection",
                    plural(found.len(), "pochette", "pochettes")
                ))
                .small()
                .color(theme::TEXT_2),
            );
            ui.add_space(6.0);
            // Une vignette décodée par image affichée : la fenêtre s'ouvre tout de suite.
            let mut budget = 1;
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for row in found.chunks(PER_ROW) {
                    ui.horizontal(|ui| {
                        for (cover, count) in row {
                            let (r, resp) = ui.allocate_exact_size(vec2(SIDE, SIDE), Sense::click());
                            if !ed.cover_thumbs.contains_key(&cover.hash) && budget > 0 {
                                budget -= 1;
                                let tex = image::load_from_memory(&cover.data).ok().map(|img| {
                                    let img = img.thumbnail(160, 160).to_rgba8();
                                    let size = [img.width() as usize, img.height() as usize];
                                    ui.ctx().load_texture(
                                        format!("tag_thumb_{}", cover.hash),
                                        egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
                                        egui::TextureOptions::LINEAR,
                                    )
                                });
                                ed.cover_thumbs.insert(cover.hash, tex);
                            }
                            match ed.cover_thumbs.get(&cover.hash) {
                                Some(tex) => widgets::cover(ui.painter(), r, tex.as_ref(), 6, ""),
                                None => {
                                    widgets::cover(ui.painter(), r, None, 6, "");
                                    ui.ctx().request_repaint();
                                }
                            }
                            if resp.hovered() {
                                ui.painter().rect_stroke(
                                    r,
                                    CornerRadius::same(6),
                                    egui::Stroke::new(2.0_f32, theme::accent_light()),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            let resp = resp
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text(format!(
                                    "Portée par {}",
                                    plural(*count, "fichier", "fichiers")
                                ));
                            if resp.clicked() {
                                chosen = Some(cover.clone());
                            }
                        }
                    });
                }
            });
        },
    );
    if let Some(c) = chosen {
        ed.set_cover(Some(c));
        ui.memory_mut(|m| m.close_popup());
    }
}

/// Outils de la sélection ; renvoie l'outil à ouvrir dans sa fenêtre, s'il y en a un.
fn tools(ui: &mut egui::Ui, ed: &mut TagEditor, scope: &[usize]) -> Option<ToolKind> {
    let mut open = None;
    ui.label(
        RichText::new("Outils")
            .font(theme::semibold(13.0))
            .color(theme::TEXT_2),
    );
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        if scope.len() > 1
            && widgets::secondary_button(ui, Some(Icon::ListMusic), "Numéroter")
                .on_hover_text("Pistes 1, 2, 3… dans l'ordre de la liste, et le total")
                .clicked()
        {
            let total = scope.len().to_string();
            ed.fill(Field::Track, |n, _| (n + 1).to_string());
            ed.fill(Field::TrackTotal, |_, _| total.clone());
        }
        if widgets::secondary_button(ui, Some(Icon::Pencil), "Titre ← fichier")
            .on_hover_text("« 03 - Artiste - Titre.mp3 » donne « Titre »")
            .clicked()
        {
            ed.fill(Field::Title, |_, p| tags::title_from_filename(p));
        }
        for (kind, icon, label, tip) in [
            (
                ToolKind::Replace,
                Icon::Replace,
                "Remplacer…",
                "Rechercher et remplacer dans les tags, expressions régulières comprises",
            ),
            (
                ToolKind::FromName,
                Icon::Tag,
                "Tags ← nom…",
                "Lire les tags dans le nom des fichiers, d'après un motif (« %track% - %title% »)",
            ),
            (
                ToolKind::Rename,
                Icon::FilePen,
                "Renommer…",
                "Renommer les fichiers d'après leurs tags (« 03 - Titre.mp3 »)",
            ),
        ] {
            if widgets::secondary_button(ui, Some(icon), label)
                .on_hover_text(tip)
                .clicked()
            {
                open = Some(kind);
            }
        }
        if scope.iter().any(|&k| ed.file_dirty(k))
            && widgets::secondary_button(ui, Some(Icon::Undo), "Rétablir")
                .on_hover_text("Annuler les modifications de la sélection")
                .clicked()
        {
            ed.revert(scope);
        }
    });
    open
}

const ROW_H: f32 = 28.0;

/// Liste des fichiers, façon tableur : tri par colonne, colonnes redimensionnables, sélection
/// multiple, filtre.
fn file_table(ui: &mut egui::Ui, ed: &mut TagEditor) -> Option<TableAction> {
    let mut action = None;
    let full = ui.max_rect();
    let footer_h = 44.0;
    let rect = Rect::from_min_max(full.min, full.max - vec2(0.0, footer_h));
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        theme::white(0x05),
        egui::Stroke::new(1.0_f32, theme::white(0x0E)),
        egui::StrokeKind::Inside,
    );
    let gutter = 26.0;
    let inner_w = rect.width() - gutter - 8.0;
    let cols = ed.prefs.cols;
    let col_x = |c: usize| -> (f32, f32) {
        let start: f32 = cols[..c].iter().sum();
        (rect.left() + gutter + start * inner_w, cols[c] * inner_w - 10.0)
    };

    // En-tête : un clic trie, un deuxième inverse, un troisième revient à l'ordre d'arrivée.
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), 32.0));
    for (c, (col, label, _)) in COLUMNS.iter().enumerate() {
        let (x, w) = col_x(c);
        let r = Rect::from_min_max(pos2(x - 6.0, head.top()), pos2(x + w - 2.0, head.bottom()));
        let resp = ui.interact(r, ui.id().with(("col", c)), Sense::click());
        widgets::paint_interaction(ui, r.shrink(3.0), 4, &resp);
        let sorted = ed.sort.filter(|s| s.0 == *col);
        let text = match sorted {
            Some((_, true)) => format!("{label} ▲"),
            Some((_, false)) => format!("{label} ▼"),
            None => (*label).to_owned(),
        };
        let color = if sorted.is_some() {
            theme::TEXT
        } else {
            theme::TEXT_3
        };
        let g = widgets::galley(ui, &text, FontId::proportional(12.5), color, w);
        ui.painter()
            .galley(pos2(x, head.center().y - g.size().y / 2.0), g, color);
        if resp.clicked() {
            ed.sort = match sorted {
                Some((_, true)) => Some((*col, false)),
                Some((_, false)) => None,
                None => Some((*col, true)),
            };
            ed.resort();
        }
    }
    // Poignées entre les colonnes : glisser pour élargir l'une aux dépens de sa voisine,
    // double-clic pour revenir aux largeurs d'origine.
    for c in 0..COLUMNS.len() - 1 {
        let (next_x, _) = col_x(c + 1);
        let x = next_x - 8.0;
        let r = Rect::from_min_max(pos2(x - 5.0, head.top()), pos2(x + 5.0, head.bottom()));
        let resp = ui
            .interact(r, ui.id().with(("col_resize", c)), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        let active = resp.hovered() || resp.dragged();
        let t = widgets::hover_t(ui, resp.id, active);
        ui.painter().vline(
            x,
            head.top() + 8.0..=head.bottom() - 8.0,
            egui::Stroke::new(1.0 + t, theme::mix(theme::white(0x16), theme::accent_light(), t)),
        );
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            let pair = ed.prefs.cols[c] + ed.prefs.cols[c + 1];
            let moved = resp.drag_delta().x / inner_w.max(1.0);
            let left = (ed.prefs.cols[c] + moved).clamp(MIN_COL, pair - MIN_COL);
            ed.prefs.cols[c] = left;
            ed.prefs.cols[c + 1] = pair - left;
        }
        if resp.double_clicked() {
            ed.prefs.cols = COLUMNS.map(|c| c.2);
        }
    }
    ui.painter().hline(
        rect.x_range(),
        head.bottom(),
        egui::Stroke::new(1.0_f32, theme::stroke()),
    );

    let rows = ed.visible();
    let list = Rect::from_min_max(
        pos2(rect.left() + 4.0, head.bottom() + 2.0),
        rect.max - vec2(4.0, 4.0),
    );
    let mut menu_remove: Option<Vec<usize>> = None;
    let mut menu_open: Option<PathBuf> = None;
    ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical()
            .id_salt("tag_table")
            .auto_shrink(false)
            .show_rows(ui, ROW_H, rows.len(), |ui, range| {
                for i in range {
                    let k = rows[i];
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                    let selected = ed.selected[k];
                    if i % 2 == 1 {
                        ui.painter()
                            .rect_filled(r, CornerRadius::same(4), theme::white(0x03));
                    }
                    if selected {
                        ui.painter().rect_filled(
                            r,
                            CornerRadius::same(4),
                            theme::alpha(theme::accent(), 0x38),
                        );
                    }
                    widgets::paint_interaction(ui, r, 4, &resp);
                    if ed.file_dirty(k) {
                        ui.painter().circle_filled(
                            pos2(r.left() + gutter / 2.0 - 2.0, r.center().y),
                            3.5,
                            theme::accent_light(),
                        );
                    }
                    let error = ed.files[k].error.is_some();
                    for (c, (col, _, _)) in COLUMNS.iter().enumerate() {
                        let (x, w) = col_x(c);
                        let mut text = ed.cell(k, *col);
                        if error && *col == Column::File {
                            text.push_str(" (illisible)");
                        }
                        let color = if error {
                            theme::TEXT_3
                        } else if ed.cell_dirty(k, *col) {
                            theme::accent_light()
                        } else if matches!(col, Column::File | Column::Title) {
                            theme::TEXT
                        } else {
                            theme::TEXT_2
                        };
                        let g = widgets::galley(ui, &text, FontId::proportional(13.0), color, w);
                        ui.painter()
                            .galley(pos2(x - 4.0, r.center().y - g.size().y / 2.0), g, color);
                    }
                    let f = &ed.files[k];
                    let tip = match &f.error {
                        Some(e) => format!("{}\n{e}", f.path.display()),
                        None => format!(
                            "{}\n{} · {} · {}",
                            f.path.display(),
                            f.kind,
                            library::fmt_duration(Duration::from_millis(f.duration_ms)),
                            size_text(f.size)
                        ),
                    };
                    let resp = resp.on_hover_text(tip);
                    if resp.clicked() {
                        let mods = ui.input(|i| i.modifiers);
                        ed.click(&rows, i, mods);
                    }
                    // Clic droit hors de la sélection : la ligne devient la sélection (comme Mp3tag).
                    if resp.secondary_clicked() && !selected {
                        ed.click(&rows, i, egui::Modifiers::NONE);
                    }
                    resp.context_menu(|ui| {
                        // Illisibles compris : on doit pouvoir les retirer de la liste aussi.
                        let targets: Vec<usize> = if ed.selected[k] { ed.selection() } else { vec![k] };
                        let n = targets.len().max(1);
                        let dirty = targets.iter().filter(|&&t| ed.file_dirty(t)).count();
                        if dirty > 0 && ed.saving.is_none() {
                            let label = if n > 1 {
                                format!("Enregistrer ces {}", plural(dirty, "fichier", "fichiers"))
                            } else {
                                "Enregistrer ce fichier".to_owned()
                            };
                            if widgets::menu_item(ui, Some(Icon::Save), &label).clicked() {
                                action = Some(TableAction::SaveSelection);
                            }
                        }
                        if dirty > 0
                            && widgets::menu_item(ui, Some(Icon::Undo), "Rétablir les tags d'origine")
                                .clicked()
                        {
                            ed.revert(&targets);
                        }
                        if widgets::menu_item(ui, Some(Icon::FolderOpen), "Ouvrir le dossier").clicked() {
                            menu_open = ed.files[k].path.parent().map(Path::to_path_buf);
                        }
                        ui.separator();
                        let label = if n > 1 {
                            format!("Retirer {n} fichiers de la liste")
                        } else {
                            "Retirer de la liste".to_owned()
                        };
                        if widgets::menu_item(ui, Some(Icon::Close), &label).clicked() {
                            menu_remove = Some(targets);
                        }
                    });
                }
            });
    });
    if let Some(ks) = menu_remove {
        ed.remove(&ks);
    }
    if let Some(dir) = menu_open {
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }

    // Pied : filtre et récapitulatif.
    let foot = Rect::from_min_max(pos2(full.left(), rect.bottom() + 8.0), full.max);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(foot)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            widgets::search_field(
                ui,
                egui::Id::new("tag_filter"),
                &mut ed.filter,
                "Filtrer la liste",
                260.0,
            );
            ui.add_space(12.0);
            let total: u64 = ed.files.iter().map(|f| f.duration_ms).sum();
            let size: u64 = ed.files.iter().map(|f| f.size).sum();
            let sel = ed.selected.iter().filter(|&&s| s).count();
            let mut parts = vec![
                plural(ed.files.len(), "fichier", "fichiers"),
                library::fmt_long(Duration::from_millis(total)),
                size_text(size),
            ];
            if sel > 0 {
                parts.push(plural(sel, "sélectionné", "sélectionnés"));
            }
            let dirty = ed.dirty_count();
            if dirty > 0 {
                parts.push(format!("{dirty} à enregistrer"));
            }
            let text = parts.join(" · ");
            let g = widgets::galley(
                ui,
                &text,
                FontId::proportional(14.0),
                theme::TEXT_3,
                ui.available_width(),
            );
            ui.label(g).on_hover_text(text);
        },
    );
    action
}
