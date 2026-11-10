//! SterMusic : état de l'application, file d'attente, actions.

mod browse;
mod chrome;
mod cover_tool;
mod dup_tool;
mod folders;
mod lib_tools;
mod lists;
mod lyrics_view;
mod mb_import;
mod nowbar;
mod radio_view;
mod rg_tool;
mod settings_extra;
mod smart;
mod stats;
mod tag_editor;
mod tag_tools;
mod views;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Key, ViewportCommand};
use serde::{Deserialize, Serialize};

use crate::backdrop::{self, Backdrop};
use crate::covers::Covers;
use crate::i18n::tr;
use crate::library::{self, Library, ScanMsg};
use crate::mpris::{self, Mpris, Remote};
use crate::player::Player;
use crate::theme;
use crate::trf;

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
    /// Éditeur de tags.
    Tags,
    /// Statistiques d'écoute.
    Stats,
    /// Webradios.
    Radio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LibTab {
    #[default]
    Songs,
    Albums,
    Artists,
    /// La bibliothèque rangée comme sur le disque.
    Folders,
    /// Par genre, par année ou par compositeur.
    Browse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SongSort {
    #[default]
    Title,
    Artist,
    Album,
    Added,
}

/// Fond de la fenêtre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BackdropKind {
    Solid,
    /// Mica simulé (fond d'écran flouté et teinté).
    #[default]
    Mica,
    /// Fenêtre transparente, floutée par KWin.
    Transparent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistId {
    Favorites,
    User(usize),
    /// Playlist intelligente (remplie d'après des règles).
    Smart(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Page(Page),
    Album(usize),
    Artist(usize),
    Playlist(PlaylistId),
    Search,
    NowPlaying,
    /// Récap d'une année d'écoute.
    Recap(i32),
    /// Un genre, une année ou un compositeur.
    Browse(browse::BrowseKind, String),
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
    /// Secondes réellement écoutées (depuis que SterMusic les mesure).
    #[serde(default)]
    pub seconds: f64,
}

/// Écoute en cours : le morceau, ce qui en a été entendu, et si elle compte déjà.
pub(crate) struct Listening {
    path: PathBuf,
    /// Déjà envoyée aux services de scrobbling.
    scrobbled: bool,
    started: u64,
    heard: f32,
    last_pos: f32,
    /// Moment du dernier relevé : la lecture n'a pas pu avancer de plus que le temps écoulé.
    last_tick: std::time::Instant,
    counted: bool,
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
    /// Ancien réglage (avant les fonds au choix) : repris dans `backdrop`.
    pub mica: bool,
    pub backdrop: BackdropKind,
    /// Intensité du Mica, ou opacité en mode transparent (0 → 1).
    pub mica_strength: f32,
    /// Style Breeze (Kubuntu) : police, icônes et boutons de fenêtre de KDE.
    pub breeze: bool,
    /// Coins arrondis de la fenêtre.
    pub rounded: bool,
    /// Version des réglages d'apparence (migrations).
    pub style_version: u32,
    /// Égaliseur graphique 10 bandes.
    pub eq_on: bool,
    pub eq_gains: [f32; 10],
    pub eq_preamp: f32,
    pub eq_preset: String,
    /// Fondu enchaîné entre les morceaux, et sa durée (s).
    pub crossfade: bool,
    pub crossfade_secs: f32,
    /// Nivellement du volume (ReplayGain, sinon automatique).
    pub leveling: bool,
    /// Visualiseur (animations qui suivent la musique).
    pub visualizer: bool,
    /// Visualiseur de « En cours de lecture » et du bandeau de l'accueil (voir `shader::STYLES`).
    pub viz_style: usize,
    pub banner_style: usize,
    /// Visualiseurs exclus du mode automatique (par leur nom).
    pub auto_excluded: Vec<String>,
    /// Éditeur de tags : ID3v2.3 plutôt que 2.4, et date de modification conservée.
    pub tag_id3v23: bool,
    pub tag_keep_mtime: bool,
    /// Éditeur de tags : largeur des colonnes, derniers motifs de renommage.
    pub tag_prefs: tag_editor::Prefs,
    /// Thèmes enregistrés en favori (couleurs figées).
    pub saved_themes: Vec<theme::SavedTheme>,
    /// Langue de l'interface (par défaut, celle du bureau).
    pub lang: crate::i18n::Lang,
    /// Playlists intelligentes, et vrai une fois les quatre proposées d'office ajoutées.
    pub smart_playlists: Vec<smart::SmartPlaylist>,
    pub smart_seeded: bool,
    pub stats_period: stats::Period,
    /// Paroles : chercher sur LRCLIB, et les montrer dans « En cours de lecture ».
    pub lyrics_online: bool,
    pub show_lyrics: bool,
    /// Bibliothèque mise à jour toute seule (surveillance des dossiers).
    pub auto_update: bool,
    pub sidebar_collapsed: bool,
    /// Bibliothèque, onglet « Parcourir » : genres, années ou compositeurs.
    pub browse_kind: browse::BrowseKind,
    /// Webradios enregistrées (« Mes stations »).
    pub radios: Vec<crate::radio::Station>,
    /// Sortie audio choisie (nom PipeWire ; vide : celle du système).
    pub audio_output: String,
    /// Notification du bureau au changement de morceau (quand la fenêtre n'est pas au premier plan).
    pub notify_tracks: bool,
    /// Scrobbling (ListenBrainz, Last.fm).
    pub scrobble: crate::scrobble::Config,
    /// Minuterie de sommeil : durée du fondu (s).
    pub sleep_fade_secs: f32,
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
            backdrop: BackdropKind::Mica,
            mica_strength: 0.6,
            breeze: true,
            rounded: true,
            style_version: 0,
            eq_on: false,
            eq_gains: [0.0; 10],
            eq_preamp: 0.0,
            eq_preset: "Plat".into(),
            crossfade: false,
            crossfade_secs: 5.0,
            leveling: false,
            visualizer: true,
            viz_style: 0,
            banner_style: 1,
            auto_excluded: Vec::new(),
            tag_id3v23: false,
            tag_keep_mtime: true,
            tag_prefs: Default::default(),
            saved_themes: Vec::new(),
            lang: crate::i18n::Lang::from_env(),
            smart_playlists: Vec::new(),
            smart_seeded: false,
            stats_period: stats::Period::default(),
            lyrics_online: true,
            show_lyrics: true,
            auto_update: true,
            sidebar_collapsed: false,
            browse_kind: Default::default(),
            radios: Vec::new(),
            audio_output: String::new(),
            notify_tracks: true,
            scrobble: Default::default(),
            sleep_fade_secs: 30.0,
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
    Play {
        tracks: Vec<usize>,
        start: usize,
    },
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
    SetLang(crate::i18n::Lang),
    /// Enregistre le thème en cours en favori (demande son nom).
    SaveTheme,
    RenameTheme(String),
    DeleteTheme(String),
    SetBreeze(bool),
    /// Ajoute SterMusic aux fenêtres floutées par l'effet Better Blur de KWin.
    EnableKwinBlur,
    FocusSearch,
    /// Ouvre l'éditeur de tags sur ces fichiers (de la bibliothèque ou non).
    EditTags(Vec<PathBuf>),
    /// Joue ces fichiers, même hors bibliothèque (double-clic dans Dolphin).
    PlayFiles(Vec<PathBuf>),
    /// Choisit des fichiers à éditer, hors bibliothèque compris.
    PickTagFiles,
    /// Surveillance des dossiers de musique (bibliothèque mise à jour toute seule).
    SetAutoUpdate(bool),
    NewSmartPlaylist,
    EditSmartPlaylist(usize),
    /// Exporter les statistiques en CSV : par morceau (vrai) ou par écoute.
    ExportStats(bool),
    ClearStats,
    /// Ouvre un outil d'entretien de la bibliothèque (pochettes manquantes, ReplayGain, doublons).
    LibTool(lib_tools::LibTool),
    /// Ouvre le calcul du ReplayGain sur ces morceaux (le libellé dit ce que c'est).
    ReplayGain(Vec<PathBuf>, String),
    /// Écoute une webradio.
    PlayStation(crate::radio::Station),
    /// Ajoute une station à « Mes stations », ou l'en retire.
    ToggleStation(crate::radio::Station),
    /// Minuterie de sommeil (ou `None` pour l'arrêter).
    SetSleep(Option<SleepKind>),
    /// Sortie audio (nom PipeWire ; vide : celle du système).
    SetOutput(String),
    /// Les réglages de scrobbling ont changé.
    ScrobbleConfig,
}

/// Minuterie de sommeil : au bout de quelques minutes, ou à la fin du morceau en cours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SleepKind {
    Minutes(u32),
    EndOfTrack,
}

pub struct Sleep {
    pub kind: SleepKind,
    /// Heure de l'arrêt (minuterie en minutes).
    pub end: std::time::Instant,
}

/// Morceaux sélectionnés dans une liste (Ctrl+clic, Maj+clic).
#[derive(Default)]
pub struct Selection {
    pub items: Vec<usize>,
    /// Point de départ d'une sélection Maj+clic.
    pub anchor: Option<usize>,
}

impl Selection {
    pub fn contains(&self, idx: usize) -> bool {
        self.items.contains(&idx)
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.anchor = None;
    }

    /// Clic sur la ligne `i` de `list`, selon les touches enfoncées.
    pub fn click(&mut self, list: &[usize], i: usize, mods: egui::Modifiers) {
        let idx = list[i];
        if mods.shift {
            let from = self
                .anchor
                .and_then(|a| list.iter().position(|&t| t == a))
                .unwrap_or(i);
            let (a, b) = (from.min(i), from.max(i));
            if !mods.command {
                self.items.clear();
            }
            for &t in &list[a..=b] {
                if !self.items.contains(&t) {
                    self.items.push(t);
                }
            }
        } else if mods.command {
            match self.items.iter().position(|&t| t == idx) {
                Some(k) => {
                    self.items.remove(k);
                }
                None => self.items.push(idx),
            }
            self.anchor = Some(idx);
        } else {
            self.items = vec![idx];
            self.anchor = Some(idx);
        }
    }

    /// Les morceaux visés par un menu ouvert sur `idx` : la sélection si `idx` en fait partie.
    pub fn targets(&self, idx: usize) -> Vec<usize> {
        if self.items.len() > 1 && self.contains(idx) {
            let mut v = self.items.clone();
            v.sort_unstable();
            v
        } else {
            vec![idx]
        }
    }
}

pub enum Dialog {
    /// Enregistrer le thème en cours en favori (`id: None`), ou renommer un favori.
    ThemeName {
        id: Option<String>,
        name: String,
    },
    /// Supprimer un thème favori.
    DeleteTheme(String),
    /// Effacer toutes les statistiques d'écoute.
    ClearStats,
    NewPlaylist {
        name: String,
        tracks: Vec<usize>,
    },
    Rename {
        index: usize,
        name: String,
    },
    ConfirmDelete(usize),
}

/// Ce dont les listes ont besoin pour se dessiner (emprunts séparés de l'application).
pub struct Ctx<'a> {
    pub lib: &'a Library,
    pub covers: &'a mut Covers,
    pub actions: &'a mut Vec<Action>,
    pub settings: &'a Settings,
    pub selected: &'a mut Selection,
    pub current: Option<usize>,
    pub playing: bool,
    /// Un morceau est chargé (en lecture ou en pause).
    pub has_track: bool,
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
    kwin: crate::kwin::KWin,
    backdrop: Backdrop,

    queue: Vec<usize>,
    queue_pos: Option<usize>,
    /// Ordre d'origine de la file, pour revenir en arrière quand on coupe l'aléatoire.
    unshuffled: Option<Vec<usize>>,

    view: View,
    history: Vec<View>,
    search: String,
    /// Recherche dans la page d'un album ou d'un artiste.
    detail_filter: String,
    /// Recherche dans la file d'attente.
    queue_filter: String,
    /// Filtre de la bibliothèque (morceaux, albums ou artistes).
    lib_filter: String,
    search_results: (Vec<usize>, Vec<usize>, Vec<usize>),
    focus_search: bool,
    songs_sorted: Option<(SongSort, Vec<usize>)>,

    scan_rx: Option<Receiver<ScanMsg>>,
    scan_count: usize,
    restored: bool,

    actions: Vec<Action>,
    selected: Selection,
    dialog: Option<Dialog>,
    tag_editor: tag_editor::TagEditor,
    /// Fichiers à éditer envoyés par une autre instance (`stermusic --tags`, menu de Dolphin).
    ipc: crate::ipc::Ipc,
    /// Fichiers glissés sur la fenêtre sous Wayland.
    dnd: crate::dnd::Dnd,
    /// Morceaux ouverts hors des dossiers de la bibliothèque (double-clic dans Dolphin) : ils
    /// restent dans la bibliothèque jusqu'à la fermeture, rescans compris.
    external: Vec<library::Track>,
    /// Outils d'entretien de la bibliothèque.
    lib_tools: lib_tools::LibTools,
    /// Historique d'écoute daté (statistiques, récap).
    listens: Vec<crate::history::Listen>,
    /// Morceau en cours d'écoute, et vrai une fois son écoute comptée (30 secondes).
    listening: Option<Listening>,
    smart_edit: Option<smart::Edit>,
    /// Morceaux des playlists intelligentes, recalculés au plus toutes les deux secondes.
    smart_cache: std::cell::RefCell<HashMap<usize, SmartCached>>,
    /// Vue par dossiers : l'index (reconstruit avec la bibliothèque) et le dossier ouvert.
    folder_index: Option<folders::FolderIndex>,
    lib_folder: Option<PathBuf>,
    lyrics: lyrics_view::LyricsState,
    /// Surveillance des dossiers de musique, et mise à jour en cours.
    watcher: crate::watch::Watcher,
    watch_rx: Option<Receiver<WatchDone>>,
    /// Fichier que le lecteur a ouvert, connu des threads qui écrivent des tags : celui-là est
    /// écrit via une copie (une écriture sur place décalerait le son sous le décodeur).
    now_loaded: std::sync::Arc<std::sync::Mutex<Option<PathBuf>>>,
    toast: Option<(String, f64)>,
    queue_drag: Option<usize>,

    mini: bool,
    restore_size: Option<egui::Vec2>,
    /// Taille à imposer à la fenêtre pendant quelques images (KWin peut d'abord rendre l'ancienne).
    pending_size: Option<(egui::Vec2, u8)>,
    window_title: String,
    seeked: bool,
    /// Égaliseur et nivellement, partagés avec le son.
    audio: crate::dsp::Shared,
    /// Le prochain morceau démarre en fondu enchaîné.
    crossfading: bool,
    /// Copie du son et spectre, pour le visualiseur.
    tap: crate::dsp::SharedTap,
    spectrum: crate::viz::Spectrum,
    aurora: crate::viz::Aurora,
    /// Visualiseurs automatiques, et nom affiché quand le visualiseur change.
    auto_np: crate::viz::AutoViz,
    auto_banner: crate::viz::AutoViz,
    viz_label: Option<(usize, f64)>,
    /// Webradios : station en cours, annuaire.
    radio: radio_view::RadioState,
    /// Genres, années, compositeurs (reconstruit avec la bibliothèque).
    browse_index: Option<browse::BrowseIndex>,
    /// Minuterie de sommeil, et facteur de volume du fondu qui la termine.
    sleep: Option<Sleep>,
    sleep_gain: f32,
    notifier: crate::notify::Notifier,
    /// Notification à envoyer quand la pochette du morceau sera prête (ou au plus tard à l'heure dite).
    pending_note: Option<(PathBuf, f64)>,
    /// La fenêtre est au premier plan (pas de notification alors).
    focused: bool,
    scrobbler: crate::scrobble::Scrobbler,
    /// Paramètres : sorties audio connues, connexion aux services de scrobbling.
    outputs: Vec<crate::outputs::Output>,
    online: settings_extra::OnlineState,
    egui_ctx: egui::Context,
}

