//! Bibliothèque : scan des dossiers (avec cache), tags, regroupement en albums et artistes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, UNIX_EPOCH};

use lofty::prelude::*;
use lofty::probe::Probe;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "wav", "m4a", "aac", "opus", "aiff", "aif",
];

pub const UNKNOWN_ARTIST: &str = "Artiste inconnu";
pub const UNKNOWN_ALBUM: &str = "Album inconnu";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    #[serde(default)]
    pub album_artist: String,
    #[serde(default)]
    pub genre: String,
    pub year: Option<u32>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub duration_ms: u64,
    /// Date de modification du fichier (secondes) : sert au cache et à « Ajoutés récemment ».
    pub mtime: u64,
}

impl Track {
    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.duration_ms)
    }

    pub fn artist_or_unknown(&self) -> &str {
        if self.artist.is_empty() {
            UNKNOWN_ARTIST
        } else {
            &self.artist
        }
    }

    pub fn album_or_unknown(&self) -> &str {
        if self.album.is_empty() {
            UNKNOWN_ALBUM
        } else {
            &self.album
        }
    }

    /// Artiste sous lequel l'album est rangé.
    pub fn album_artist_or_artist(&self) -> &str {
        if !self.album_artist.is_empty() {
            &self.album_artist
        } else {
            self.artist_or_unknown()
        }
    }
}

#[derive(Debug, Clone)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub year: Option<u32>,
    pub genre: String,
    pub tracks: Vec<usize>,
    pub added: u64,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub struct Artist {
    pub name: String,
    pub albums: Vec<usize>,
    pub tracks: Vec<usize>,
}

