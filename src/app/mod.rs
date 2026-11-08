//! SterMusic : état de l'application, file d'attente, actions.

mod chrome;
mod lists;
mod nowbar;
mod views;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Key, ViewportCommand};
use serde::{Deserialize, Serialize};

use crate::backdrop::{self, Backdrop};
use crate::covers::Covers;
use crate::library::{self, Library, ScanMsg};
use crate::mpris::{self, Mpris, Remote};
use crate::player::Player;
use crate::theme;

// ---------- Réglages enregistrés ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Page {
    #[default]
    Home,
    Library,
    Queue,
    Playlists,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LibTab {
    #[default]
    Songs,
    Albums,
    Artists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SongSort {
    #[default]
    Title,
    Artist,
    Album,
    Added,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistId {
    Favorites,
    User(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Page(Page),
    Album(usize),
    Artist(usize),
    Playlist(PlaylistId),
    Search,
    NowPlaying,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    pub tracks: Vec<PathBuf>,
}

#[derive(Clone, Copy, Serialize, Deserialize, Default)]
pub struct PlayStat {
    pub count: u32,
    pub last: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub music_dirs: Vec<PathBuf>,
    /// Ancienne version (un seul dossier) : repris dans `music_dirs`.
    #[serde(skip_serializing)]
    pub music_dir: Option<PathBuf>,
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub theme: String,
    pub mica: bool,
    pub sidebar_collapsed: bool,
    pub resume: bool,
    pub page: Page,
    pub lib_tab: LibTab,
    pub song_sort: SongSort,
    pub queue: Vec<PathBuf>,
    pub queue_pos: Option<usize>,
    pub position: f32,
    pub playlists: Vec<Playlist>,
    pub favorites: HashSet<PathBuf>,
    pub plays: HashMap<PathBuf, PlayStat>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            music_dirs: Vec::new(),
            music_dir: None,
            volume: 0.8,
            muted: false,
            shuffle: false,
            repeat: Repeat::Off,
            theme: "stercode".into(),
            mica: true,
            sidebar_collapsed: false,
            resume: true,
            page: Page::Home,
            lib_tab: LibTab::Songs,
            song_sort: SongSort::Title,
            queue: Vec::new(),
            queue_pos: None,
            position: 0.0,
            playlists: Vec::new(),
            favorites: HashSet::new(),
            plays: HashMap::new(),
        }
    }
}

// ---------- Actions ----------

/// Tout ce que l'interface peut demander ; appliqué en fin de frame.
pub enum Action {
    Play { tracks: Vec<usize>, start: usize },
    PlayShuffled(Vec<usize>),
    PlayNext(Vec<usize>),
    Enqueue(Vec<usize>),
    Go(View),
    Back,
    ToggleFavorite(usize),
    AddToPlaylist(usize, Vec<usize>),
    NewPlaylist(Vec<usize>),
    RemoveFromPlaylist(PlaylistId, usize),
    RenamePlaylist(usize),
    DeletePlaylist(usize),
    ExportPlaylist(PlaylistId),
    ImportPlaylist,
    SaveQueueAsPlaylist,
    OpenFolder(PathBuf),
    QueueJump(usize),
    QueueRemove(usize),
    QueueMove(usize, usize),
    QueueClear,
    AddFolder,
    UseFolder(PathBuf),
    RemoveFolder(usize),
    Rescan,
    TogglePlay,
    Next,
    Previous,
    SeekTo(f32),
    SeekBy(f32),
    ToggleShuffle,
    CycleRepeat,
    SetVolume(f32),
    ToggleMute,
    ToggleMini,
    ToggleSidebar,
    SetTheme(&'static str),
    FocusSearch,
}

pub enum Dialog {
    NewPlaylist { name: String, tracks: Vec<usize> },
    Rename { index: usize, name: String },
    ConfirmDelete(usize),
}

/// Ce dont les listes ont besoin pour se dessiner (emprunts séparés de l'application).
pub struct Ctx<'a> {
    pub lib: &'a Library,
    pub covers: &'a mut Covers,
    pub actions: &'a mut Vec<Action>,
    pub settings: &'a Settings,
    pub selected: &'a mut Option<usize>,
    pub current: Option<usize>,
    pub playing: bool,
}

impl Ctx<'_> {
    pub fn go(&mut self, view: View) {
        self.actions.push(Action::Go(view));
    }
}