/// Morceaux d'une playlist intelligente déjà calculés : empreinte des règles, heure du calcul.
type SmartCached = (u64, f64, Vec<usize>);

/// Résultat d'une mise à jour de la bibliothèque après des changements sur le disque.
pub enum WatchDone {
    Rescan,
    Update {
        tracks: Vec<library::Track>,
        gone: Vec<PathBuf>,
    },
}

/// Ce qui est demandé au lancement, en ligne de commande.
#[derive(Default)]
pub struct Launch {
    /// Dossier à ajouter à la bibliothèque.
    pub music_dir: Option<PathBuf>,
    /// Morceaux à jouer (double-clic dans le gestionnaire de fichiers).
    pub play_files: Vec<PathBuf>,
    /// Fichiers à ouvrir dans l'éditeur de tags.
    pub tag_files: Vec<PathBuf>,
}

/// Taille du mini lecteur.
const MINI_SIZE: egui::Vec2 = egui::vec2(380.0, 116.0);

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Ce qu'un relevé ajoute au temps écouté : l'avancée de la lecture, sauf un saut. La lecture
/// avance au rythme de l'horloge (même si la fenêtre, cachée, n'a pas été redessinée depuis
/// longtemps) : un bond plus grand que le temps écoulé, ou en arrière, est un saut.
fn heard_step(step: f32, wall: f32) -> f32 {
    if step >= 0.0 && step <= wall + 0.75 {
        step
    } else {
        0.0
    }
}