#[derive(Default)]
pub struct Library {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    /// Album de chaque morceau.
    pub track_album: Vec<usize>,
    /// Artiste de chaque album.
    pub album_artist: Vec<usize>,
    search_text: Vec<String>,
    by_path: HashMap<PathBuf, usize>,
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

impl Library {
    pub fn new(mut tracks: Vec<Track>) -> Self {
        tracks.sort_by(|a, b| {
            lower(a.album_artist_or_artist())
                .cmp(&lower(b.album_artist_or_artist()))
                .then_with(|| lower(&a.album).cmp(&lower(&b.album)))
                .then_with(|| a.disc_no.unwrap_or(1).cmp(&b.disc_no.unwrap_or(1)))
                .then_with(|| {
                    a.track_no
                        .unwrap_or(u32::MAX)
                        .cmp(&b.track_no.unwrap_or(u32::MAX))
                })
                .then_with(|| lower(&a.title).cmp(&lower(&b.title)))
        });

        // Albums : même artiste d'album + même titre.
        let mut album_index: HashMap<(String, String), usize> = HashMap::new();
        let mut albums: Vec<Album> = Vec::new();
        let mut track_album = Vec::with_capacity(tracks.len());
        for (i, t) in tracks.iter().enumerate() {
            let key = (
                lower(t.album_artist_or_artist()),
                lower(t.album_or_unknown()),
            );
            let idx = *album_index.entry(key).or_insert_with(|| {
                albums.push(Album {
                    title: t.album_or_unknown().to_owned(),
                    artist: t.album_artist_or_artist().to_owned(),
                    year: None,
                    genre: String::new(),
                    tracks: Vec::new(),
                    added: 0,
                    duration: Duration::ZERO,
                });
                albums.len() - 1
            });
            let a = &mut albums[idx];
            a.tracks.push(i);
            a.added = a.added.max(t.mtime);
            a.duration += t.duration();
            if a.year.is_none() {
                a.year = t.year;
            }
            if a.genre.is_empty() {
                a.genre = t.genre.clone();
            }
            track_album.push(idx);
        }

        // Albums triés par titre ; on renumérote les références.
        let mut order: Vec<usize> = (0..albums.len()).collect();
        order.sort_by(|&a, &b| {
            let (x, y) = (&albums[a], &albums[b]);
            (x.title == UNKNOWN_ALBUM)
                .cmp(&(y.title == UNKNOWN_ALBUM))
                .then_with(|| lower(&x.title).cmp(&lower(&y.title)))
                .then_with(|| lower(&x.artist).cmp(&lower(&y.artist)))
        });
        let mut remap = vec![0; albums.len()];
        for (new, &old) in order.iter().enumerate() {
            remap[old] = new;
        }
        let mut sorted: Vec<Option<Album>> = albums.into_iter().map(Some).collect();
        let albums: Vec<Album> = order.iter().map(|&o| sorted[o].take().unwrap()).collect();
        let track_album: Vec<usize> = track_album.into_iter().map(|a| remap[a]).collect();

        // Artistes (d'album).
        let mut artist_index: HashMap<String, usize> = HashMap::new();
        let mut artists: Vec<Artist> = Vec::new();
        let mut album_artist = vec![0; albums.len()];
        for (ai, a) in albums.iter().enumerate() {
            let idx = *artist_index.entry(lower(&a.artist)).or_insert_with(|| {
                artists.push(Artist {
                    name: a.artist.clone(),
                    albums: Vec::new(),
                    tracks: Vec::new(),
                });
                artists.len() - 1
            });
            artists[idx].albums.push(ai);
            artists[idx].tracks.extend(&a.tracks);
            album_artist[ai] = idx;
        }
        let mut order: Vec<usize> = (0..artists.len()).collect();
        order.sort_by(|&a, &b| {
            let (x, y) = (&artists[a], &artists[b]);
            (x.name == UNKNOWN_ARTIST)
                .cmp(&(y.name == UNKNOWN_ARTIST))
                .then_with(|| lower(&x.name).cmp(&lower(&y.name)))
        });
        let mut remap = vec![0; artists.len()];
        for (new, &old) in order.iter().enumerate() {
            remap[old] = new;
        }
        let mut sorted: Vec<Option<Artist>> = artists.into_iter().map(Some).collect();
        let mut artists: Vec<Artist> = order.iter().map(|&o| sorted[o].take().unwrap()).collect();
        let album_artist = album_artist.into_iter().map(|a| remap[a]).collect();
        for artist in &mut artists {
            artist.tracks.sort_unstable();
            // Albums les plus récents d'abord.
            artist
                .albums
                .sort_by(|&a, &b| albums[b].year.cmp(&albums[a].year));
        }

        let search_text = tracks
            .iter()
            .map(|t| {
                lower(&format!(
                    "{} {} {} {}",
                    t.title, t.artist, t.album, t.album_artist
                ))
            })
            .collect();
        let by_path = tracks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.path.clone(), i))
            .collect();

        Self {
            tracks,
            albums,
            artists,
            track_album,
            album_artist,
            search_text,
            by_path,
        }
    }

    pub fn find(&self, path: &Path) -> Option<usize> {
        self.by_path.get(path).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Morceaux, albums et artistes qui contiennent tous les mots de la recherche.
    pub fn search(&self, query: &str) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
        let words: Vec<String> = lower(query).split_whitespace().map(str::to_owned).collect();
        if words.is_empty() {
            return Default::default();
        }
        let matches = |text: &str| words.iter().all(|w| text.contains(w.as_str()));
        let tracks = (0..self.tracks.len())
            .filter(|&i| matches(&self.search_text[i]))
            .collect();
        let albums = (0..self.albums.len())
            .filter(|&i| {
                matches(&lower(&format!(
                    "{} {}",
                    self.albums[i].title, self.albums[i].artist
                )))
            })
            .collect();
        let artists = (0..self.artists.len())
            .filter(|&i| matches(&lower(&self.artists[i].name)))
            .collect();
        (tracks, albums, artists)
    }
}

// ---------- Scan ----------

