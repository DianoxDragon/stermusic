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
    "mp3", "flac", "ogg", "oga", "wav", "m4a", "aac", "opus", "aiff", "aif", "wma", "ape", "wv", "dsf",
    "dff", "mpc", "tta", "mka", "alac",
];

pub const UNKNOWN_ARTIST: &str = crate::i18n::n_("Artiste inconnu");
pub const UNKNOWN_ALBUM: &str = crate::i18n::n_("Album inconnu");

/// « Artiste inconnu » dans la langue de l'interface (la bibliothèque est reconstruite quand elle change).
pub fn unknown_artist() -> &'static str {
    crate::i18n::tr(UNKNOWN_ARTIST)
}

pub fn unknown_album() -> &'static str {
    crate::i18n::tr(UNKNOWN_ALBUM)
}

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
    #[serde(default)]
    pub composer: String,
    /// Version de la lecture des tags : un morceau du cache lu par une version plus ancienne
    /// (sans le compositeur, par exemple) est relu.
    #[serde(default)]
    pub schema: u8,
    /// Morceau d'un fichier découpé par une feuille CUE : le vrai fichier, son début et sa fin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cue: Option<CueSpan>,
    /// Morceau d'un serveur de musique : où le lire, sa pochette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<Remote>,
    /// Note (1 à 5 étoiles) inscrite dans le fichier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueSpan {
    pub file: PathBuf,
    pub start_ms: u64,
    /// Fin du morceau (`None` : jusqu'à la fin du fichier).
    pub end_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Remote {
    /// Adresse du son.
    pub stream: String,
    /// Adresse de la pochette (vide s'il n'y en a pas).
    pub cover: String,
    /// Serveur d'où vient le morceau.
    pub server: String,
}

/// Ce qu'il faut ouvrir pour jouer un morceau.
pub fn media(t: &Track) -> crate::media::Media {
    if let Some(r) = &t.remote {
        return crate::media::Media::url(&r.stream, Some(t.duration()).filter(|d| !d.is_zero()));
    }
    if let Some(c) = &t.cue {
        let mut m = crate::media::Media::file(&c.file);
        m.start = Duration::from_millis(c.start_ms);
        m.end = c.end_ms.map(Duration::from_millis);
        m.duration = Some(t.duration());
        return m;
    }
    let mut m = crate::media::Media::file(&t.path);
    m.duration = Some(t.duration()).filter(|d| !d.is_zero());
    m
}

/// Pour un morceau d'un fichier CUE ou d'un serveur, son vrai fichier et l'adresse de sa
/// pochette (les pochettes et les paroles ne connaissent que le chemin du morceau).
#[derive(Clone, Debug, Default)]
pub struct Resolved {
    pub file: Option<PathBuf>,
    pub cover_url: Option<String>,
}

static RESOLVE: std::sync::RwLock<Option<HashMap<PathBuf, Resolved>>> = std::sync::RwLock::new(None);

/// Le vrai fichier d'un morceau (celui d'un morceau CUE, rien pour un morceau distant).
pub fn real_file(path: &Path) -> Option<PathBuf> {
    match RESOLVE.read().ok().and_then(|m| m.as_ref().and_then(|m| m.get(path).cloned())) {
        Some(r) => r.file,
        None => Some(path.to_path_buf()),
    }
}

/// Adresse de la pochette d'un morceau distant.
pub fn cover_url(path: &Path) -> Option<String> {
    RESOLVE
        .read()
        .ok()?
        .as_ref()?
        .get(path)
        .and_then(|r| r.cover_url.clone())
        .filter(|u| !u.is_empty())
}

/// Voir `Track::schema`.
pub const TRACK_SCHEMA: u8 = 2;

/// Genres d'un morceau : le champ peut en contenir plusieurs (« Rock; Pop »).
pub fn split_genres(genre: &str) -> impl Iterator<Item = &str> {
    genre.split([';', '\0']).map(str::trim).filter(|g| !g.is_empty())
}