pub struct SterMusic {
    settings: Settings,
    player: Option<Player>,
    lib: Library,
    covers: Covers,
    mpris: Mpris,
    backdrop: Backdrop,

    queue: Vec<usize>,
    queue_pos: Option<usize>,
    /// Ordre d'origine de la file, pour revenir en arrière quand on coupe l'aléatoire.
    unshuffled: Option<Vec<usize>>,

    view: View,
    history: Vec<View>,
    search: String,
    search_results: (Vec<usize>, Vec<usize>, Vec<usize>),
    focus_search: bool,
    songs_sorted: Option<(SongSort, Vec<usize>)>,

    scan_rx: Option<Receiver<ScanMsg>>,
    scan_count: usize,
    restored: bool,

    actions: Vec<Action>,
    selected: Option<usize>,
    dialog: Option<Dialog>,
    toast: Option<(String, f64)>,
    queue_drag: Option<usize>,

    mini: bool,
    restore_size: Option<egui::Vec2>,
    window_title: String,
    seeked: bool,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Courbe de volume plus naturelle pour l'oreille qu'une échelle linéaire.
fn perceptual(v: f32) -> f32 {
    v * v
}

impl SterMusic {
    pub fn new(cc: &eframe::CreationContext<'_>, cli_dir: Option<PathBuf>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        if let Some(old) = settings.music_dir.take() {
            if !settings.music_dirs.contains(&old) {
                settings.music_dirs.push(old);
            }
        }
        if let Some(dir) = cli_dir.filter(|d| d.is_dir()) {
            let dir = std::path::absolute(&dir).unwrap_or(dir);
            if !settings.music_dirs.contains(&dir) {
                settings.music_dirs.push(dir);
            }
        }
        theme::install(&cc.egui_ctx, &settings.theme);

        let volume = if settings.muted {
            0.0
        } else {
            perceptual(settings.volume)
        };
        let (player, toast) = match Player::new(volume) {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some((e, 0.0))),
        };