pub enum ScanMsg {
    Progress(usize),
    Done(Vec<Track>),
}

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Lit les tags d'un fichier ; sans tags, le titre est le nom du fichier.
pub fn read_track(path: &Path) -> Track {
    let mut track = Track {
        path: path.to_path_buf(),
        title: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        artist: String::new(),
        album: String::new(),
        album_artist: String::new(),
        genre: String::new(),
        year: None,
        track_no: None,
        disc_no: None,
        duration_ms: 0,
        mtime: mtime(path),
    };
    if let Ok(tagged) = Probe::open(path).and_then(|p| p.read()) {
        track.duration_ms = tagged.properties().duration().as_millis() as u64;
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(t) = tag.title().filter(|t| !t.trim().is_empty()) {
                track.title = t.trim().to_owned();
            }
            let get = |v: Option<std::borrow::Cow<str>>| {
                v.map(|s| s.trim().to_owned()).unwrap_or_default()
            };
            track.artist = get(tag.artist());
            track.album = get(tag.album());
            track.genre = get(tag.genre());
            track.album_artist = tag
                .get_string(&ItemKey::AlbumArtist)
                .map(|s| s.trim().to_owned())
                .unwrap_or_default();
            track.year = tag.year();
            track.track_no = tag.track();
            track.disc_no = tag.disk();
        }
    }
    track
}

/// Parcourt les dossiers ; les fichiers inchangés depuis le dernier scan sont repris du cache.
pub fn scan(dirs: Vec<PathBuf>, cache: HashMap<PathBuf, Track>, tx: Sender<ScanMsg>) {
    let mut tracks = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in &dirs {
        for entry in WalkDir::new(dir)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file() && is_audio_file(e.path()))
        {
            let path = entry.path();
            if !seen.insert(path.to_path_buf()) {
                continue;
            }
            let track = match cache.get(path) {
                Some(t) if t.mtime == mtime(path) => t.clone(),
                _ => read_track(path),
            };
            tracks.push(track);
            if tracks.len() % 50 == 0 {
                let _ = tx.send(ScanMsg::Progress(tracks.len()));
            }
        }
    }
    save_cache(&tracks);
    let _ = tx.send(ScanMsg::Done(tracks));
}

// ---------- Cache sur disque ----------

/// `~/.local/share/stermusic` (ou `$XDG_DATA_HOME/stermusic`).
pub fn data_dir() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share").join("stermusic")
}

/// `~/.cache/stermusic` (ou `$XDG_CACHE_HOME/stermusic`).
pub fn cache_dir() -> PathBuf {
    xdg_dir("XDG_CACHE_HOME", ".cache").join("stermusic")
}

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback)
        })
}

fn cache_file() -> PathBuf {
    data_dir().join("library.json")
}

pub fn load_cache() -> Vec<Track> {
    std::fs::read(cache_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_cache(tracks: &[Track]) {
    let _ = std::fs::create_dir_all(data_dir());
    if let Ok(json) = serde_json::to_vec(tracks) {
        let tmp = cache_file().with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, cache_file());
        }
    }
}

/// Dossier « Musique » de l'utilisateur (xdg-user-dirs), s'il existe.
pub fn default_music_dir() -> Option<PathBuf> {
    let from_xdg = std::process::Command::new("xdg-user-dir")
        .arg("MUSIC")
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .filter(|p| {
            p.is_dir() && p != &PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        });
    from_xdg.or_else(|| {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        ["Musique", "Music"]
            .iter()
            .map(|d| home.join(d))
            .find(|p| p.is_dir())
    })
}

/// Formate une durée en `m:ss` (ou `h:mm:ss`).
pub fn fmt_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Durée longue, lisible : « 1 h 12 min », « 42 min ».
pub fn fmt_long(d: Duration) -> String {
    let m = d.as_secs().div_ceil(60);
    if m >= 60 {
        format!("{} h {:02} min", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n > 1 { many } else { one })
}