impl Track {
    /// Vrai pour un vrai fichier de musique sur le disque (ni morceau CUE, ni morceau distant) :
    /// ses tags peuvent être modifiés.
    pub fn is_plain_file(&self) -> bool {
        self.cue.is_none() && self.remote.is_none()
    }

    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.duration_ms)
    }

    pub fn artist_or_unknown(&self) -> &str {
        if self.artist.is_empty() {
            unknown_artist()
        } else {
            &self.artist
        }
    }

    pub fn album_or_unknown(&self) -> &str {
        if self.album.is_empty() {
            unknown_album()
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

/// Minuscules sans accents, pour chercher « pokemon » et trouver « Pokémon ».
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => out.push('a'),
            'ç' | 'ć' | 'č' => out.push('c'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' => out.push('i'),
            'ñ' | 'ń' | 'ň' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => out.push('o'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'š' | 'ś' => out.push('s'),
            'ž' | 'ź' | 'ż' => out.push('z'),
            'ł' => out.push('l'),
            'ß' => out.push_str("ss"),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            '’' | '‘' => out.push('\''),
            c => out.push(c),
        }
    }
    out
}

fn words(query: &str) -> Vec<String> {
    fold(query).split_whitespace().map(str::to_owned).collect()
}

impl Library {
    pub fn new(mut tracks: Vec<Track>) -> Self {
        let special: HashMap<PathBuf, Resolved> = tracks
            .iter()
            .filter(|t| !t.is_plain_file())
            .map(|t| {
                (
                    t.path.clone(),
                    Resolved {
                        file: t.cue.as_ref().map(|c| c.file.clone()),
                        cover_url: t.remote.as_ref().map(|r| r.cover.clone()),
                    },
                )
            })
            .collect();
        if let Ok(mut r) = RESOLVE.write() {
            *r = Some(special);
        }
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
            let key = (lower(t.album_artist_or_artist()), lower(t.album_or_unknown()));
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
            (x.title == unknown_album())
                .cmp(&(y.title == unknown_album()))
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
            (x.name == unknown_artist())
                .cmp(&(y.name == unknown_artist()))
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
                fold(&format!(
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

    /// Le fichier du morceau `i` a été renommé (ses tags, donc sa place, ne changent pas).
    pub fn rename(&mut self, i: usize, path: PathBuf) {
        self.by_path.remove(&self.tracks[i].path);
        self.by_path.insert(path.clone(), i);
        self.tracks[i].path = path;
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Filtre de la bibliothèque : les éléments de `list` dont le texte contient tous les mots.
    pub fn filter_tracks(&self, list: &[usize], query: &str) -> Vec<usize> {
        let words = words(query);
        list.iter()
            .copied()
            .filter(|&i| words.iter().all(|w| self.search_text[i].contains(w.as_str())))
            .collect()
    }

    pub fn filter_albums(&self, query: &str) -> Vec<usize> {
        let words = words(query);
        (0..self.albums.len())
            .filter(|&i| {
                let a = &self.albums[i];
                let year = a.year.map(|y| y.to_string()).unwrap_or_default();
                let text = fold(&format!("{} {} {} {year}", a.title, a.artist, a.genre));
                words.iter().all(|w| text.contains(w.as_str()))
            })
            .collect()
    }

    pub fn filter_artists(&self, query: &str) -> Vec<usize> {
        let words = words(query);
        (0..self.artists.len())
            .filter(|&i| {
                let text = fold(&self.artists[i].name);
                words.iter().all(|w| text.contains(w.as_str()))
            })
            .collect()
    }

    /// Morceaux, albums et artistes qui contiennent tous les mots de la recherche.
    pub fn search(&self, query: &str) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
        let words = words(query);
        if words.is_empty() {
            return Default::default();
        }
        let matches = |text: &str| words.iter().all(|w| text.contains(w.as_str()));
        let tracks = (0..self.tracks.len())
            .filter(|&i| matches(&self.search_text[i]))
            .collect();
        let albums = (0..self.albums.len())
            .filter(|&i| {
                matches(&fold(&format!(
                    "{} {}",
                    self.albums[i].title, self.albums[i].artist
                )))
            })
            .collect();
        let artists = (0..self.artists.len())
            .filter(|&i| matches(&fold(&self.artists[i].name)))
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
        composer: String::new(),
        schema: TRACK_SCHEMA,
        cue: None,
        rating: None,
        remote: None,
    };
    // MP3 : la note (trame POPM) ne passe pas par le tag générique de lofty ; le fichier est lu
    // une seule fois, sous sa forme ID3v2, puis converti.
    let tagged = if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
    {
        std::fs::File::open(path)
            .ok()
            .and_then(|mut f| lofty::mpeg::MpegFile::read_from(&mut f, lofty::config::ParseOptions::new()).ok())
            .map(|mp3| {
                track.rating = mp3.id3v2().and_then(crate::tags::id3v2_rating);
                lofty::file::TaggedFile::from(mp3)
            })
            .ok_or(())
    } else {
        Probe::open(path).and_then(|p| p.read()).map_err(|_| ())
    };
    if let Ok(tagged) = tagged {
        if track.rating.is_none() {
            track.rating = tagged.tags().iter().find_map(crate::tags::tag_rating);
        }
        track.duration_ms = tagged.properties().duration().as_millis() as u64;
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(t) = tag.title().filter(|t| !t.trim().is_empty()) {
                track.title = t.trim().to_owned();
            }
            let get = |v: Option<std::borrow::Cow<str>>| v.map(|s| s.trim().to_owned()).unwrap_or_default();
            track.artist = get(tag.artist());
            track.album = get(tag.album());
            track.genre = get(tag.genre());
            track.album_artist = tag
                .get_string(&ItemKey::AlbumArtist)
                .map(|s| s.trim().to_owned())
                .unwrap_or_default();
            track.composer = tag
                .get_string(&ItemKey::Composer)
                .map(|s| s.trim().to_owned())
                .unwrap_or_default();
            track.year = tag.year();
            track.track_no = tag.track();
            track.disc_no = tag.disk();
        }
    } else if let Some(p) = crate::ffdecode::probe(path) {
        // Formats inconnus de lofty (WMA, DSD…) : ffprobe.
        if !p.title.is_empty() {
            track.title = p.title;
        }
        track.artist = p.artist;
        track.album = p.album;
        track.album_artist = p.album_artist;
        track.genre = p.genre;
        track.composer = p.composer;
        track.year = p.year;
        track.track_no = p.track;
        track.disc_no = p.disc;
        track.duration_ms = p.duration_ms;
    }
    track
}

// ---------- Feuilles CUE ----------

/// Les morceaux décrits par une feuille CUE (un album en un seul fichier).
pub fn read_cue(cue: &Path) -> Vec<Track> {
    let Ok(bytes) = std::fs::read(cue) else {
        return Vec::new();
    };
    // UTF-8 (avec ou sans BOM), sinon Latin-1 / Windows-1252 des vieux extracteurs.
    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => t.trim_start_matches('\u{feff}').to_owned(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    };
    parse_cue(&text, cue.parent().unwrap_or(Path::new(".")), mtime(cue))
}

fn parse_cue(text: &str, dir: &Path, mtime: u64) -> Vec<Track> {
    fn value(rest: &str) -> String {
        let rest = rest.trim();
        if let Some(r) = rest.strip_prefix('"') {
            r.rsplit_once('"').map(|(v, _)| v).unwrap_or(r).to_owned()
        } else {
            rest.to_owned()
        }
    }
    // « 03:25:40 » : minutes, secondes, images (1/75 s).
    fn time(s: &str) -> Option<u64> {
        let mut p = s.trim().split(':').map(|x| x.parse::<u64>().ok());
        let (m, sec, f) = (p.next()??, p.next()??, p.next()??);
        Some((m * 60 + sec) * 1000 + f * 1000 / 75)
    }
    struct Raw {
        file: PathBuf,
        no: u32,
        title: String,
        performer: String,
        composer: String,
        start: u64,
    }
    let (mut album, mut album_artist, mut genre, mut year) = (String::new(), String::new(), String::new(), None);
    let mut file: Option<PathBuf> = None;
    let mut raws: Vec<Raw> = Vec::new();
    let mut in_track = false;
    for line in text.lines() {
        let line = line.trim();
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        match word.to_ascii_uppercase().as_str() {
            "FILE" => {
                // « FILE "album.flac" WAVE » : le nom, sans le type à la fin.
                let r = rest.trim();
                let name = if r.starts_with('"') {
                    value(r)
                } else {
                    r.rsplit_once(' ').map(|(n, _)| n).unwrap_or(r).to_owned()
                };
                file = Some(dir.join(name));
                in_track = false;
            }
            "TRACK" => {
                let no = rest.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
                if let Some(f) = &file {
                    raws.push(Raw {
                        file: f.clone(),
                        no,
                        title: String::new(),
                        performer: String::new(),
                        composer: String::new(),
                        start: 0,
                    });
                    in_track = true;
                }
            }
            "TITLE" if in_track => raws.last_mut().unwrap().title = value(rest),
            "TITLE" => album = value(rest),
            "PERFORMER" if in_track => raws.last_mut().unwrap().performer = value(rest),
            "PERFORMER" => album_artist = value(rest),
            "SONGWRITER" if in_track => raws.last_mut().unwrap().composer = value(rest),
            "INDEX" if in_track => {
                let mut parts = rest.split_whitespace();
                if parts.next() == Some("01") {
                    if let Some(t) = parts.next().and_then(time) {
                        raws.last_mut().unwrap().start = t;
                    }
                }
            }
            "REM" => {
                let (key, v) = rest.split_once(' ').unwrap_or((rest, ""));
                match key.to_ascii_uppercase().as_str() {
                    "GENRE" => genre = value(v),
                    "DATE" => year = value(v).get(..4).and_then(|y| y.parse().ok()),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (i, r) in raws.iter().enumerate() {
        if !r.file.is_file() {
            continue;
        }
        // Fin : le début du morceau suivant du même fichier, sinon la fin du fichier.
        let end = raws.get(i + 1).filter(|n| n.file == r.file).map(|n| n.start);
        let file_ms = || {
            Probe::open(&r.file)
                .and_then(|p| p.read())
                .map(|t| t.properties().duration().as_millis() as u64)
                .ok()
                .filter(|&d| d > 0)
                .or_else(|| crate::ffdecode::probe(&r.file).map(|p| p.duration_ms))
                .unwrap_or(0)
        };
        let duration = end.unwrap_or_else(file_ms).saturating_sub(r.start);
        let performer = if r.performer.is_empty() {
            album_artist.clone()
        } else {
            r.performer.clone()
        };
        out.push(Track {
            path: PathBuf::from(format!("{}#{:02}", r.file.display(), r.no)),
            title: if r.title.is_empty() {
                format!("Track {:02}", r.no)
            } else {
                r.title.clone()
            },
            artist: performer,
            album: album.clone(),
            album_artist: album_artist.clone(),
            genre: genre.clone(),
            year,
            track_no: Some(r.no),
            disc_no: None,
            duration_ms: duration,
            mtime,
            composer: r.composer.clone(),
            schema: TRACK_SCHEMA,
            cue: Some(CueSpan {
                file: r.file.clone(),
                start_ms: r.start,
                end_ms: end,
            }),
            rating: None,
            remote: None,
        });
    }
    out
}

/// Vrai pour une feuille CUE.
pub fn is_cue_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cue"))
}

/// Parcourt les dossiers ; les fichiers inchangés depuis le dernier scan sont repris du cache.
/// Un album décrit par une feuille CUE donne ses morceaux, à la place du grand fichier.
pub fn scan(dirs: Vec<PathBuf>, cache: HashMap<PathBuf, Track>, tx: Sender<ScanMsg>) {
    let mut tracks = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut audio = Vec::new();
    for dir in &dirs {
        for entry in WalkDir::new(dir)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            if is_cue_file(path) {
                for t in read_cue(path) {
                    if let Some(c) = &t.cue {
                        seen.insert(c.file.clone());
                    }
                    if seen.insert(t.path.clone()) {
                        tracks.push(t);
                    }
                }
            } else if is_audio_file(path) {
                audio.push(path.to_path_buf());
            }
        }
    }
    {
        for path in &audio {
            let path = path.as_path();
            if !seen.insert(path.to_path_buf()) {
                continue;
            }
            let track = match cache.get(path) {
                Some(t) if t.mtime == mtime(path) && t.schema >= TRACK_SCHEMA => t.clone(),
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
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback))
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

pub fn save_cache(tracks: &[Track]) {
    let _ = std::fs::create_dir_all(data_dir());
    // Les morceaux des serveurs ont leur propre cache (voir `servers`).
    let local: Vec<&Track> = tracks.iter().filter(|t| t.remote.is_none()).collect();
    if let Ok(json) = serde_json::to_vec(&local) {
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
        .filter(|p| p.is_dir() && p != &PathBuf::from(std::env::var_os("HOME").unwrap_or_default()));
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

/// « 3 fichiers », « 1 fichier », dans la langue de l'interface (voir `i18n::plural`).
pub fn plural(n: usize, one: &str, many: &str) -> String {
    crate::i18n::plural(n, one, many)
}

#[cfg(test)]
mod cue_tests {
    use super::*;

    #[test]
    fn cue_sheet() {
        let dir = std::env::temp_dir().join(format!("stermusic-cue-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Album.flac"), b"pas vraiment du son").unwrap();
        let text = "REM GENRE Rock\nREM DATE 1997\nPERFORMER \"Groupe\"\nTITLE \"Le disque\"\n\
            FILE \"Album.flac\" WAVE\n  TRACK 01 AUDIO\n    TITLE \"Un\"\n    INDEX 01 00:00:00\n\
            \x20 TRACK 02 AUDIO\n    TITLE \"Deux\"\n    PERFORMER \"Invité\"\n    INDEX 00 03:58:00\n\
            \x20   INDEX 01 04:00:37\n";
        let t = parse_cue(text, &dir, 7);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].title.as_str(), t[0].artist.as_str()), ("Un", "Groupe"));
        assert_eq!((t[1].artist.as_str(), t[1].album_artist.as_str()), ("Invité", "Groupe"));
        assert_eq!(t[0].album, "Le disque");
        assert_eq!((t[0].genre.as_str(), t[0].year), ("Rock", Some(1997)));
        // 4:00 et 37 images (1/75 s) : 240 493 ms ; le premier morceau finit là.
        let c = t[1].cue.as_ref().unwrap();
        assert_eq!(c.start_ms, 240_493);
        assert_eq!(t[0].cue.as_ref().unwrap().end_ms, Some(240_493));
        assert_eq!(t[0].duration_ms, 240_493);
        assert!(t[1].path.to_string_lossy().ends_with("Album.flac#02"));
        assert!(!t[0].is_plain_file());
    }
}