        let page = settings.page;
        let mut app = Self {
            player,
            lib: Library::new(library::load_cache()),
            covers: Covers::new(&cc.egui_ctx),
            mpris: Mpris::new(&cc.egui_ctx),
            backdrop: Backdrop::default(),
            queue: Vec::new(),
            queue_pos: None,
            unshuffled: None,
            view: View::Page(page),
            history: Vec::new(),
            search: String::new(),
            search_results: Default::default(),
            focus_search: false,
            songs_sorted: None,
            scan_rx: None,
            scan_count: 0,
            restored: false,
            actions: Vec::new(),
            selected: None,
            dialog: None,
            toast,
            queue_drag: None,
            mini: false,
            restore_size: None,
            window_title: String::new(),
            seeked: false,
            settings,
        };
        app.restore_queue();
        app.start_scan();
        app
    }

    // ---------- Bibliothèque ----------

    fn start_scan(&mut self) {
        if self.scan_rx.is_some() || self.settings.music_dirs.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        let dirs = self.settings.music_dirs.clone();
        let cache = self
            .lib
            .tracks
            .iter()
            .map(|t| (t.path.clone(), t.clone()))
            .collect();
        std::thread::spawn(move || library::scan(dirs, cache, tx));
        self.scan_rx = Some(rx);
        self.scan_count = 0;
    }

    fn poll_scan(&mut self) {
        let Some(rx) = &self.scan_rx else { return };
        let mut finished = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                ScanMsg::Progress(n) => self.scan_count = n,
                ScanMsg::Done(tracks) => finished = Some(tracks),
            }
        }
        let Some(tracks) = finished else { return };
        self.scan_rx = None;

        // On garde la file d'attente et le morceau en cours à travers le rescan.
        let queue_paths: Vec<PathBuf> = self
            .queue
            .iter()
            .map(|&i| self.lib.tracks[i].path.clone())
            .collect();
        let current_path = self.current().map(|i| self.lib.tracks[i].path.clone());
        // Le scan ne rend pas les morceaux dans l'ordre de la bibliothèque : on compare par chemin.
        let changed = tracks.len() != self.lib.tracks.len()
            || tracks.iter().any(|t| {
                self.lib
                    .find(&t.path)
                    .is_none_or(|i| self.lib.tracks[i].mtime != t.mtime)
            });
        if !changed {
            return;
        }
        let count = tracks.len();
        self.lib = Library::new(tracks);
        self.queue = queue_paths
            .iter()
            .filter_map(|p| self.lib.find(p))
            .collect();
        self.queue_pos = current_path
            .and_then(|p| self.lib.find(&p))
            .and_then(|c| self.queue.iter().position(|&i| i == c));
        if self.queue_pos.is_none() {
            if let Some(p) = &mut self.player {
                p.stop();
            }
        }
        self.unshuffled = None;
        self.songs_sorted = None;
        self.selected = None;
        self.covers.clear();
        if matches!(self.view, View::Album(_) | View::Artist(_)) {
            self.view = View::Page(Page::Library);
            self.history.clear();
        }
        self.refresh_search();
        if !self.restored {
            self.restore_queue();
        }
        self.toast(format!(
            "Bibliothèque à jour : {}",
            library::plural(count, "morceau", "morceaux")
        ));
    }

    /// Reprend la file d'attente et la position du dernier lancement (en pause).
    fn restore_queue(&mut self) {
        if self.restored || self.lib.is_empty() {
            return;
        }
        self.restored = true;
        if !self.settings.resume {
            return;
        }
        let saved_pos = self
            .settings
            .queue_pos
            .and_then(|p| self.settings.queue.get(p).cloned());
        self.queue = self
            .settings
            .queue
            .iter()
            .filter_map(|p| self.lib.find(p))
            .collect();
        self.queue_pos = saved_pos
            .and_then(|p| self.lib.find(&p))
            .and_then(|c| self.queue.iter().position(|&i| i == c));
        if let (Some(idx), Some(player)) = (self.current(), &mut self.player) {
            if player.load(&self.lib.tracks[idx].path, true).is_ok() {
                let _ = player.seek(Duration::from_secs_f32(self.settings.position.max(0.0)));
            }
        }
    }

    fn refresh_search(&mut self) {
        self.search_results = self.lib.search(&self.search);
    }

    pub fn sorted_songs(&mut self) -> &[usize] {
        let sort = self.settings.song_sort;
        if self.songs_sorted.as_ref().map(|(s, _)| *s) != Some(sort) {
            let lib = &self.lib;
            let mut v: Vec<usize> = (0..lib.tracks.len()).collect();
            let key = |i: usize| lib.tracks[i].title.to_lowercase();
            match sort {
                SongSort::Title => v.sort_by_cached_key(|&i| key(i)),
                SongSort::Artist => {} // ordre naturel de la bibliothèque : artiste, album, piste
                SongSort::Album => v.sort_by_cached_key(|&i| {
                    (lib.tracks[i].album.to_lowercase(), lib.tracks[i].track_no)
                }),
                SongSort::Added => v.sort_by_key(|&i| std::cmp::Reverse(lib.tracks[i].mtime)),
            }
            self.songs_sorted = Some((sort, v));
        }
        &self.songs_sorted.as_ref().unwrap().1
    }

    // ---------- Lecture ----------

    pub fn current(&self) -> Option<usize> {
        self.queue_pos.and_then(|p| self.queue.get(p).copied())
    }

    fn is_playing(&self) -> bool {
        self.player.as_ref().is_some_and(Player::is_playing)
    }

    fn play_current(&mut self) {
        let Some(idx) = self.current() else { return };
        let Some(player) = &mut self.player else {
            return;
        };
        let track = &self.lib.tracks[idx];
        match player.load(&track.path, false) {
            Ok(()) => {
                let stat = self.settings.plays.entry(track.path.clone()).or_default();
                stat.count += 1;
                stat.last = now_secs();
            }
            Err(e) => {
                let msg = format!("{} — {e}", track.title);
                self.toast(msg);
            }
        }
        self.seeked = true;
    }

    fn play_list(&mut self, tracks: Vec<usize>, start: usize) {
        if tracks.is_empty() {
            return;
        }
        self.queue = tracks;
        self.queue_pos = Some(start.min(self.queue.len() - 1));
        self.unshuffled = None;
        if self.settings.shuffle {
            self.shuffle_queue();
        }
        self.play_current();
    }

    /// Mélange la file en gardant le morceau en cours en tête.
    fn shuffle_queue(&mut self) {
        let Some(pos) = self.queue_pos else { return };
        self.unshuffled = Some(self.queue.clone());
        let cur = self.queue.remove(pos);
        fastrand::shuffle(&mut self.queue);
        self.queue.insert(0, cur);
        self.queue_pos = Some(0);
    }

    fn unshuffle_queue(&mut self) {
        if let Some(original) = self.unshuffled.take() {
            let cur = self.current();
            self.queue = original;
            self.queue_pos = cur.and_then(|c| self.queue.iter().position(|&i| i == c));
        }
    }

    fn toggle_play(&mut self) {
        let has_track = self.player.as_ref().is_some_and(Player::has_track);
        if has_track {
            if let Some(p) = &self.player {
                if p.is_paused() {
                    p.play();
                } else {
                    p.pause();
                }
            }
        } else if self.current().is_some() {
            self.play_current();
        } else if !self.lib.is_empty() {
            let all: Vec<usize> = self.sorted_songs().to_vec();
            self.play_list(all, 0);
        }
    }

    /// Morceau suivant ; `auto` quand le précédent s'est terminé tout seul.
    fn next(&mut self, auto: bool) {
        if auto && self.settings.repeat == Repeat::One {
            self.play_current();
            return;
        }
        let Some(pos) = self.queue_pos else { return };
        if pos + 1 < self.queue.len() {
            self.queue_pos = Some(pos + 1);
        } else if !auto || self.settings.repeat == Repeat::All {
            if self.settings.shuffle && self.queue.len() > 1 {
                fastrand::shuffle(&mut self.queue);
            }
            self.queue_pos = Some(0);
        } else {
            // Fin de la file : on s'arrête sur le dernier morceau.
            if let Some(p) = &mut self.player {
                p.stop();
            }
            return;
        }
        self.play_current();
    }

    fn previous(&mut self) {
        if let Some(p) = &self.player {
            if p.has_track() && p.position() > Duration::from_secs(3) {
                let _ = p.seek(Duration::ZERO);
                self.seeked = true;
                return;
            }
        }
        let Some(pos) = self.queue_pos else { return };
        self.queue_pos = Some(if pos > 0 {
            pos - 1
        } else if self.settings.repeat == Repeat::All {
            self.queue.len() - 1
        } else {
            0
        });
        self.play_current();
    }

    fn seek_to(&mut self, secs: f32) {
        let Some(p) = &self.player else { return };
        if !p.has_track() {
            return;
        }
        let max = self
            .current()
            .map(|i| self.lib.tracks[i].duration().as_secs_f32())
            .unwrap_or(f32::MAX);
        if let Err(e) = p.seek(Duration::from_secs_f32(secs.clamp(0.0, max.max(0.0)))) {
            self.toast(e);
        }
        self.seeked = true;
    }

    fn apply_volume(&mut self) {
        let v = if self.settings.muted {
            0.0
        } else {
            perceptual(self.settings.volume)
        };
        if let Some(p) = &mut self.player {
            p.set_volume(v);
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), -1.0));
    }

    // ---------- Navigation ----------

    fn go(&mut self, view: View) {
        if view == self.view {
            return;
        }
        if let View::Page(p) = view {
            self.settings.page = p;
            // Une page principale repart d'un historique vide, comme dans le lecteur de Windows.
            self.history.clear();
        } else {
            self.history.push(self.view.clone());
        }
        if view != View::Search && self.view == View::Search {
            self.search.clear();
        }
        self.view = view;
        self.selected = None;
    }

    fn back(&mut self) {
        if let Some(v) = self.history.pop() {
            self.view = v;
        } else if !matches!(self.view, View::Page(_)) {
            self.view = View::Page(self.settings.page);
        }
        if self.view != View::Search {
            self.search.clear();
        }
    }

    pub fn playlist_tracks(&self, id: PlaylistId) -> Vec<usize> {
        match id {
            PlaylistId::Favorites => {
                let mut v: Vec<usize> = self
                    .settings
                    .favorites
                    .iter()
                    .filter_map(|p| self.lib.find(p))
                    .collect();
                v.sort_unstable();
                v
            }
            PlaylistId::User(i) => self
                .settings
                .playlists
                .get(i)
                .map(|pl| pl.tracks.iter().filter_map(|p| self.lib.find(p)).collect())
                .unwrap_or_default(),
        }
    }

    pub fn playlist_name(&self, id: PlaylistId) -> String {
        match id {
            PlaylistId::Favorites => "Favoris".into(),
            PlaylistId::User(i) => self
                .settings
                .playlists
                .get(i)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
        }
    }

    fn paths(&self, tracks: &[usize]) -> Vec<PathBuf> {
        tracks
            .iter()
            .map(|&i| self.lib.tracks[i].path.clone())
            .collect()
    }

    // ---------- Application des actions ----------

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Play { tracks, start } => self.play_list(tracks, start),
            Action::PlayShuffled(tracks) => {
                if tracks.is_empty() {
                    return;
                }
                self.settings.shuffle = true;
                let start = fastrand::usize(..tracks.len());
                self.play_list(tracks, start);
            }
            Action::PlayNext(tracks) => {
                let n = tracks.len();
                match self.queue_pos {
                    Some(pos) => {
                        for (k, t) in tracks.into_iter().enumerate() {
                            self.queue.insert(pos + 1 + k, t);
                        }
                        self.unshuffled = None;
                        self.toast(format!(
                            "{} lu{} ensuite",
                            library::plural(n, "morceau sera", "morceaux seront"),
                            if n > 1 { "s" } else { "" }
                        ));
                    }
                    None => self.play_list(tracks, 0),
                }
            }
            Action::Enqueue(tracks) => {
                let n = tracks.len();
                if self.queue.is_empty() {
                    self.queue = tracks;
                    self.queue_pos = Some(0);
                    self.play_current();
                } else {
                    self.queue.extend(tracks);
                    self.unshuffled = None;
                }
                self.toast(format!(
                    "{} ajouté{} à la file d'attente",
                    library::plural(n, "morceau", "morceaux"),
                    if n > 1 { "s" } else { "" }
                ));
            }
            Action::Go(view) => {
                // Rejoue le fondu d'apparition de la page.
                crate::anim::reset(ctx, egui::Id::new(("page_fade", format!("{view:?}"))));
                self.go(view);
            }
            Action::Back => {
                self.back();
                crate::anim::reset(
                    ctx,
                    egui::Id::new(("page_fade", format!("{:?}", self.view))),
                );
            }
            Action::ToggleFavorite(i) => {
                let path = self.lib.tracks[i].path.clone();
                if !self.settings.favorites.remove(&path) {
                    self.settings.favorites.insert(path);
                }
            }
            Action::AddToPlaylist(pl, tracks) => {
                let paths = self.paths(&tracks);
                if let Some(p) = self.settings.playlists.get_mut(pl) {
                    p.tracks.extend(paths);
                    let msg = format!("Ajouté à « {} »", p.name);
                    self.toast(msg);
                }
            }
            Action::NewPlaylist(tracks) => {
                let name = format!("Playlist {}", self.settings.playlists.len() + 1);
                self.dialog = Some(Dialog::NewPlaylist { name, tracks });
            }
            Action::RemoveFromPlaylist(id, pos) => match id {
                PlaylistId::Favorites => {
                    let tracks = self.playlist_tracks(id);
                    if let Some(&t) = tracks.get(pos) {
                        let p = self.lib.tracks[t].path.clone();
                        self.settings.favorites.remove(&p);
                    }
                }
                PlaylistId::User(i) => {
                    // `pos` compte les morceaux présents ; on retrouve l'entrée correspondante.
                    let lib = &self.lib;
                    if let Some(pl) = self.settings.playlists.get_mut(i) {
                        let real = pl
                            .tracks
                            .iter()
                            .enumerate()
                            .filter(|(_, p)| lib.find(p).is_some())
                            .nth(pos)
                            .map(|(k, _)| k);
                        if let Some(k) = real {
                            pl.tracks.remove(k);
                        }
                    }
                }
            },
            Action::RenamePlaylist(i) => {
                if let Some(p) = self.settings.playlists.get(i) {
                    self.dialog = Some(Dialog::Rename {
                        index: i,
                        name: p.name.clone(),
                    });
                }
            }
            Action::DeletePlaylist(i) => self.dialog = Some(Dialog::ConfirmDelete(i)),
            Action::ExportPlaylist(id) => self.export_playlist(id),
            Action::ImportPlaylist => self.import_playlist(),
            Action::SaveQueueAsPlaylist => {
                let tracks = self.queue.clone();
                self.apply(ctx, Action::NewPlaylist(tracks));
            }
            Action::OpenFolder(path) => {
                let _ = std::process::Command::new("xdg-open").arg(path).spawn();
            }
            Action::QueueJump(pos) => {
                if pos < self.queue.len() {
                    self.queue_pos = Some(pos);
                    self.play_current();
                }
            }
            Action::QueueRemove(pos) => {
                if pos >= self.queue.len() {
                    return;
                }
                self.queue.remove(pos);
                self.unshuffled = None;
                match self.queue_pos {
                    Some(cur) if pos < cur => self.queue_pos = Some(cur - 1),
                    Some(cur) if pos == cur => {
                        if self.queue.is_empty() {
                            self.queue_pos = None;
                            if let Some(p) = &mut self.player {
                                p.stop();
                            }
                        } else {
                            self.queue_pos = Some(cur.min(self.queue.len() - 1));
                            if self.is_playing() {
                                self.play_current();
                            } else if let Some(p) = &mut self.player {
                                p.stop();
                            }
                        }
                    }
                    _ => {}
                }
            }
            Action::QueueMove(from, to) => {
                if from >= self.queue.len() || from == to {
                    return;
                }
                let cur = self.current();
                let item = self.queue.remove(from);
                let to = if to > from { to - 1 } else { to }.min(self.queue.len());
                self.queue.insert(to, item);
                self.unshuffled = None;
                // Le morceau en cours garde sa place dans la lecture.
                if let Some(pos) = self.queue_pos {
                    self.queue_pos = Some(if from == pos {
                        to
                    } else {
                        self.queue
                            .iter()
                            .position(|&i| Some(i) == cur)
                            .unwrap_or(pos)
                    });
                }
            }
            Action::QueueClear => {
                self.queue.clear();
                self.queue_pos = None;
                self.unshuffled = None;
                if let Some(p) = &mut self.player {
                    p.stop();
                }
            }
            Action::AddFolder => {
                if let Some(dir) = rfd::FileDialog::new()
                    .set_title("Ajouter un dossier de musique")
                    .pick_folder()
                {
                    self.apply(ctx, Action::UseFolder(dir));
                }
            }
            Action::UseFolder(dir) => {
                if !self.settings.music_dirs.contains(&dir) {
                    self.settings.music_dirs.push(dir);
                    self.start_scan();
                }
            }
            Action::RemoveFolder(i) => {
                if i < self.settings.music_dirs.len() {
                    self.settings.music_dirs.remove(i);
                    if self.settings.music_dirs.is_empty() {
                        self.lib = Library::default();
                        self.queue.clear();
                        self.queue_pos = None;
                        self.songs_sorted = None;
                        if let Some(p) = &mut self.player {
                            p.stop();
                        }
                    } else {
                        self.start_scan();
                    }
                }
            }
            Action::Rescan => self.start_scan(),
            Action::TogglePlay => self.toggle_play(),
            Action::Next => self.next(false),
            Action::Previous => self.previous(),
            Action::SeekTo(s) => self.seek_to(s),
            Action::SeekBy(d) => {
                let pos = self
                    .player
                    .as_ref()
                    .map(|p| p.position().as_secs_f32())
                    .unwrap_or(0.0);
                self.seek_to(pos + d);
            }
            Action::ToggleShuffle => {
                self.settings.shuffle = !self.settings.shuffle;
                if self.settings.shuffle {
                    self.shuffle_queue();
                } else {
                    self.unshuffle_queue();
                }
            }
            Action::CycleRepeat => {
                self.settings.repeat = match self.settings.repeat {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                };
            }
            Action::SetVolume(v) => {
                self.settings.volume = v.clamp(0.0, 1.0);
                self.settings.muted = false;
                self.apply_volume();
            }
            Action::ToggleMute => {
                self.settings.muted = !self.settings.muted;
                self.apply_volume();
            }
            Action::ToggleMini => self.toggle_mini(ctx),
            Action::ToggleSidebar => {
                self.settings.sidebar_collapsed = !self.settings.sidebar_collapsed
            }
            Action::SetTheme(id) => {
                self.settings.theme = id.to_owned();
                theme::set(ctx, id);
            }
            Action::FocusSearch => {
                self.settings.sidebar_collapsed = false;
                self.focus_search = true;
            }
        }
    }

    fn toggle_mini(&mut self, ctx: &egui::Context) {
        self.mini = !self.mini;
        if self.mini {
            self.restore_size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(egui::vec2(320.0, 132.0)));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(400.0, 148.0)));
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
        } else {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(egui::vec2(760.0, 520.0)));
            let size = self.restore_size.unwrap_or(egui::vec2(1200.0, 780.0));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
    }

    fn export_playlist(&mut self, id: PlaylistId) {
        let name = self.playlist_name(id);
        let Some(file) = rfd::FileDialog::new()
            .set_title("Exporter la playlist")
            .set_file_name(format!("{name}.m3u"))
            .add_filter("Playlist M3U", &["m3u", "m3u8"])
            .save_file()
        else {
            return;
        };
        let mut out = String::from("#EXTM3U\n");
        for i in self.playlist_tracks(id) {
            let t = &self.lib.tracks[i];
            out.push_str(&format!(
                "#EXTINF:{},{} - {}\n{}\n",
                t.duration().as_secs(),
                t.artist_or_unknown(),
                t.title,
                t.path.display()
            ));
        }
        match std::fs::write(&file, out) {
            Ok(()) => self.toast(format!("Playlist exportée : {}", file.display())),
            Err(e) => self.toast(format!("Export impossible : {e}")),
        }
    }

    fn import_playlist(&mut self) {
        let Some(file) = rfd::FileDialog::new()
            .set_title("Importer une playlist")
            .add_filter("Playlist M3U", &["m3u", "m3u8"])
            .pick_file()
        else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            self.toast("Lecture du fichier impossible");
            return;
        };
        let base = file.parent().map(PathBuf::from).unwrap_or_default();
        let tracks: Vec<PathBuf> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let p = PathBuf::from(l.trim_start_matches("file://"));
                if p.is_absolute() {
                    p
                } else {
                    base.join(p)
                }
            })
            .collect();
        let found = tracks.iter().filter(|p| self.lib.find(p).is_some()).count();
        let name = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Playlist".into());
        self.settings.playlists.push(Playlist {
            name: name.clone(),
            tracks,
        });
        self.toast(format!(
            "« {name} » importée ({found} morceaux trouvés dans la bibliothèque)"
        ));
    }

    // ---------- Entrées ----------

    fn handle_remote(&mut self, ctx: &egui::Context) {
        for remote in self.mpris.poll() {
            let action = match remote {
                Remote::Play => {
                    if !self.is_playing() {
                        Action::TogglePlay
                    } else {
                        continue;
                    }
                }
                Remote::Pause => {
                    if self.is_playing() {
                        Action::TogglePlay
                    } else {
                        continue;
                    }
                }
                Remote::Toggle => Action::TogglePlay,
                Remote::Next => Action::Next,
                Remote::Previous => Action::Previous,
                Remote::Stop => {
                    if let Some(p) = &mut self.player {
                        p.stop();
                    }
                    continue;
                }
                Remote::SeekBy(d) => Action::SeekBy(d),
                Remote::SetPosition(d) => Action::SeekTo(d.as_secs_f32()),
                Remote::Raise => {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                    continue;
                }
            };
            self.actions.push(action);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for path in dropped {
            if path.is_dir() {
                self.actions.push(Action::UseFolder(path));
            }
        }
        let cmd = egui::Modifiers::COMMAND;
        let none = egui::Modifiers::NONE;
        if ctx.input_mut(|i| i.consume_key(cmd, Key::F)) {
            self.actions.push(Action::FocusSearch);
        }
        if ctx.input_mut(|i| i.consume_key(cmd, Key::M)) {
            self.actions.push(Action::ToggleMini);
        }
        if ctx.wants_keyboard_input() || self.dialog.is_some() {
            return;
        }
        let keys = [
            (none, Key::Space, Action::TogglePlay),
            (cmd, Key::ArrowRight, Action::Next),
            (cmd, Key::ArrowLeft, Action::Previous),
            (none, Key::ArrowRight, Action::SeekBy(5.0)),
            (none, Key::ArrowLeft, Action::SeekBy(-5.0)),
            (none, Key::Escape, Action::Back),
        ];
        for (m, k, action) in keys {
            if ctx.input_mut(|i| i.consume_key(m, k)) {
                self.actions.push(action);
            }
        }
        let vol = self.settings.volume;
        if ctx.input_mut(|i| i.consume_key(cmd, Key::ArrowUp)) {
            self.actions.push(Action::SetVolume(vol + 0.05));
        }
        if ctx.input_mut(|i| i.consume_key(cmd, Key::ArrowDown)) {
            self.actions.push(Action::SetVolume(vol - 0.05));
        }
    }

    // ---------- Synchronisation avec le bureau ----------

    fn sync_desktop(&mut self, ctx: &egui::Context) {
        let current = self
            .current()
            .filter(|_| self.player.as_ref().is_some_and(Player::has_track));
        let cover_file = current.and_then(|i| {
            let path = self.lib.tracks[i].path.clone();
            self.covers.large(&path).and_then(|l| l.file.clone())
        });
        self.mpris
            .set_track(current.map(|i| &self.lib.tracks[i]), cover_file.as_deref());
        let (state, pos) = match &self.player {
            Some(p) if p.has_track() => (
                if p.is_paused() {
                    mpris::State::Paused
                } else {
                    mpris::State::Playing
                },
                p.position(),
            ),
            _ => (mpris::State::Stopped, Duration::ZERO),
        };
        self.mpris
            .set_state(state, pos, std::mem::take(&mut self.seeked));

        let title = match current.map(|i| &self.lib.tracks[i]) {
            Some(t) => format!("{} — {} · SterMusic", t.title, t.artist_or_unknown()),
            None => "SterMusic".to_owned(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    fn view_ctx(&mut self) -> Ctx<'_> {
        let current = self.current();
        let playing = self.is_playing();
        Ctx {
            lib: &self.lib,
            covers: &mut self.covers,
            actions: &mut self.actions,
            settings: &self.settings,
            selected: &mut self.selected,
            current,
            playing,
        }
    }
}

impl eframe::App for SterMusic {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_scan();
        self.covers.poll(ctx);
        self.handle_remote(ctx);
        if self.player.as_ref().is_some_and(Player::finished) {
            self.next(true);
        }
        self.handle_keys(ctx);

        self.backdrop.paint(ctx, self.settings.mica);
        if self.view == View::NowPlaying || self.mini {
            self.paint_cover_backdrop(ctx);
        }

        if self.mini {
            self.mini_player(ctx);
        } else {
            let resize = chrome::resize_direction(ctx);
            self.title_bar(ctx);
            self.now_bar(ctx);
            if self.view != View::NowPlaying {
                self.sidebar(ctx);
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| self.content(ui));
            self.dialogs(ctx);
            self.toast_ui(ctx);
            chrome::apply_resize(ctx, resize);
        }

        let actions = std::mem::take(&mut self.actions);
        for action in actions {
            self.apply(ctx, action);
        }
        self.sync_desktop(ctx);

        if self.scan_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(150));
        } else if self.is_playing() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.queue = self.paths(&self.queue);
        self.settings.queue_pos = self.queue_pos;
        self.settings.position = self
            .player
            .as_ref()
            .map(|p| p.position().as_secs_f32())
            .unwrap_or(0.0);
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }
}

impl SterMusic {
    /// Fond de « En cours de lecture » et du mini lecteur : la pochette, floutée et assombrie.
    fn paint_cover_backdrop(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.current() else { return };
        let path = self.lib.tracks[idx].path.clone();
        let blurred = self.covers.large(&path).and_then(|l| l.blurred.clone());
        let t = crate::anim::toggle(
            ctx,
            egui::Id::new("cover_backdrop"),
            blurred.is_some(),
            crate::anim::SLOW,
        );
        if let Some(tex) = blurred {
            let painter = ctx.layer_painter(egui::LayerId::background());
            let screen = ctx.screen_rect();
            backdrop::paint_fill(
                &painter,
                &tex,
                screen,
                egui::Color32::from_gray(150).gamma_multiply(t),
            );
            painter.rect_filled(
                screen,
                0.0,
                theme::alpha(theme::bg(), (0x70 as f32 * t) as u8),
            );
        }
    }
}
