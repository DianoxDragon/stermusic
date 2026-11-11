//! SterMusic : état de l'application, file d'attente, actions.

mod ambience;
mod browse;
mod cast_view;
mod chrome;
mod cover_tool;
mod dup_tool;
mod folders;
mod lib_tools;
mod lists;
mod lyrics_view;
mod mb_import;
mod nowbar;
mod pace;
mod podcasts_view;
mod radio_globe;
mod radio_view;
mod ratings;
mod servers_view;
mod rg_tool;
mod settings_extra;
mod smart;
mod stats;
mod stats_extra;
mod tag_editor;
mod tag_tools;
mod taste;
mod tools;
mod views;
mod viz_view;

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
    Podcasts,
    /// Outils (éditeur de tags, entretien, conversion, CD, synchronisation).
    Tools,
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
    /// Un podcast (par l'adresse de son flux).
    Podcast(String),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    pub tracks: Vec<PathBuf>,
    /// Épinglée dans la barre latérale.
    #[serde(default)]
    pub pinned: bool,
}

/// Nombre maximal de playlists épinglées dans la barre latérale.
pub const MAX_PINS: usize = 6;

#[derive(Clone, Copy, Serialize, Deserialize, Default)]
pub struct PlayStat {
    pub count: u32,
    pub last: u64,
    /// Secondes réellement écoutées (depuis que SterMusic les mesure).
    #[serde(default)]
    pub seconds: f64,
    /// Fois où le morceau a été passé avant d'avoir compté comme une écoute.
    #[serde(default)]
    pub skips: u32,
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
    /// Fondu aussi entre deux morceaux d'un même album (d'ordinaire, ils s'enchaînent sans blanc).
    pub crossfade_albums: bool,
    /// Sortie bit-perfect : le son du fichier part tel quel, à sa fréquence.
    pub bit_perfect: bool,
    /// Icône dans la zone de notification ; fermer la fenêtre y garde alors SterMusic.
    pub tray: bool,
    /// Thème adaptatif : les couleurs de l'interface suivent la pochette du morceau en cours.
    pub adaptive_theme: bool,
    /// Nivellement du volume (ReplayGain, sinon automatique).
    pub leveling: bool,
    /// Visualiseur (animations qui suivent la musique).
    pub visualizer: bool,
    /// Visualiseur de « En cours de lecture » et du bandeau de l'accueil (voir `shader::STYLES`).
    pub viz_style: usize,
    pub banner_style: usize,
    /// Visualiseurs exclus du mode automatique (par leur nom).
    pub auto_excluded: Vec<String>,
    /// Mode automatique : un autre visualiseur toutes les `viz_auto_secs` secondes (0 : seulement
    /// au changement de morceau).
    pub viz_auto_secs: f32,
    /// Luminosité (0,4 → 1,6) et vitesse (0,4 → 2) des visualiseurs.
    pub viz_intensity: f32,
    pub viz_speed: f32,
    /// Poussières de lumière par-dessus le visualiseur.
    pub viz_particles: bool,
    /// Réglages propres à chaque visualiseur (par son nom) ; absents : ceux d'origine.
    pub viz_tunes: HashMap<String, crate::viz::Tune>,
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
    /// Version des playlists d'office déjà proposées (2 : celles sur les sauts et l'engagement).
    #[serde(default)]
    pub smart_presets: u32,
    /// Playlists épinglées dans la barre latérale : les favoris (les autres portent leur propre
    /// marque), et vrai une fois les premières épinglées d'office.
    pub favorites_pinned: bool,
    pub pins_seeded: bool,
    /// Page Playlists : tri, sens du tri, affichage en liste (sinon en grille).
    pub pl_sort: views::PlSort,
    pub pl_sort_desc: bool,
    pub pl_list: bool,
    /// Bibliothèque, onglet Morceaux : tri par colonnes (clic sur leurs titres).
    pub song_columns: lists::SortSpec,
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
    /// Page Webradios : vue Globe plutôt que l'annuaire.
    pub radio_globe: bool,
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
    /// Notes des morceaux, de 1 à 5 étoiles.
    pub ratings: HashMap<PathBuf, u8>,
    /// Serveurs de musique (Navidrome, Jellyfin…).
    pub servers: Vec<crate::servers::Server>,
    /// Podcasts suivis, et où en est l'écoute de chaque épisode.
    pub podcasts: Vec<crate::podcasts::Podcast>,
    pub episode_progress: HashMap<String, crate::podcasts::Progress>,
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
            crossfade_albums: false,
            bit_perfect: false,
            tray: false,
            adaptive_theme: false,
            leveling: false,
            visualizer: true,
            viz_style: 0,
            banner_style: 1,
            auto_excluded: Vec::new(),
            viz_auto_secs: 45.0,
            viz_intensity: 1.0,
            viz_speed: 1.0,
            viz_particles: true,
            viz_tunes: HashMap::new(),
            tag_id3v23: false,
            tag_keep_mtime: true,
            tag_prefs: Default::default(),
            saved_themes: Vec::new(),
            lang: crate::i18n::Lang::from_env(),
            smart_playlists: Vec::new(),
            smart_seeded: false,
            smart_presets: 0,
            favorites_pinned: false,
            pins_seeded: false,
            pl_sort: Default::default(),
            pl_sort_desc: false,
            pl_list: false,
            song_columns: Default::default(),
            stats_period: stats::Period::default(),
            lyrics_online: true,
            show_lyrics: true,
            auto_update: true,
            sidebar_collapsed: false,
            browse_kind: Default::default(),
            radios: Vec::new(),
            radio_globe: false,
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
            ratings: HashMap::new(),
            servers: Vec::new(),
            podcasts: Vec::new(),
            episode_progress: HashMap::new(),
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
    /// Une file rangée selon mes goûts, du morceau préféré au moins aimé (sans aléatoire).
    PlayByTaste(Vec<usize>),
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
    /// Épingle une playlist dans la barre latérale, ou l'en retire.
    TogglePin(PlaylistId),
    /// Vrai plein écran, sur « En cours de lecture ».
    ToggleFullscreen,
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
    /// Arrête la lecture (morceau ou webradio) : plus rien en cours, la file d'attente reste.
    Stop,
    /// Note d'un morceau (0 : pas de note).
    SetRating(usize, u8),
    /// Redemande au serveur la liste de ses morceaux.
    SyncServer(String),
    /// Retire un serveur de la bibliothèque.
    RemoveServer(String),
    /// Ouvre un outil (conversion, CD, synchronisation).
    OpenTool(tools::Tool),
    /// Convertit ces morceaux.
    Convert(Vec<usize>),
    /// Diffuse sur l'appareil n° k de la liste, ou revient sur l'ordinateur.
    StartCast(usize),
    StopCast,
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
    /// Morceaux sautés (passés avant d'avoir compté), datés comme les écoutes.
    skips: Vec<crate::history::Listen>,
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
    /// Choix du visualiseur : bandeau d'aperçus ouvert (« En cours de lecture ») et défilement
    /// à faire jusqu'au visualiseur choisi, galerie des paramètres, moment du dernier changement
    /// (la pastille s'allume), visualiseur à l'écran.
    viz_strip: bool,
    viz_strip_scroll: bool,
    /// Carte des réglages de l'effet en cours, ouverte sous la pastille.
    viz_strip_tune: bool,
    viz_gallery: Option<viz_view::Gallery>,
    viz_flash: f64,
    np_shown: usize,
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
    /// Notes inscrites dans les fichiers (thread à part).
    ratings: ratings::RatingWriter,
    /// Tri par colonnes des listes de morceaux d'un album, d'une playlist, d'un dossier (le
    /// temps de la session), par vue.
    list_sorts: HashMap<String, lists::SortSpec>,
    /// Page Playlists : filtre.
    pl_filter: String,
    /// Couleurs et pochettes qui changent en douceur d'un morceau à l'autre.
    ambience: ambience::Ambience,
    /// « En cours de lecture » : le morceau affiché (il suit la pochette, qui attend d'être
    /// chargée), et les textes du précédent, qui s'effacent.
    np_track: Option<usize>,
    np_prev_text: Option<[String; 3]>,
    np_dir: f32,
    /// Barre de lecture : le morceau affiché et le précédent (chemin, titre, artiste, album), qui
    /// s'efface.
    bar_track: Option<(usize, [String; 4])>,
    bar_prev: Option<[String; 4]>,
    /// Vrai plein écran : la vue d'avant, et le dernier mouvement de la souris (les commandes se
    /// cachent quand elle ne bouge plus).
    fullscreen: bool,
    fs_navigated: bool,
    fs_last_move: f64,
    fs_changed: f64,
    /// Molette = volume : la zone du réglage de volume, et la page « En cours de lecture »
    /// (sa zone, moins ce qui défile déjà à la molette). Redonnées à chaque image.
    volume_zone: Option<egui::Rect>,
    np_wheel: (Option<egui::Rect>, Vec<egui::Rect>),
    /// Icône dans la zone de notification, et fenêtre cachée dedans.
    tray: Option<crate::tray::Tray>,
    hidden: bool,
    /// Cadence des images (pas de synchronisation verticale).
    pacer: pace::Pacer,
    /// « Quitter » demandé : la fermeture de la fenêtre n'est plus interceptée.
    quitting: bool,
    /// La fenêtre est au premier plan (pas de notification alors).
    focused: bool,
    scrobbler: crate::scrobble::Scrobbler,
    /// Paramètres : sorties audio connues, connexion aux services de scrobbling.
    outputs: Vec<crate::outputs::Output>,
    online: settings_extra::OnlineState,
    egui_ctx: egui::Context,
    /// Place dans la file du morceau arrêté (bouton Arrêter) : « Lecture » y reprend.
    stopped_at: Option<usize>,
    /// Morceaux des serveurs de musique (par serveur), et les actualisations en cours.
    server_tracks: HashMap<String, Vec<library::Track>>,
    server_rx: Vec<(String, Receiver<ServerFetch>)>,
    /// Podcasts : épisode en cours, flux, recherche, téléchargements.
    podcasts: podcasts_view::PodcastState,
    tools: tools::ToolsState,
    cast: cast_view::CastState,
}