/// Ce qu'on envoie aux services de scrobbling d'un morceau (rien sans artiste ni titre).
fn scrobble_song(t: &library::Track) -> Option<crate::scrobble::Song> {
    if t.artist.trim().is_empty() || t.title.trim().is_empty() {
        return None;
    }
    Some(crate::scrobble::Song {
        artist: t.artist.clone(),
        title: t.title.clone(),
        album: t.album.clone(),
        album_artist: t.album_artist.clone(),
        duration: t.duration().as_secs() as u32,
        track_no: t.track_no,
    })
}

/// Courbe de volume plus naturelle pour l'oreille qu'une échelle linéaire.
fn perceptual(v: f32) -> f32 {
    v * v
}

impl SterMusic {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        let Launch {
            music_dir: cli_dir,
            play_files,
            tag_files,
        } = launch;
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
        // Apparence v2 : style Breeze et couleurs de KDE par défaut sous Plasma (Kubuntu).
        if settings.style_version < 2 {
            settings.style_version = 2;
            settings.backdrop = if settings.mica {
                BackdropKind::Mica
            } else {
                BackdropKind::Solid
            };
            settings.breeze = theme::is_kde_desktop() || theme::kde_theme().is_some();
            if settings.breeze && theme::kde_theme().is_some() {
                settings.theme = theme::KDE_ID.into();
            }
        }
        crate::i18n::set(settings.lang);
        theme::set_saved(&settings.saved_themes);
        // Visualiseurs v3 : dix de plus. « Automatique » venait juste après les dix-huit premiers.
        if settings.style_version < 3 {
            settings.style_version = 3;
            for style in [&mut settings.viz_style, &mut settings.banner_style] {
                if *style >= 18 {
                    *style = crate::shader::AUTO;
                }
            }
        }
        theme::install(&cc.egui_ctx, &settings.theme, settings.breeze);