/// Morceaux d'un serveur de musique, ou pourquoi on n'a pas pu les avoir.
type ServerFetch = Result<Vec<library::Track>, String>;

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

/// Volume maximal : 150 % (au-delà de 100 %, le son est amplifié, voir `dsp::set_boost`).
pub const VOLUME_MAX: f32 = 1.5;

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
        // Visualiseurs v4 : dix de plus ; « Automatique » était le 28.
        if settings.style_version < 4 {
            settings.style_version = 4;
            for style in [&mut settings.viz_style, &mut settings.banner_style] {
                if *style == 28 {
                    *style = crate::shader::AUTO;
                }
            }
        }
        // Visualiseurs v5 : douze de plus (cinquante) ; « Automatique » était le 38.
        if settings.style_version < 5 {
            settings.style_version = 5;
            for style in [&mut settings.viz_style, &mut settings.banner_style] {
                if *style == 38 {
                    *style = crate::shader::AUTO;
                }
            }
        }
        theme::install(&cc.egui_ctx, &settings.theme, settings.breeze);

        let volume = if settings.muted {
            0.0
        } else {
            perceptual(settings.volume.min(1.0))
        };
        let audio = crate::dsp::AudioSettings::new();
        audio.set(
            settings.eq_on,
            &settings.eq_gains,
            settings.eq_preamp,
            settings.leveling,
        );
        audio.set_exact(settings.bit_perfect);
        let volume = if settings.bit_perfect && !settings.muted { 1.0 } else { volume };
        let tap = crate::dsp::Tap::new();
        crate::outputs::route_to(&settings.audio_output);
        let (player, toast) = match Player::new(volume, audio.clone(), tap.clone()) {
            Ok(mut p) => {
                let _ = p.set_exact(settings.bit_perfect);
                (Some(p), None)
            }
            Err(e) => (None, Some((e, 0.0))),
        };

        let page = settings.page;
        let (listens, skips) = crate::history::load();
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
            listens,
            skips,
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
            viz_strip: false,
            viz_strip_scroll: false,
            viz_strip_tune: false,
            viz_gallery: None,
            viz_flash: f64::NEG_INFINITY,
            np_shown: 0,
            radio: Default::default(),
            browse_index: None,
            sleep: None,
            sleep_gain: 1.0,
            notifier: Default::default(),
            pending_note: None,
            ratings: ratings::RatingWriter::new(&cc.egui_ctx),
            list_sorts: HashMap::new(),
            pl_filter: String::new(),
            ambience: Default::default(),
            np_track: None,
            np_prev_text: None,
            np_dir: 1.0,
            bar_track: None,
            bar_prev: None,
            fullscreen: false,
            fs_navigated: false,
            fs_last_move: 0.0,
            fs_changed: 0.0,
            volume_zone: None,
            np_wheel: (None, Vec::new()),
            tray: None,
            hidden: false,
            pacer: Default::default(),
            quitting: false,
            focused: true,
            scrobbler: crate::scrobble::Scrobbler::new(settings.scrobble.clone()),
            outputs: Vec::new(),
            online: Default::default(),
            egui_ctx: cc.egui_ctx.clone(),
            stopped_at: None,
            server_tracks: HashMap::new(),
            server_rx: Vec::new(),
            podcasts: Default::default(),
            tools: Default::default(),
            cast: Default::default(),
            settings,
        };
        if !app.settings.smart_seeded {
            app.settings.smart_seeded = true;
            app.settings.smart_presets = smart::PRESETS_V2;
            app.settings
                .smart_playlists
                .extend(smart::SmartPlaylist::presets());
        } else if app.settings.smart_presets < smart::PRESETS_V2 {
            // Playlists d'office arrivées depuis : ajoutées une fois, à la suite des autres.
            app.settings.smart_presets = smart::PRESETS_V2;
            app.settings
                .smart_playlists
                .extend(smart::SmartPlaylist::presets().into_iter().skip(smart::PRESETS_V1));
        }
        for p in &mut app.settings.smart_playlists {
            p.guess_icon();
        }
        app.seed_pins();
        if app.settings.auto_update {
            app.watcher = crate::watch::Watcher::start(&app.settings.music_dirs, &cc.egui_ctx);
        }
        // Morceaux des serveurs : ceux de la dernière actualisation, tout de suite.
        for s in app.settings.servers.iter().filter(|s| s.enabled) {
            let tracks = crate::servers::load_cache(&s.id);
            app.server_tracks.insert(s.id.clone(), tracks);
        }
        if !app.server_tracks.is_empty() {
            let tracks = app.lib.tracks.clone();
            app.set_library(tracks);
        }
        app.apply_volume();
        app.restore_queue();
        if app.settings.tray {
            app.start_tray();
        }
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
        // Les morceaux des serveurs et ceux ouverts hors bibliothèque ne comptent pas.
        let local = self
            .lib
            .tracks
            .iter()
            .filter(|t| t.remote.is_none() && !self.external.iter().any(|e| e.path == t.path))
            .count();
        let changed = tracks.len() != local
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
        // Les morceaux ouverts hors bibliothèque restent jouables après un rescan ; ceux des
        // serveurs actifs s'y ajoutent (ceux d'un serveur coupé ou retiré s'en vont).
        let servers: HashSet<&str> = self
            .settings
            .servers
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.id.as_str())
            .collect();
        tracks.retain(|t| t.remote.as_ref().is_none_or(|r| servers.contains(r.server.as_str())));
        let mut present: HashSet<PathBuf> = tracks.iter().map(|t| t.path.clone()).collect();
        let extra = self.external.iter().chain(
            self.server_tracks
                .iter()
                .filter(|(id, _)| servers.contains(id.as_str()))
                .flat_map(|(_, v)| v.iter()),
        );
        for t in extra {
            if present.insert(t.path.clone()) {
                tracks.push(t.clone());
            }
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
        self.sync_ratings();
        // Les numéros de morceaux ont changé : les fondus repartent du morceau en cours.
        self.np_track = None;
        self.np_prev_text = None;
        self.bar_track = None;
        self.bar_prev = None;
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
            if player.load(&library::media(&self.lib.tracks[idx]), true).is_ok() {
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
        if self.radio.live.is_some() || self.podcasts.playing.is_some() {
            return None;
        }
        self.queue_pos.and_then(|p| self.queue.get(p).copied())
    }

    fn is_playing(&self) -> bool {
        self.player.as_ref().is_some_and(Player::is_playing)
    }

    fn play_current(&mut self) {
        self.leave_radio();
        self.leave_episode();
        let Some(idx) = self.current() else { return };
        if self.player.is_none() {
            return;
        }
        let meta = self.cast_meta(idx);
        let album = self.queue_pos.is_some_and(|p| self.album_run(p));
        let Some(player) = &mut self.player else { return };
        player.set_meta(meta);
        player.set_album_gain(album);
        let track = &self.lib.tracks[idx];
        let result = if self.crossfading {
            let duration = Duration::from_secs_f32(self.settings.crossfade_secs.clamp(1.0, 12.0));
            player.crossfade_to(&library::media(track), duration)
        } else {
            player.load(&library::media(track), false)
        };
        match result {
            Ok(()) => self.track_started(idx),
            Err(e) => {
                let msg = format!("{} — {e}", track.title);
                self.toast(msg);
            }
        }
        self.seeked = true;
    }

    /// Un morceau vient de démarrer : écoute, scrobbling, notification.
    fn track_started(&mut self, idx: usize) {
        let track = &self.lib.tracks[idx];
        let path = track.path.clone();
        let song = scrobble_song(track);
        self.finish_listen_as(true);
        if let Some(song) = song {
            if self.settings.scrobble.any() {
                self.scrobbler.now_playing(song);
            }
        }
        let now = self.egui_ctx.input(|i| i.time);
        self.pending_note = Some((path.clone(), now + 1.5));
        // L'écoute n'est comptée qu'au bout de 30 secondes (voir `track_listen`).
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

    /// Fondu enchaîné : quelques secondes avant la fin du morceau, le suivant démarre en fondu.
    fn maybe_crossfade(&mut self) {
        // La minuterie « fin du morceau » arrête la lecture : pas de morceau suivant.
        if !self.settings.crossfade || self.settings.bit_perfect || self.sleep_at_end() {
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
        // Deux morceaux d'un même album s'enchaînent sans blanc, pas en fondu.
        if let Some(next) = self.upcoming() {
            if !self.will_crossfade(cur, self.queue[next]) {
                return;
            }
        }
        self.crossfading = true;
        self.next(true);
        self.crossfading = false;
    }

    fn sleep_at_end(&self) -> bool {
        self.sleep.as_ref().is_some_and(|s| s.kind == SleepKind::EndOfTrack)
    }

    /// Position dans la file du morceau qui suivra tout seul celui en cours ; `None` s'il n'y
    /// en a pas, ou s'il n'est pas encore connu (fin de la file en aléatoire : elle sera remélangée).
    fn upcoming(&self) -> Option<usize> {
        let pos = self.queue_pos?;
        if self.settings.repeat == Repeat::One {
            return Some(pos);
        }
        if pos + 1 < self.queue.len() {
            return Some(pos + 1);
        }
        let reshuffled = self.settings.shuffle && self.queue.len() > 1;
        (self.settings.repeat == Repeat::All && !reshuffled).then_some(0)
    }

    /// Vrai si les morceaux `a` et `b` (indices de la bibliothèque) sont du même album, ou du
    /// même fichier découpé par une feuille CUE.
    fn same_album(&self, a: usize, b: usize) -> bool {
        let (a, b) = (&self.lib.tracks[a], &self.lib.tracks[b]);
        if let (Some(x), Some(y)) = (&a.cue, &b.cue) {
            if x.file == y.file {
                return true;
            }
        }
        !a.album.trim().is_empty()
            && library::fold(&a.album) == library::fold(&b.album)
            && library::fold(a.album_artist_or_artist()) == library::fold(b.album_artist_or_artist())
    }

    /// Vrai si le morceau de la file en position `pos` est écouté au milieu de son album (le
    /// précédent ou le suivant de la file en est aussi) : ReplayGain prend alors le gain d'album.
    fn album_run(&self, pos: usize) -> bool {
        let Some(&idx) = self.queue.get(pos) else { return false };
        let neighbor = |p: Option<usize>| p.and_then(|p| self.queue.get(p)).is_some_and(|&n| n != idx && self.same_album(idx, n));
        neighbor(pos.checked_sub(1)) || neighbor(Some(pos + 1))
    }

    /// Vrai si le passage de `cur` à `next` se fera en fondu enchaîné.
    fn will_crossfade(&self, cur: usize, next: usize) -> bool {
        let fade = self.settings.crossfade_secs.clamp(1.0, 12.0);
        self.settings.crossfade
            && !self.settings.bit_perfect
            && (self.settings.crossfade_albums || !self.same_album(cur, next))
            && self.lib.tracks[cur].duration().as_secs_f32() >= fade * 3.0
    }

    /// Lecture sans blanc : quelques secondes avant la fin du morceau, le suivant est mis à sa
    /// suite dans la sortie audio (sauf s'il doit venir en fondu enchaîné). La file d'attente
    /// peut changer d'ici là : ce qui est mis à la suite est revérifié à chaque image.
    fn keep_next_queued(&mut self) {
        /// Avance avec laquelle le morceau suivant est ouvert.
        const AHEAD: f32 = 15.0;
        let Some(cur) = self.current() else { return };
        let Some(p) = &self.player else { return };
        if !p.is_playing() || p.casting().is_some() || p.is_fading() || p.is_live() {
            return;
        }
        let remaining = self.lib.tracks[cur].duration().as_secs_f32() - p.position().as_secs_f32();
        let next = self
            .upcoming()
            .filter(|_| !self.sleep_at_end() && remaining < AHEAD)
            .filter(|&pos| !self.will_crossfade(cur, self.queue[pos]));
        let want = next.map(|pos| (library::media(&self.lib.tracks[self.queue[pos]]), self.album_run(pos)));
        let Some(p) = &mut self.player else { return };
        match want {
            Some((media, album)) => {
                if p.queued() != Some(&media) {
                    p.set_album_gain(album);
                    p.queue_next(&media);
                }
            }
            None => {
                if p.queued().is_some() {
                    p.clear_next();
                }
            }
        }
    }

    /// Le morceau mis à la suite vient de démarrer, sans blanc : la file avance avec lui.
    fn gapless_advanced(&mut self, media: crate::media::Media) {
        let matches = |s: &Self, pos: usize| s.queue.get(pos).is_some_and(|&i| library::media(&s.lib.tracks[i]) == media);
        let pos = match self.upcoming().filter(|&p| matches(self, p)) {
            Some(p) => p,
            // La file a changé au dernier moment : on retrouve le morceau.
            None => match (0..self.queue.len()).find(|&p| matches(self, p)) {
                Some(p) => p,
                None => return,
            },
        };
        self.queue_pos = Some(pos);
        let idx = self.queue[pos];
        self.track_started(idx);
        self.seeked = true;
    }

    /// Sortie bit-perfect activée ou coupée.
    pub(super) fn set_bit_perfect(&mut self, on: bool) {
        if on == self.settings.bit_perfect {
            return;
        }
        self.settings.bit_perfect = on;
        self.audio.set_exact(on);
        let live = self.radio.live.as_ref().filter(|l| !l.paused).map(|l| l.station.clone());
        if let Some(p) = &mut self.player {
            if let Err(e) = p.set_exact(on) {
                self.toast(e);
            }
        }
        self.apply_volume();
        if let Some(station) = live {
            let ctx = self.egui_ctx.clone();
            self.play_station(&ctx, station);
        }
        self.seeked = true;
    }

    /// Visualiseur : analyse le son quand une page qui l'affiche est à l'écran, et redessine en
    /// continu pendant la lecture (ou tant que l'animation retombe).
    fn update_visualizer(&mut self, ctx: &egui::Context) {
        // Aussi pour les aperçus animés (paramètres, galerie).
        let visible = self.settings.visualizer
            && !self.mini
            && !self.hidden
            && !self.kwin.minimized
            && (matches!(self.view, View::NowPlaying | View::Page(Page::Home) | View::Page(Page::Settings))
                || self.viz_gallery.is_some());
        if !visible {
            return;
        }
        let playing = self.is_playing();
        let dt = ctx.input(|i| i.stable_dt);
        self.spectrum.update(&self.tap, dt, playing, self.settings.viz_speed);
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
        self.stopped_at = None;
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
        let mut rest = std::mem::take(&mut self.queue);
        fastrand::shuffle(&mut rest);
        self.queue = rest;
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
        // Épisode arrêté (fin de minuterie, fin d'écoute) : il repart là où il en était.
        if !has_track {
            if let Some(p) = self.podcasts.playing.take() {
                self.play_episode(p.podcast, p.episode);
                return;
            }
        }
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
        } else if !self.queue.is_empty() {
            // Après « Arrêter » : la file reprend au morceau arrêté.
            let pos = self.stopped_at.take().filter(|&p| p < self.queue.len()).unwrap_or(0);
            self.queue_pos = Some(pos);
            self.play_current();
        } else if !self.lib.is_empty() {
            let all: Vec<usize> = self.sorted_songs().to_vec();
            self.play_list(all, 0);
        }
    }

    /// Bouton Arrêter : coupe le morceau ou la webradio en cours ; il n'y a plus rien en
    /// lecture, mais la file d'attente reste telle quelle.
    fn stop_playback(&mut self) {
        self.leave_radio();
        self.leave_episode();
        if let Some(p) = &mut self.player {
            p.stop();
        }
        self.finish_listen();
        if self.queue_pos.is_some() {
            self.stopped_at = self.queue_pos;
        }
        self.queue_pos = None;
        self.pending_note = None;
        if self.sleep.take().is_some() {
            self.sleep_gain = 1.0;
            self.apply_volume();
        }
        self.seeked = true;
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
                let mut all = std::mem::take(&mut self.queue);
                fastrand::shuffle(&mut all);
                self.queue = all;
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

    /// Molette sur le réglage de volume, ou sur la page « En cours de lecture » : un cran, 5 %.
    /// Les zones sont celles de l'image précédente ; rien ne se passe sous une fenêtre flottante.
    fn volume_wheel(&mut self, ctx: &egui::Context) {
        let zone = self.volume_zone.take();
        let (page, excluded) = std::mem::take(&mut self.np_wheel);
        if self.settings.bit_perfect || self.dialog.is_some() {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
        let on_page = page.is_some_and(|r| r.contains(pos)) && !excluded.iter().any(|r| r.contains(pos));
        if !(zone.is_some_and(|r| r.contains(pos)) || on_page) {
            return;
        }
        // Une bulle, un menu ou le bandeau des visualiseurs est au-dessus : la molette est à eux.
        if ctx.layer_id_at(pos).is_some_and(|l| l.order > egui::Order::Middle) {
            return;
        }
        let steps: f32 = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * 4.0,
                        // Pavé tactile : une quarantaine de points pour un cran.
                        egui::MouseWheelUnit::Point => delta.y / 40.0,
                    }),
                    _ => None,
                })
                .sum()
        });
        if steps.abs() > 0.001 {
            self.actions.push(Action::SetVolume(self.settings.volume + 0.05 * steps.clamp(-4.0, 4.0)));
        }
    }

    fn apply_volume(&mut self) {
        let v = if self.settings.muted {
            0.0
        } else if self.settings.bit_perfect {
            // Le son part tel quel : le volume se règle dans le système (seule la minuterie de
            // sommeil fait son fondu).
            self.sleep_gain
        } else {
            perceptual(self.settings.volume.min(1.0)) * self.sleep_gain
        };
        // Au-delà de 100 % : le reste du gain passe par l'amplification (avec limiteur).
        let boost = if self.settings.bit_perfect {
            1.0
        } else {
            perceptual(self.settings.volume.max(1.0))
        };
        self.audio.set_boost(boost);
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
        self.finish_listen_as(false);
    }

    /// Fin de l'écoute en cours. `moved_on` : on est passé à un autre morceau (et non arrêté,
    /// fermé SterMusic ou parti sur une webradio). L'écoute est alors :
    /// - **comptée** si elle avait atteint 30 secondes (ou la moitié du morceau) : son temps
    ///   s'ajoute, et l'historique dira si elle était complète ou partielle ;
    /// - **sautée** si l'on est passé à autre chose avant, après au moins deux secondes.
    fn finish_listen_as(&mut self, moved_on: bool) {
        let Some(l) = self.listening.take() else { return };
        let seconds = l.heard.round() as u32;
        if !l.counted {
            if moved_on && l.heard >= 2.0 {
                self.settings.plays.entry(l.path.clone()).or_default().skips += 1;
                let skip = crate::history::Listen {
                    time: l.started,
                    path: l.path,
                    seconds,
                    skipped: true,
                };
                crate::history::append(&skip);
                self.skips.push(skip);
                self.smart_cache.borrow_mut().clear();
            }
            return;
        }
        if let Some(stat) = self.settings.plays.get_mut(&l.path) {
            stat.seconds += l.heard as f64;
        }
        let listen = crate::history::Listen {
            time: l.started,
            path: l.path,
            seconds,
            skipped: false,
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

    /// Vrai si la playlist est épinglée dans la barre latérale.
    pub fn is_pinned(&self, id: PlaylistId) -> bool {
        match id {
            PlaylistId::Favorites => self.settings.favorites_pinned,
            PlaylistId::Smart(i) => self.settings.smart_playlists.get(i).is_some_and(|p| p.pinned),
            PlaylistId::User(i) => self.settings.playlists.get(i).is_some_and(|p| p.pinned),
        }
    }

    /// Les playlists épinglées, dans l'ordre de la page Playlists (favoris, intelligentes, les
    /// miennes).
    pub fn pinned(&self) -> Vec<PlaylistId> {
        let s = &self.settings;
        let mut ids: Vec<PlaylistId> = Vec::new();
        ids.extend(s.favorites_pinned.then_some(PlaylistId::Favorites));
        ids.extend(s.smart_playlists.iter().enumerate().filter(|(_, p)| p.pinned).map(|(i, _)| PlaylistId::Smart(i)));
        ids.extend(s.playlists.iter().enumerate().filter(|(_, p)| p.pinned).map(|(i, _)| PlaylistId::User(i)));
        ids
    }

    fn set_pinned(&mut self, id: PlaylistId, on: bool) {
        match id {
            PlaylistId::Favorites => self.settings.favorites_pinned = on,
            PlaylistId::Smart(i) => {
                if let Some(p) = self.settings.smart_playlists.get_mut(i) {
                    p.pinned = on;
                }
            }
            PlaylistId::User(i) => {
                if let Some(p) = self.settings.playlists.get_mut(i) {
                    p.pinned = on;
                }
            }
        }
    }

    fn toggle_pin(&mut self, id: PlaylistId) {
        let on = !self.is_pinned(id);
        if on && self.pinned().len() >= MAX_PINS {
            self.toast(trf!(
                "{0} playlists épinglées au plus : désépingles-en une d'abord.",
                MAX_PINS
            ));
            return;
        }
        self.set_pinned(id, on);
    }

    /// Première fois : épingle les favoris, puis les playlists intelligentes et les miennes,
    /// jusqu'à la limite (la barre latérale les montrait toutes).
    fn seed_pins(&mut self) {
        if std::mem::replace(&mut self.settings.pins_seeded, true) {
            return;
        }
        let mut ids = vec![PlaylistId::Favorites];
        ids.extend((0..self.settings.smart_playlists.len()).map(PlaylistId::Smart));
        ids.extend((0..self.settings.playlists.len()).map(PlaylistId::User));
        for id in ids.into_iter().take(MAX_PINS) {
            self.set_pinned(id, true);
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
            Action::PlayByTaste(mut tracks) => {
                if tracks.is_empty() {
                    return;
                }
                taste::taste_order(&mut tracks, |&i| self.taste_weight(i));
                // L'ordre est celui des goûts : l'aléatoire de la barre s'éteint pour ne pas le défaire.
                if self.settings.shuffle {
                    self.settings.shuffle = false;
                }
                self.play_list(tracks, 0);
                self.toast(tr("Selon mes goûts : la file va de tes morceaux préférés aux moins aimés"));
            }
            Action::PlayShuffled(tracks) => {
                if tracks.is_empty() {
                    return;
                }
                self.settings.shuffle = true;
                let first = fastrand::usize(..tracks.len());
                self.play_list(tracks, first);
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
            // Le morceau est désigné lui-même (la liste affichée peut être triée ou filtrée) : sa
            // première occurrence quitte la playlist.
            Action::RemoveFromPlaylist(id, track) => {
                let Some(path) = self.lib.tracks.get(track).map(|t| t.path.clone()) else {
                    return;
                };
                match id {
                    // Une playlist intelligente se remplit d'après ses règles : rien à retirer.
                    PlaylistId::Smart(_) => {}
                    PlaylistId::Favorites => {
                        self.settings.favorites.remove(&path);
                    }
                    PlaylistId::User(i) => {
                        if let Some(pl) = self.settings.playlists.get_mut(i) {
                            if let Some(k) = pl.tracks.iter().position(|p| *p == path) {
                                pl.tracks.remove(k);
                            }
                        }
                    }
                }
            }
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
                if let Some(s) = self.stopped_at {
                    self.stopped_at = (pos != s).then(|| if pos < s { s - 1 } else { s });
                }
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
                self.stopped_at = None;
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
            Action::Stop => self.stop_playback(),
            Action::SyncServer(id) => self.sync_server(ctx, &id),
            Action::OpenTool(tool) => self.open_tool(ctx, tool),
            Action::StartCast(k) => self.start_cast(ctx, k),
            Action::StopCast => self.stop_cast(ctx),
            Action::Convert(tracks) => self.convert_tracks(tracks),
            Action::RemoveServer(id) => {
                self.settings.servers.retain(|s| s.id != id);
                self.server_tracks.remove(&id);
                crate::servers::forget(&id);
                let tracks = self.lib.tracks.clone();
                self.set_library(tracks);
            }
            Action::SetRating(i, n) => self.set_rating(i, n),
            Action::Next if self.radio.live.is_some() => self.step_station(ctx, 1),
            Action::Previous if self.radio.live.is_some() => self.step_station(ctx, -1),
            // Podcast : Suivant avance de 30 s, Précédent recule de 15 s.
            Action::Next if self.podcasts.playing.is_some() => self.apply(ctx, Action::SeekBy(30.0)),
            Action::Previous if self.podcasts.playing.is_some() => self.apply(ctx, Action::SeekBy(-15.0)),
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
                // On repart de l'ordre d'origine, puis on remélange.
                self.unshuffle_queue();
                if self.settings.shuffle {
                    self.shuffle_queue();
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
                // Un cran magnétique à 100 % : on ne passe pas en amplification sans le vouloir.
                let was = self.settings.volume;
                let v = if (was - 1.0).abs() > 0.001 && (was - 1.0) * (v - 1.0) < 0.0 { 1.0 } else { v };
                self.settings.volume = v.clamp(0.0, VOLUME_MAX);
                self.settings.muted = false;
                self.apply_volume();
            }
            Action::ToggleMute => {
                self.settings.muted = !self.settings.muted;
                self.apply_volume();
            }
            Action::ToggleMini => self.toggle_mini(ctx),
            Action::TogglePin(id) => self.toggle_pin(id),
            Action::ToggleFullscreen => self.toggle_fullscreen(ctx),
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

    /// Vrai plein écran : la fenêtre prend tout l'écran sur « En cours de lecture », sans barre de
    /// titre ; la barre de lecture et le pointeur se cachent quand la souris ne bouge plus.
    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        if self.mini {
            return;
        }
        let now = ctx.input(|i| i.time);
        self.fullscreen = !self.fullscreen;
        self.fs_changed = now;
        self.fs_last_move = now;
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.fullscreen));
        if self.fullscreen {
            // On y entre par « En cours de lecture » ; en sortant, on revient d'où l'on venait.
            self.fs_navigated = self.view != View::NowPlaying && self.current().is_some();
            if self.fs_navigated {
                self.actions.push(Action::Go(View::NowPlaying));
            }
        } else if std::mem::take(&mut self.fs_navigated) && self.view == View::NowPlaying {
            self.actions.push(Action::Back);
        }
    }

    /// Plein écran sur « En cours de lecture » : l'interface s'efface (ailleurs, elle reste).
    pub(super) fn immersive(&self) -> bool {
        self.fullscreen && self.view == View::NowPlaying && !self.mini
    }

    /// En plein écran : vrai tant que la souris a bougé récemment, qu'elle est en bas de l'écran,
    /// qu'une bulle est ouverte ou que rien ne joue.
    pub(super) fn fs_controls_visible(&mut self, ctx: &egui::Context) -> bool {
        let now = ctx.input(|i| i.time);
        let active = ctx.input(|i| {
            i.pointer.delta() != egui::Vec2::ZERO || i.pointer.any_down() || !i.events.is_empty()
        });
        if active {
            self.fs_last_move = now;
        }
        let low = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| p.y > ctx.screen_rect().bottom() - 110.0);
        let busy = ctx.memory(|m| m.any_popup_open()) || self.viz_strip || self.viz_strip_tune || self.dialog.is_some();
        let visible = now - self.fs_last_move < 3.0 || low || busy || !self.is_playing();
        if visible {
            // Pour se replier à l'heure, même sans nouvel événement.
            ctx.request_repaint_after(Duration::from_millis(500));
        } else {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
        visible
    }

    /// Suit un plein écran mis ou retiré par le bureau lui-même (raccourci de KWin…).
    fn sync_fullscreen(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if now - self.fs_changed < 1.5 {
            return;
        }
        if let Some(real) = ctx.input(|i| i.viewport().fullscreen) {
            if real != self.fullscreen {
                self.fullscreen = real;
                self.fs_navigated = false;
                self.fs_last_move = now;
            }
        }
    }

    fn toggle_mini(&mut self, ctx: &egui::Context) {
        if self.fullscreen {
            self.toggle_fullscreen(ctx);
        }
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
            pinned: false,
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
                Remote::Stop => Action::Stop,
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
            || self.tools.open.is_some()
            || self.viz_gallery.is_some()
        {
            return;
        }
        self.viz_keys(ctx);
        if ctx.input_mut(|i| i.consume_key(none, Key::F11)) {
            self.actions.push(Action::ToggleFullscreen);
        }
        // Échap quitte le plein écran (après avoir fermé ce qui était ouvert par-dessus).
        if self.fullscreen && ctx.input_mut(|i| i.consume_key(none, Key::Escape)) {
            self.actions.push(Action::ToggleFullscreen);
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
        // Ctrl+1 à Ctrl+5 : noter le morceau en cours ; Ctrl+0 : retirer sa note.
        if let Some(cur) = self.current() {
            let digits = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5];
            for (n, key) in digits.into_iter().enumerate() {
                if ctx.input_mut(|i| i.consume_key(cmd, key)) {
                    self.actions.push(Action::SetRating(cur, n as u8));
                    let msg = if n == 0 {
                        tr("Note retirée").to_owned()
                    } else {
                        trf!("Noté {0}", "★".repeat(n))
                    };
                    self.toast(msg);
                }
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
            cue: None,
            rating: None,
            remote: None,
        });
        // Épisode de podcast : le titre de l'épisode, le podcast en artiste.
        let radio_track = radio_track.or_else(|| {
            self.podcasts.playing.as_ref().map(|e| library::Track {
                path: PathBuf::from(format!("podcast:{}", e.episode.key())),
                title: e.episode.title.clone(),
                artist: e.podcast.title.clone(),
                album: tr("Podcast").into(),
                album_artist: String::new(),
                genre: String::new(),
                year: None,
                track_no: None,
                disc_no: None,
                duration_ms: e.episode.duration_ms,
                mtime: 0,
                composer: String::new(),
                schema: 0,
                cue: None,
                rating: None,
                remote: None,
            })
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
        if let Some(tray) = &mut self.tray {
            let track = match (current.map(|i| &self.lib.tracks[i]), &radio_track) {
                (_, Some(t)) => format!("{} — {}", t.title, t.artist),
                (Some(t), None) => format!("{} — {}", t.title, t.artist_or_unknown()),
                (None, None) => String::new(),
            };
            tray.set_state(crate::tray::State {
                playing: state == mpris::State::Playing,
                track,
                hidden: self.hidden,
            });
        }
        // Le script KWin garde au premier plan la fenêtre dont le titre porte cette marque, et
        // cache celle qui porte l'autre.
        let title = if self.hidden {
            format!("{title}{}", crate::kwin::HIDDEN_MARK)
        } else if self.mini {
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
        let unseen = self.hidden || self.kwin.minimized;
        self.pacer.wait(self.kwin.geometry.as_ref().map(|g| g.output.as_str()), unseen);
        self.poll_scan();
        self.poll_tag_editor(ctx);
        self.poll_lib_tools(ctx);
        self.sync_now_loaded();
        self.track_listen();
        self.poll_watch(ctx);
        self.poll_lyrics(ctx);
        self.poll_radio(ctx);
        self.poll_online(ctx);
        self.poll_servers(ctx);
        self.poll_podcasts(ctx);
        self.poll_tools(ctx);
        self.poll_cast(ctx);
        self.poll_ratings();
        self.focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        self.send_pending_note(ctx);
        self.sleep_tick(ctx);
        self.poll_tray(ctx);
        for request in self.ipc.poll() {
            self.hidden = false;
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
        // Lecture sans blanc : le morceau mis à la suite a démarré.
        if let Some(media) = self.player.as_mut().and_then(Player::take_started) {
            self.gapless_advanced(media);
        }
        // Fin d'un morceau (une webradio coupée est relancée par `poll_radio`).
        if self.radio.live.is_none() && self.player.as_ref().is_some_and(Player::finished) {
            if self.podcasts.playing.is_some() {
                self.episode_finished();
            } else if self.sleep.as_ref().is_some_and(|s| s.kind == SleepKind::EndOfTrack) {
                self.sleep_done();
            } else {
                self.next(true);
            }
        }
        self.maybe_crossfade();
        self.keep_next_queued();
        if let Some(p) = &mut self.player {
            p.tick();
            if p.is_fading() {
                ctx.request_repaint_after(Duration::from_millis(30));
            }
            if p.holds_idle_output() {
                ctx.request_repaint_after(crate::player::RELEASE_AFTER + Duration::from_millis(50));
            }
        }
        self.update_ambience(ctx);
        self.handle_keys(ctx);
        self.volume_wheel(ctx);
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
            self.sync_fullscreen(ctx);
            let resize = chrome::resize_direction(ctx);
            if !self.immersive() {
                self.title_bar(ctx);
            }
            self.now_bar(ctx);
            if self.view != View::NowPlaying {
                self.sidebar(ctx);
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| self.content(ui));
            self.dialogs(ctx);
            self.viz_gallery_ui(ctx);
            self.radio_dialog(ctx);
            self.tool_dialogs(ctx);
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
        self.save_episode_progress();
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
        let now = ctx.input(|i| i.time);
        // L'ancien fond reste le temps que le nouveau soit chargé, puis ils se fondent.
        let loaded = self.covers.large(&path).map(|l| l.blurred.clone());
        self.ambience.backdrop.update(now, &path, loaded);
        let fade = self.ambience.backdrop.t(now);
        let back = &self.ambience.backdrop;
        let shown = crate::anim::toggle(
            ctx,
            egui::Id::new("cover_backdrop"),
            back.current().is_some() || back.previous(now).flatten().is_some(),
            crate::anim::SLOW,
        );
        let painter = ctx.layer_painter(egui::LayerId::background());
        let screen = ctx.screen_rect();
        let radius = egui::CornerRadius::same(theme::window_radius());
        let gray = egui::Color32::from_gray(150);
        let old = back.previous(now).flatten();
        if let Some(tex) = old {
            backdrop::paint_fill(&painter, tex, screen, gray.gamma_multiply(shown), radius);
        }
        if let Some(tex) = back.current() {
            // Par-dessus l'ancien : opaque à la fin du fondu. Sans ancien, il monte seul.
            let a = if old.is_some() { fade } else { shown };
            backdrop::paint_fill(&painter, tex, screen, gray.gamma_multiply(a), radius);
        }
        if back.current().is_some() || old.is_some() {
            painter.rect_filled(screen, radius, theme::alpha(theme::bg(), (0x70 as f32 * shown) as u8));
        }
        if back.fading(now) {
            ctx.request_repaint();
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