        let volume = if settings.muted {
            0.0
        } else {
            perceptual(settings.volume)
        };
        let audio = crate::dsp::AudioSettings::new();
        audio.set(
            settings.eq_on,
            &settings.eq_gains,
            settings.eq_preamp,
            settings.leveling,
        );
        let tap = crate::dsp::Tap::new();
        crate::outputs::route_to(&settings.audio_output);
        let (player, toast) = match Player::new(volume, audio.clone(), tap.clone()) {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some((e, 0.0))),
        };

        let page = settings.page;
        let mut app = Self {
            player,
            lib: Library::new(library::load_cache()),
            covers: Covers::new(&cc.egui_ctx),
            mpris: Mpris::new(&cc.egui_ctx),
            kwin: crate::kwin::KWin::new(&cc.egui_ctx),
            backdrop: Backdrop::default(),
            queue: Vec::new(),
            queue_pos: None,
            unshuffled: None,
            view: View::Page(page),
            history: Vec::new(),
            search: String::new(),
            detail_filter: String::new(),
            queue_filter: String::new(),
            lib_filter: String::new(),
            search_results: Default::default(),
            focus_search: false,
            songs_sorted: None,
            scan_rx: None,
            scan_count: 0,
            restored: false,
            actions: Vec::new(),
            selected: Selection::default(),
            dialog: None,
            tag_editor: tag_editor::TagEditor::new(settings.tag_prefs.clone().sanitized()),
            ipc: crate::ipc::Ipc::new(&cc.egui_ctx),
            dnd: crate::dnd::Dnd::new(cc),
            external: Vec::new(),
            lib_tools: Default::default(),
            listens: crate::history::load(),
            listening: None,
            smart_edit: None,
            smart_cache: Default::default(),
            folder_index: None,
            lib_folder: None,
            lyrics: Default::default(),
            watcher: Default::default(),
            watch_rx: None,
            now_loaded: Default::default(),
            toast,
            queue_drag: None,
            mini: false,
            restore_size: None,
            pending_size: None,
            window_title: String::new(),
            seeked: false,
            audio,
            crossfading: false,
            tap,
            spectrum: crate::viz::Spectrum::default(),
            aurora: crate::viz::Aurora::default(),
            auto_np: crate::viz::AutoViz::default(),
            auto_banner: crate::viz::AutoViz::default(),
            viz_label: None,
            radio: Default::default(),
            browse_index: None,
            sleep: None,
            sleep_gain: 1.0,
            notifier: Default::default(),
            pending_note: None,
            focused: true,
            scrobbler: crate::scrobble::Scrobbler::new(settings.scrobble.clone()),
            outputs: Vec::new(),
            online: Default::default(),
            egui_ctx: cc.egui_ctx.clone(),
            settings,
        };
        if !app.settings.smart_seeded {
            app.settings.smart_seeded = true;
            app.settings
                .smart_playlists
                .extend(smart::SmartPlaylist::presets());
        }
        for p in &mut app.settings.smart_playlists {
            p.guess_icon();
        }
        if app.settings.auto_update {
            app.watcher = crate::watch::Watcher::start(&app.settings.music_dirs, &cc.egui_ctx);
        }
        app.restore_queue();
        app.start_scan();
        if !tag_files.is_empty() {
            app.open_tag_editor(&cc.egui_ctx, tag_files);
        }
        if !play_files.is_empty() {
            app.play_files(play_files);
        }
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
        self.set_library(tracks);
        self.covers.clear();
        if !self.restored {
            self.restore_queue();
        }
        self.toast(trf!(
            "Bibliothèque à jour : {0}",
            library::plural(count, "morceau", "morceaux")
        ));
    }

    /// Remplace la bibliothèque en gardant la file d'attente, le morceau en cours et la page
    /// ouverte (un album dont on vient de changer le titre reste affiché sous son nouveau nom).
    fn set_library(&mut self, mut tracks: Vec<library::Track>) {
        self.folder_index = None;
        self.browse_index = None;
        self.smart_cache.borrow_mut().clear();
        // Les morceaux ouverts hors bibliothèque restent jouables après un rescan.
        if !self.external.is_empty() {
            let present: HashSet<PathBuf> = tracks.iter().map(|t| t.path.clone()).collect();
            tracks.extend(
                self.external
                    .iter()
                    .filter(|t| !present.contains(&t.path))
                    .cloned(),
            );
        }
        let paths = |lib: &Library, v: &[usize]| -> Vec<PathBuf> {
            v.iter().map(|&i| lib.tracks[i].path.clone()).collect()
        };
        let queue_paths = paths(&self.lib, &self.queue);
        let unshuffled = self.unshuffled.as_ref().map(|u| paths(&self.lib, u));
        let current_path = self.current().map(|i| self.lib.tracks[i].path.clone());
        // Album ou artiste : repéré par l'un de ses morceaux.
        let anchor = |lib: &Library, v: &View| -> Option<(bool, PathBuf)> {
            match *v {
                View::Album(a) => lib
                    .albums
                    .get(a)
                    .map(|al| (true, lib.tracks[al.tracks[0]].path.clone())),
                View::Artist(a) => lib
                    .artists
                    .get(a)
                    .and_then(|ar| ar.tracks.first())
                    .map(|&t| (false, lib.tracks[t].path.clone())),
                _ => None,
            }
        };
        let view_anchor = anchor(&self.lib, &self.view);
        let history: Vec<(View, Option<(bool, PathBuf)>)> = self
            .history
            .iter()
            .map(|v| (v.clone(), anchor(&self.lib, v)))
            .collect();

        self.lib = Library::new(tracks);
        let lib = &self.lib;
        let remap = |v: View, anchor: Option<(bool, PathBuf)>| -> Option<View> {
            match anchor {
                None => Some(v),
                Some((album, path)) => lib.find(&path).map(|t| {
                    let a = lib.track_album[t];
                    if album {
                        View::Album(a)
                    } else {
                        View::Artist(lib.album_artist[a])
                    }
                }),
            }
        };
        self.history = history.into_iter().filter_map(|(v, a)| remap(v, a)).collect();
        match remap(self.view.clone(), view_anchor) {
            Some(v) => self.view = v,
            None => {
                self.view = View::Page(Page::Library);
                self.history.clear();
            }
        }
        let find = |v: &[PathBuf]| -> Vec<usize> { v.iter().filter_map(|p| self.lib.find(p)).collect() };
        self.queue = find(&queue_paths);
        self.unshuffled = unshuffled.map(|u| find(&u));
        self.queue_pos = current_path
            .and_then(|p| self.lib.find(&p))
            .and_then(|c| self.queue.iter().position(|&i| i == c));
        if self.queue_pos.is_none() {
            if let Some(p) = &mut self.player {
                p.stop();
            }
        }
        self.songs_sorted = None;
        self.selected.clear();
        self.refresh_search();
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
                SongSort::Album => {
                    v.sort_by_cached_key(|&i| (lib.tracks[i].album.to_lowercase(), lib.tracks[i].track_no))
                }
                SongSort::Added => v.sort_by_key(|&i| std::cmp::Reverse(lib.tracks[i].mtime)),
            }
            self.songs_sorted = Some((sort, v));
        }
        &self.songs_sorted.as_ref().unwrap().1
    }

    // ---------- Lecture ----------

    /// Joue des fichiers donnés par leur chemin ; ceux qui ne sont pas dans la bibliothèque y
    /// sont ajoutés pour la session.
    fn play_files(&mut self, paths: Vec<PathBuf>) {
        let paths = crate::tags::expand_until(paths, &std::sync::atomic::AtomicBool::new(false));
        let new: Vec<library::Track> = paths
            .iter()
            .filter(|p| self.lib.find(p).is_none())
            .map(|p| library::read_track(p))
            .collect();
        if !new.is_empty() {
            self.external.extend(new);
            let tracks = self.lib.tracks.clone();
            self.set_library(tracks);
        }
        let tracks: Vec<usize> = paths.iter().filter_map(|p| self.lib.find(p)).collect();
        if tracks.is_empty() {
            self.toast(tr("Aucun fichier audio à lire"));
            return;
        }
        self.play_list(tracks, 0);
    }

    /// Morceau en cours (aucun pendant l'écoute d'une webradio).
    pub fn current(&self) -> Option<usize> {
        if self.radio.live.is_some() {
            return None;
        }
        self.queue_pos.and_then(|p| self.queue.get(p).copied())
    }

    fn is_playing(&self) -> bool {
        self.player.as_ref().is_some_and(Player::is_playing)
    }

    fn play_current(&mut self) {
        self.leave_radio();
        let Some(idx) = self.current() else { return };
        let Some(player) = &mut self.player else {
            return;
        };
        let track = &self.lib.tracks[idx];
        let result = if self.crossfading {
            let duration = Duration::from_secs_f32(self.settings.crossfade_secs.clamp(1.0, 12.0));
            player.crossfade_to(&track.path, duration)
        } else {
            player.load(&track.path, false)
        };
        match result {
            // L'écoute n'est comptée qu'au bout de 30 secondes (voir `track_listen`).
            Ok(()) => {
                let path = track.path.clone();
                let song = scrobble_song(track);
                self.finish_listen();
                if let Some(song) = song {
                    if self.settings.scrobble.any() {
                        self.scrobbler.now_playing(song);
                    }
                }
                let now = self.egui_ctx.input(|i| i.time);
                self.pending_note = Some((path.clone(), now + 1.5));
                self.listening = Some(Listening {
                    scrobbled: false,
                    path,
                    started: now_secs(),
                    heard: 0.0,
                    last_pos: 0.0,
                    last_tick: std::time::Instant::now(),
                    counted: false,
                });
            }
            Err(e) => {
                let msg = format!("{} — {e}", track.title);
                self.toast(msg);
            }
        }
        self.seeked = true;
    }

    /// Fondu enchaîné : quelques secondes avant la fin du morceau, le suivant démarre en fondu.
    fn maybe_crossfade(&mut self) {
        // La minuterie « fin du morceau » arrête la lecture : pas de morceau suivant.
        if !self.settings.crossfade || self.sleep.as_ref().is_some_and(|s| s.kind == SleepKind::EndOfTrack) {
            return;
        }
        let Some(p) = &self.player else { return };
        if !p.is_playing() || p.is_fading() {
            return;
        }
        let Some(cur) = self.current() else { return };
        let total = self.lib.tracks[cur].duration().as_secs_f32();
        let fade = self.settings.crossfade_secs.clamp(1.0, 12.0);
        // Morceaux très courts : pas de fondu (il en mangerait une trop grande part).
        if total < fade * 3.0 || total - p.position().as_secs_f32() > fade {
            return;
        }
        let has_next =
            self.settings.repeat != Repeat::Off || self.queue_pos.is_some_and(|q| q + 1 < self.queue.len());
        if !has_next {
            return;
        }
        self.crossfading = true;
        self.next(true);
        self.crossfading = false;
    }

    /// Visualiseur : analyse le son quand une page qui l'affiche est à l'écran, et redessine en
    /// continu pendant la lecture (ou tant que l'animation retombe).
    fn update_visualizer(&mut self, ctx: &egui::Context) {
        let visible = self.settings.visualizer
            && !self.mini
            && matches!(self.view, View::NowPlaying | View::Page(Page::Home));
        if !visible {
            return;
        }
        let playing = self.is_playing();
        let dt = ctx.input(|i| i.stable_dt);
        self.spectrum.update(&self.tap, dt, playing);
        if playing || self.spectrum.alive() {
            ctx.request_repaint();
        }
    }

    /// Visualiseurs du tirage automatique (ceux que l'utilisateur n'a pas exclus).
    pub(super) fn auto_allowed(&self) -> Vec<usize> {
        let all = crate::shader::STYLES.iter().enumerate();
        let allowed: Vec<usize> = all
            .filter(|(_, (n, _))| !self.settings.auto_excluded.iter().any(|e| e == n))
            .map(|(i, _)| i)
            .collect();
        if allowed.is_empty() {
            (0..crate::shader::STYLES.len()).collect()
        } else {
            allowed
        }
    }

    /// Couleurs du visualiseur : celles de la pochette en cours, sinon celles du thème.
    pub(super) fn viz_palette(&mut self) -> [egui::Color32; 3] {
        let from_cover = self.current().and_then(|i| {
            let path = self.lib.tracks[i].path.clone();
            self.covers.large(&path).and_then(|l| l.palette)
        });
        from_cover.unwrap_or([theme::accent(), theme::accent_2(), theme::accent_light()])
    }

    /// Transmet les réglages de l'égaliseur et du nivellement au son (effet immédiat).
    fn push_audio(&self) {
        let s = &self.settings;
        self.audio.set(s.eq_on, &s.eq_gains, s.eq_preamp, s.leveling);
    }

    fn play_list(&mut self, tracks: Vec<usize>, start: usize) {
        if tracks.is_empty() {
            return;
        }
        // Une lecture choisie remplace la file du dernier lancement : sans cela, la fin du premier
        // scan (bibliothèque encore vide au démarrage) restaurait l'ancienne file par-dessus.
        self.restored = true;
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
        if let Some(live) = &self.radio.live {
            if live.paused {
                let station = live.station.clone();
                self.radio.connecting = None;
                let ctx = self.egui_ctx.clone();
                self.play_station(&ctx, station);
            } else {
                self.pause_station();
            }
            return;
        }
        let has_track = self.player.as_ref().is_some_and(Player::has_track);
        if has_track {
            let mut error = None;
            if let Some(p) = &mut self.player {
                if p.is_paused() {
                    error = p.play().err();
                } else {
                    p.pause();
                }
            }
            if let Some(e) = error {
                self.toast(e);
            }
            self.seeked = true;
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
        if let Some(p) = &mut self.player {
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
        let max = self
            .current()
            .map(|i| self.lib.tracks[i].duration().as_secs_f32())
            .unwrap_or(f32::MAX);
        let Some(p) = &mut self.player else { return };
        if !p.has_track() {
            return;
        }
        if let Err(e) = p.seek(Duration::from_secs_f32(secs.clamp(0.0, max.max(0.0)))) {
            self.toast(e);
        }
        self.seeked = true;
    }

    fn apply_volume(&mut self) {
        let v = if self.settings.muted {
            0.0
        } else {
            perceptual(self.settings.volume) * self.sleep_gain
        };
        if let Some(p) = &mut self.player {
            p.set_volume(v);
        }
    }

    /// Tient `now_loaded` à jour (voir ce champ).
    fn sync_now_loaded(&mut self) {
        let loaded = self
            .current()
            .filter(|_| self.player.as_ref().is_some_and(Player::has_track))
            .map(|i| &self.lib.tracks[i].path);
        let mut shared = self.now_loaded.lock().unwrap();
        if shared.as_ref() != loaded {
            *shared = loaded.cloned();
        }
    }

    /// Morceaux d'une playlist intelligente ; le calcul (toute la bibliothèque) est gardé deux
    /// secondes, ou jusqu'à ce que ses règles changent.
    fn smart_tracks(&self, i: usize) -> Vec<usize> {
        let Some(p) = self.settings.smart_playlists.get(i) else {
            return Vec::new();
        };
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(p).unwrap_or_default().hash(&mut h);
        (self.lib.tracks.len(), self.settings.favorites.len()).hash(&mut h);
        let key = h.finish();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        if let Some((k, at, v)) = self.smart_cache.borrow().get(&i) {
            if *k == key && now - at < 2.0 {
                return v.clone();
            }
        }
        let v = smart::evaluate(p, &self.lib, &self.smart_facts());
        self.smart_cache.borrow_mut().insert(i, (key, now, v.clone()));
        v
    }

    /// Suit l'écoute du morceau en cours : seules les secondes vraiment entendues comptent (un
    /// saut dans le morceau n'en ajoute aucune). Passé 30 secondes (ou la moitié d'un morceau
    /// court), l'écoute compte ; elle est notée dans l'historique quand le morceau s'arrête, avec
    /// sa durée réelle.
    fn track_listen(&mut self) {
        let Some(path) = self.listening.as_ref().map(|l| l.path.clone()) else {
            return;
        };
        let current = self.current().map(|i| self.lib.tracks[i].clone());
        let same = current.as_ref().is_some_and(|t| t.path == path);
        let playing = self
            .player
            .as_ref()
            .is_some_and(|p| p.has_track() && p.is_playing());
        if !same || !self.player.as_ref().is_some_and(Player::has_track) {
            self.finish_listen();
            return;
        }
        let Some(p) = &self.player else { return };
        let pos = p.position().as_secs_f32();
        let Some(l) = &mut self.listening else { return };
        let wall = l.last_tick.elapsed().as_secs_f32();
        if playing {
            let step = pos - l.last_pos;
            l.heard += heard_step(step, wall);
        }
        l.last_pos = pos;
        l.last_tick = std::time::Instant::now();
        let needed = current
            .as_ref()
            .map(|t| (t.duration().as_secs_f32() * 0.5).clamp(1.0, 30.0))
            .unwrap_or(30.0);
        if !l.counted && l.heard >= needed {
            l.counted = true;
            let stat = self.settings.plays.entry(l.path.clone()).or_default();
            stat.count += 1;
            stat.last = now_secs();
        }
        let duration = current.as_ref().map(|t| t.duration().as_secs_f32()).unwrap_or(0.0);
        if !l.scrobbled && crate::scrobble::counts(duration, l.heard) {
            l.scrobbled = true;
            let started = l.started;
            if self.settings.scrobble.any() {
                if let Some(song) = current.as_ref().and_then(scrobble_song) {
                    self.scrobbler.scrobble(song, started);
                }
            }
        }
    }

    /// L'écoute en cours, si elle compte déjà : le morceau et les secondes entendues.
    pub(super) fn listening_now(&self) -> Option<(PathBuf, f32)> {
        self.listening
            .as_ref()
            .filter(|l| l.counted)
            .map(|l| (l.path.clone(), l.heard))
    }

    /// Fin de l'écoute en cours (autre morceau, arrêt, fermeture) : notée si elle a compté.
    fn finish_listen(&mut self) {
        let Some(l) = self.listening.take() else { return };
        if !l.counted {
            return;
        }
        if let Some(stat) = self.settings.plays.get_mut(&l.path) {
            stat.seconds += l.heard as f64;
        }
        let listen = crate::history::Listen {
            time: l.started,
            path: l.path,
            seconds: l.heard.round() as u32,
        };
        crate::history::append(&listen);
        self.listens.push(listen);
    }

    /// (Re)lance la surveillance des dossiers de musique, ou l'arrête.
    fn restart_watch(&mut self, ctx: &egui::Context) {
        self.watcher = if self.settings.auto_update {
            crate::watch::Watcher::start(&self.settings.music_dirs, ctx)
        } else {
            Default::default()
        };
    }

    /// Changements sur le disque : relit les fichiers touchés en arrière-plan, puis met la
    /// bibliothèque à jour.
    fn poll_watch(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.watch_rx {
            match rx.try_recv() {
                Ok(done) => {
                    self.watch_rx = None;
                    self.apply_watch(done);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.watch_rx = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.watcher.waiting() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        // Un scan complet ou une mise à jour déjà en route : les changements attendent leur tour.
        if self.scan_rx.is_some() || self.watch_rx.is_some() {
            return;
        }
        let Some(paths) = self.watcher.poll() else { return };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let done = match crate::watch::plan(paths) {
                crate::watch::Plan::Rescan => WatchDone::Rescan,
                crate::watch::Plan::Update { read, gone } => WatchDone::Update {
                    tracks: read.iter().map(|p| library::read_track(p)).collect(),
                    gone,
                },
            };
            let _ = tx.send(done);
            ctx.request_repaint();
        });
        self.watch_rx = Some(rx);
    }

    fn apply_watch(&mut self, done: WatchDone) {
        let (tracks, gone) = match done {
            WatchDone::Rescan => {
                self.start_scan();
                return;
            }
            WatchDone::Update { tracks, gone } => (tracks, gone),
        };
        let before = self.lib.tracks.len();
        let updated: HashSet<PathBuf> = tracks.iter().map(|t| t.path.clone()).collect();
        let mut all: Vec<library::Track> = self
            .lib
            .tracks
            .iter()
            // Un chemin disparu peut être un fichier ou un dossier entier.
            .filter(|t| !updated.contains(&t.path) && !gone.iter().any(|g| t.path.starts_with(g)))
            .cloned()
            .collect();
        let removed = before - all.len();
        // Seuls les fichiers rangés dans un dossier de la bibliothèque y entrent.
        let dirs = &self.settings.music_dirs;
        all.extend(
            tracks
                .into_iter()
                .filter(|t| dirs.iter().any(|d| t.path.starts_with(d))),
        );
        let added = all.len().saturating_sub(before - removed);
        let changed = added + removed > 0 || !updated.is_empty();
        if !changed {
            return;
        }
        let paths: Vec<PathBuf> = updated.iter().cloned().collect();
        library::save_cache(&all);
        self.set_library(all);
        self.covers.forget(&paths);
        let net_added = self.lib.tracks.len().saturating_sub(before);
        let net_removed = before.saturating_sub(self.lib.tracks.len());
        if net_added > 0 {
            self.toast(trf!(
                "Bibliothèque à jour : {0}",
                library::plural(net_added, "morceau ajouté", "morceaux ajoutés")
            ));
        } else if net_removed > 0 {
            self.toast(trf!(
                "Bibliothèque à jour : {0}",
                library::plural(net_removed, "morceau retiré", "morceaux retirés")
            ));
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
        self.selected.clear();
        self.detail_filter.clear();
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
        self.detail_filter.clear();
    }

    pub fn playlist_tracks(&self, id: PlaylistId) -> Vec<usize> {
        match id {
            PlaylistId::Smart(i) => self.smart_tracks(i),
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
            PlaylistId::Smart(i) => self
                .settings
                .smart_playlists
                .get(i)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            PlaylistId::Favorites => tr("Favoris").into(),
            PlaylistId::User(i) => self
                .settings
                .playlists
                .get(i)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
        }
    }

    fn paths(&self, tracks: &[usize]) -> Vec<PathBuf> {
        tracks.iter().map(|&i| self.lib.tracks[i].path.clone()).collect()
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
                        self.toast(library::plural(
                            n,
                            "morceau sera lu ensuite",
                            "morceaux seront lus ensuite",
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
                self.toast(library::plural(
                    n,
                    "morceau ajouté à la file d'attente",
                    "morceaux ajoutés à la file d'attente",
                ));
            }
            Action::Go(view) => {
                // Rejoue le fondu d'apparition de la page.
                lists::reset_page(ctx, &format!("{view:?}"));
                self.go(view);
            }
            Action::Back => {
                self.back();
                lists::reset_page(ctx, &format!("{:?}", self.view));
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
                    let msg = trf!("Ajouté à « {0} »", p.name);
                    self.toast(msg);
                }
            }
            Action::NewPlaylist(tracks) => {
                let name = trf!("Playlist {0}", self.settings.playlists.len() + 1);
                self.dialog = Some(Dialog::NewPlaylist { name, tracks });
            }
            Action::RemoveFromPlaylist(id, pos) => match id {
                // Une playlist intelligente se remplit d'après ses règles : rien à retirer à la main.
                PlaylistId::Smart(_) => {}
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
                        self.queue.iter().position(|&i| Some(i) == cur).unwrap_or(pos)
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
            Action::EditTags(paths) => self.open_tag_editor(ctx, paths),
            Action::PlayFiles(paths) => self.play_files(paths),
            Action::LibTool(tool) => self.open_lib_tool(tool, ctx),
            Action::ReplayGain(paths, label) => self.open_replaygain_for(paths, label),
            Action::PickTagFiles => {
                if let Some(files) = rfd::FileDialog::new()
                    .set_title(tr("Fichiers dont modifier les tags"))
                    .add_filter(tr("Musique"), crate::library::AUDIO_EXTENSIONS)
                    .pick_files()
                {
                    self.open_tag_editor(ctx, files);
                }
            }
            Action::AddFolder => {
                if let Some(dir) = rfd::FileDialog::new()
                    .set_title(tr("Ajouter un dossier de musique"))
                    .pick_folder()
                {
                    self.apply(ctx, Action::UseFolder(dir));
                }
            }
            Action::UseFolder(dir) => {
                if !self.settings.music_dirs.contains(&dir) {
                    self.settings.music_dirs.push(dir);
                    self.start_scan();
                    self.restart_watch(ctx);
                }
            }
            Action::SetAutoUpdate(on) => {
                self.settings.auto_update = on;
                self.restart_watch(ctx);
            }
            Action::NewSmartPlaylist => self.smart_edit = Some(smart::Edit::new(None, None)),
            Action::ExportStats(per_track) => self.export_stats(per_track),
            Action::ClearStats => self.dialog = Some(Dialog::ClearStats),
            Action::EditSmartPlaylist(i) => {
                if let Some(p) = self.settings.smart_playlists.get(i).cloned() {
                    self.smart_edit = Some(smart::Edit::new(Some(i), Some(p)));
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
                    self.restart_watch(ctx);
                }
            }
            Action::Rescan => self.start_scan(),
            Action::TogglePlay => self.toggle_play(),
            Action::Next if self.radio.live.is_some() => self.step_station(ctx, 1),
            Action::Previous if self.radio.live.is_some() => self.step_station(ctx, -1),
            Action::Next => self.next(false),
            Action::Previous => self.previous(),
            Action::PlayStation(station) => self.play_station(ctx, station),
            Action::ToggleStation(station) => self.toggle_saved_station(&station),
            Action::SetSleep(kind) => self.set_sleep(kind),
            Action::SetOutput(name) => self.set_output(name),
            Action::ScrobbleConfig => self.scrobbler.set_config(self.settings.scrobble.clone()),
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
            Action::ToggleSidebar => self.settings.sidebar_collapsed = !self.settings.sidebar_collapsed,
            Action::SetBreeze(on) => {
                self.settings.breeze = on;
                theme::set_breeze(ctx, on);
            }
            Action::EnableKwinBlur => self.enable_kwin_blur(),
            Action::SetTheme(id) => {
                self.settings.theme = id.to_owned();
                theme::set(ctx, id);
            }
            Action::SetLang(lang) => {
                if lang != crate::i18n::lang() {
                    self.settings.lang = lang;
                    crate::i18n::set(lang);
                    // « Artiste inconnu », « Album inconnu » : la bibliothèque prend les nouveaux noms.
                    let tracks = self.lib.tracks.clone();
                    self.set_library(tracks);
                }
            }
            Action::SaveTheme => {
                self.dialog = Some(Dialog::ThemeName {
                    id: None,
                    name: theme::suggested_name(),
                });
            }
            Action::RenameTheme(id) => {
                if let Some(t) = self.settings.saved_themes.iter().find(|t| t.id == id) {
                    self.dialog = Some(Dialog::ThemeName {
                        name: t.name.clone(),
                        id: Some(id),
                    });
                }
            }
            Action::DeleteTheme(id) => self.dialog = Some(Dialog::DeleteTheme(id)),
            Action::FocusSearch => {
                self.settings.sidebar_collapsed = false;
                self.focus_search = true;
            }
        }
    }

    fn toggle_mini(&mut self, ctx: &egui::Context) {
        self.mini = !self.mini;
        if self.mini {
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            if !maximized {
                self.restore_size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
            }
            ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(MINI_SIZE));
            ctx.send_viewport_cmd(ViewportCommand::MaxInnerSize(MINI_SIZE * egui::vec2(1.6, 1.25)));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(MINI_SIZE));
            // X11 seulement ; sous Wayland, c'est le script KWin qui s'en charge (titre marqué).
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            self.pending_size = Some((MINI_SIZE, 30));
        } else {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
            ctx.send_viewport_cmd(ViewportCommand::MaxInnerSize(egui::vec2(
                f32::INFINITY,
                f32::INFINITY,
            )));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(egui::vec2(760.0, 520.0)));
            let size = self.restore_size.unwrap_or(egui::vec2(1240.0, 800.0));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            self.pending_size = Some((size, 30));
        }
    }

    /// Redemande la taille voulue tant que la fenêtre ne l'a pas : en sortant de l'état agrandi,
    /// KWin peut d'abord lui rendre son ancienne taille.
    fn apply_pending_size(&mut self, ctx: &egui::Context) {
        let Some((size, frames)) = self.pending_size else {
            return;
        };
        let current = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
        let done = current.is_some_and(|c| (c - size).length() < 2.0);
        if done || frames == 0 {
            self.pending_size = None;
            return;
        }
        if frames % 5 == 0 {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
        self.pending_size = Some((size, frames - 1));
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    fn export_playlist(&mut self, id: PlaylistId) {
        let name = self.playlist_name(id);
        let Some(file) = rfd::FileDialog::new()
            .set_title(tr("Exporter la playlist"))
            .set_file_name(format!("{name}.m3u"))
            .add_filter(tr("Playlist M3U"), &["m3u", "m3u8"])
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
            Ok(()) => self.toast(trf!("Playlist exportée : {0}", file.display())),
            Err(e) => self.toast(trf!("Export impossible : {e}", e)),
        }
    }

    fn import_playlist(&mut self) {
        let Some(file) = rfd::FileDialog::new()
            .set_title(tr("Importer une playlist"))
            .add_filter(tr("Playlist M3U"), &["m3u", "m3u8"])
            .pick_file()
        else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            self.toast(tr("Lecture du fichier impossible"));
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
            .unwrap_or_else(|| tr("Playlist").into());
        self.settings.playlists.push(Playlist {
            name: name.clone(),
            tracks,
        });
        self.toast(trf!(
            "« {name} » importée ({found} morceaux trouvés dans la bibliothèque)",
            name,
            found
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
                    if self.radio.live.is_some() {
                        self.pause_station();
                    } else if let Some(p) = &mut self.player {
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
        if !dropped.is_empty() {
            self.handle_drop(ctx, dropped);
        }
        let cmd = egui::Modifiers::COMMAND;
        let none = egui::Modifiers::NONE;
        if ctx.input_mut(|i| i.consume_key(cmd, Key::F)) {
            self.actions.push(Action::FocusSearch);
        }
        if ctx.input_mut(|i| i.consume_key(cmd, Key::M)) {
            self.actions.push(Action::ToggleMini);
        }
        if ctx.wants_keyboard_input()
            || self.dialog.is_some()
            || self.tag_editor.modal_open()
            || self.lib_tools.open.is_some()
            || self.radio.add.is_some()
        {
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
        let radio_track = self.radio.live.as_ref().map(|l| library::Track {
            path: PathBuf::from(format!("{}#{}", l.station.url, l.last_title)),
            title: if l.last_title.is_empty() {
                l.station.name.clone()
            } else {
                l.last_title.clone()
            },
            artist: l.station.name.clone(),
            album: tr("Webradio").into(),
            album_artist: String::new(),
            genre: String::new(),
            year: None,
            track_no: None,
            disc_no: None,
            duration_ms: 0,
            mtime: 0,
            composer: String::new(),
            schema: 0,
        });
        match &radio_track {
            Some(t) => self.mpris.set_track(Some(t), None),
            None => self
                .mpris
                .set_track(current.map(|i| &self.lib.tracks[i]), cover_file.as_deref()),
        }
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
        self.mpris.set_state(state, pos, std::mem::take(&mut self.seeked));

        let title = match (current.map(|i| &self.lib.tracks[i]), &radio_track) {
            (_, Some(t)) => format!("{} — {} · SterMusic", t.title, t.artist),
            (Some(t), None) => format!("{} — {} · SterMusic", t.title, t.artist_or_unknown()),
            (None, None) => "SterMusic".to_owned(),
        };
        // Le script KWin garde au premier plan la fenêtre dont le titre porte cette marque.
        let title = if self.mini {
            format!("{title}{}", crate::kwin::KEEP_ABOVE_MARK)
        } else {
            title
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
            has_track: self.player.as_ref().is_some_and(Player::has_track),
        }
    }
}

impl eframe::App for SterMusic {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_scan();
        self.poll_tag_editor(ctx);
        self.poll_lib_tools(ctx);
        self.sync_now_loaded();
        self.track_listen();
        self.poll_watch(ctx);
        self.poll_lyrics(ctx);
        self.poll_radio(ctx);
        self.poll_online(ctx);
        self.focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        self.send_pending_note(ctx);
        self.sleep_tick(ctx);
        for request in self.ipc.poll() {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            match request {
                crate::ipc::Request::EditTags(files) => self.actions.push(Action::EditTags(files)),
                crate::ipc::Request::Play(files) => self.actions.push(Action::PlayFiles(files)),
                crate::ipc::Request::Show => {}
            }
        }
        self.covers.poll(ctx);
        self.kwin.poll();
        self.apply_pending_size(ctx);
        self.handle_remote(ctx);
        // Fin d'un morceau (une webradio coupée est relancée par `poll_radio`).
        if self.radio.live.is_none() && self.player.as_ref().is_some_and(Player::finished) {
            if self.sleep.as_ref().is_some_and(|s| s.kind == SleepKind::EndOfTrack) {
                self.sleep_done();
            } else {
                self.next(true);
            }
        }
        self.maybe_crossfade();
        if let Some(p) = &mut self.player {
            p.tick();
            if p.is_fading() {
                ctx.request_repaint_after(Duration::from_millis(30));
            }
            if p.holds_idle_output() {
                ctx.request_repaint_after(crate::player::RELEASE_AFTER + Duration::from_millis(50));
            }
        }
        self.handle_keys(ctx);
        self.update_visualizer(ctx);

        // Coins arrondis, sauf fenêtre agrandie ou plein écran.
        let full = ctx
            .input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false));
        theme::set_window_radius(if self.settings.rounded && !full { 12 } else { 0 });

        let strength = self.settings.mica_strength;
        let mode = match self.settings.backdrop {
            BackdropKind::Solid => backdrop::Mode::Solid,
            BackdropKind::Mica => backdrop::Mode::Mica(strength),
            BackdropKind::Transparent => backdrop::Mode::Transparent(strength),
        };
        self.backdrop.paint(ctx, mode, self.kwin.geometry.as_ref());
        if self.view == View::NowPlaying || self.mini {
            self.paint_cover_backdrop(ctx);
        }
        self.paint_window_border(ctx);

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
            self.radio_dialog(ctx);
            self.lib_tool_dialogs(ctx);
            self.smart_dialog(ctx);
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

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.dnd.feed(raw_input);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.finish_listen();
        self.leave_radio();
        self.kwin.stop();
        self.lib_tools.stop();
    }

    /// Fenêtre transparente : seul le fond arrondi peint par SterMusic est opaque.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.queue = self.paths(&self.queue);
        self.settings.queue_pos = self.queue_pos;
        self.settings.tag_prefs = self.tag_editor.prefs.clone();
        // Pendant une webradio, le lecteur ne tient plus la position du morceau de la file.
        self.settings.position = self
            .player
            .as_ref()
            .filter(|_| self.radio.live.is_none())
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
            let radius = egui::CornerRadius::same(theme::window_radius());
            backdrop::paint_fill(
                &painter,
                &tex,
                screen,
                egui::Color32::from_gray(150).gamma_multiply(t),
                radius,
            );
            painter.rect_filled(screen, radius, theme::alpha(theme::bg(), (0x70 as f32 * t) as u8));
        }
    }

    /// Liseré de la fenêtre (elle n'a pas de bordure du système) : fin trait clair, arrondi.
    fn paint_window_border(&self, ctx: &egui::Context) {
        let r = theme::window_radius();
        if r == 0 {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("window_border"),
        ));
        painter.rect_stroke(
            ctx.screen_rect(),
            egui::CornerRadius::same(r),
            egui::Stroke::new(1.0_f32, theme::white(0x1E)),
            egui::StrokeKind::Inside,
        );
    }

    /// Ajoute « stermusic » aux classes de fenêtres de l'effet Better Blur (forceblur) de KWin,
    /// puis demande à KWin de relire sa configuration.
    fn enable_kwin_blur(&mut self) {
        let read = std::process::Command::new("kreadconfig6")
            .args([
                "--file",
                "kwinrc",
                "--group",
                "Effect-blurplus",
                "--key",
                "WindowClasses",
            ])
            .output();
        let Ok(out) = read else {
            self.toast(tr(
                "kreadconfig6 introuvable : l'effet Better Blur n'est pas réglable automatiquement",
            ));
            return;
        };
        let current = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        // kreadconfig6 rend la liste avec de vrais retours à la ligne (kwriteconfig6 les échappe).
        let classes: Vec<&str> = current.lines().map(str::trim).filter(|c| !c.is_empty()).collect();
        if !classes.contains(&"stermusic") {
            let value = classes
                .iter()
                .copied()
                .chain(["stermusic"])
                .collect::<Vec<_>>()
                .join("\n");
            let ok = std::process::Command::new("kwriteconfig6")
                .args([
                    "--file",
                    "kwinrc",
                    "--group",
                    "Effect-blurplus",
                    "--key",
                    "WindowClasses",
                    &value,
                ])
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                self.toast(tr("Impossible de modifier la configuration de KWin"));
                return;
            }
        }
        let _ = std::process::Command::new("qdbus6")
            .args(["org.kde.KWin", "/KWin", "reconfigure"])
            .status();
        self.settings.backdrop = BackdropKind::Transparent;
        self.toast(tr(
            "SterMusic ajouté à Better Blur : KWin floute maintenant l'arrière-plan",
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::heard_step;

    #[test]
    fn listening_time() {
        // Lecture normale, même après une longue pause d'affichage (fenêtre cachée).
        assert_eq!(heard_step(0.25, 0.25), 0.25);
        assert_eq!(heard_step(42.0, 42.1), 42.0);
        // Saut de 2:00 à 4:00 : rien n'est compté ; retour en arrière non plus.
        assert_eq!(heard_step(120.0, 0.3), 0.0);
        assert_eq!(heard_step(-30.0, 0.3), 0.0);
        // Écouter 2 minutes, sauter 2 minutes, écouter encore 1 minute : 3 minutes.
        let mut heard = 0.0;
        for _ in 0..480 {
            heard += heard_step(0.25, 0.25);
        }
        heard += heard_step(120.0, 0.25);
        for _ in 0..240 {
            heard += heard_step(0.25, 0.25);
        }
        assert_eq!(heard, 180.0);
    }
}
