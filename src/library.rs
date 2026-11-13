//! Library: folder scanning (with a cache), tags, grouping into albums and artists.

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

/// "Unknown artist" in the interface language (the library is rebuilt when it changes).
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
    /// Modification date of the file (seconds): used by the cache and by "Recently added".
    pub mtime: u64,
    #[serde(default)]
    pub composer: String,
    /// Version of the tag reading: a cache track read by an older version (without the composer,
    /// for example) is read again.
    #[serde(default)]
    pub schema: u8,
    /// Track of a file split by a CUE sheet: the real file, its start and its end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cue: Option<CueSpan>,
    /// Track of a music server: where to play it from, its album art.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<Remote>,
    /// Rating (1 to 5 stars) written in the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueSpan {
    pub file: PathBuf,
    pub start_ms: u64,
    /// End of the track (`None`: up to the end of the file).
    pub end_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Remote {
    /// Address of the audio.
    pub stream: String,
    /// Address of the album art (empty if there is none).
    pub cover: String,
    /// Server the track comes from.
    pub server: String,
}

/// What must be opened to play a track.
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

/// For a track of a CUE sheet or a server, its real file and the address of its album art
/// (album art and lyrics only know the track's path).
#[derive(Clone, Debug, Default)]
pub struct Resolved {
    pub file: Option<PathBuf>,
    pub cover_url: Option<String>,
}

static RESOLVE: std::sync::RwLock<Option<HashMap<PathBuf, Resolved>>> = std::sync::RwLock::new(None);

/// The real file of a track (that of a CUE track, nothing for a remote track).
pub fn real_file(path: &Path) -> Option<PathBuf> {
    match RESOLVE.read().ok().and_then(|m| m.as_ref().and_then(|m| m.get(path).cloned())) {
        Some(r) => r.file,
        None => Some(path.to_path_buf()),
    }
}

/// Address of the album art of a remote track.
pub fn cover_url(path: &Path) -> Option<String> {
    RESOLVE
        .read()
        .ok()?
        .as_ref()?
        .get(path)
        .and_then(|r| r.cover_url.clone())
        .filter(|u| !u.is_empty())
}

/// Tracks that must not take their folder's image as album art (see
/// [`without_folder_art`]).
static NO_FOLDER_ART: std::sync::RwLock<Option<std::collections::HashSet<PathBuf>>> =
    std::sync::RwLock::new(None);

/// True if the folder image (cover.jpg, Folder.jpg…) can serve as album art for this track.
pub fn folder_art_allowed(path: &Path) -> bool {
    !NO_FOLDER_ART
        .read()
        .ok()
        .is_some_and(|s| s.as_ref().is_some_and(|s| s.contains(path)))
}

/// In a folder mixing several albums, the folder image (often a Folder.jpg left by Windows
/// Media Player) is the album art of only one of them: a track without album art there used to
/// get another album's. It therefore only applies to the album taking up more than half of the
/// folder, if there is one; the other tracks are listed here.
fn without_folder_art(tracks: &[Track]) -> std::collections::HashSet<PathBuf> {
    let mut dirs: HashMap<&Path, Vec<&Track>> = HashMap::new();
    for t in tracks.iter().filter(|t| t.is_plain_file()) {
        if let Some(dir) = t.path.parent() {
            dirs.entry(dir).or_default().push(t);
        }
    }
    let mut denied = std::collections::HashSet::new();
    for list in dirs.values() {
        let mut albums: HashMap<String, usize> = HashMap::new();
        for t in list {
            *albums.entry(fold(t.album.trim())).or_default() += 1;
        }
        if albums.len() < 2 {
            continue;
        }
        let main = albums
            .iter()
            .filter(|(name, n)| !name.is_empty() && **n * 2 > list.len())
            .map(|(name, _)| name.clone())
            .next();
        for t in list {
            if main.as_deref() != Some(fold(t.album.trim()).as_str()) {
                denied.insert(t.path.clone());
            }
        }
    }
    denied
}

/// See `Track::schema`.
pub const TRACK_SCHEMA: u8 = 2;

/// Genres of a track: the field may contain several ("Rock; Pop").
pub fn split_genres(genre: &str) -> impl Iterator<Item = &str> {
    genre.split([';', '\0']).map(str::trim).filter(|g| !g.is_empty())
}

impl Track {
    /// True for a real music file on disk (neither a CUE track nor a remote track): its tags can
    /// be modified.
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

    /// Artist the album is filed under.
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
    /// Album of each track.
    pub track_album: Vec<usize>,
    /// Artist of each album.
    pub album_artist: Vec<usize>,
    search_text: Vec<String>,
    /// Everything one may want to filter in a list: like `search_text`, plus the genre, the year,
    /// the composer and the file name.
    filter_text: Vec<String>,
    by_path: HashMap<PathBuf, usize>,
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Lowercase without accents, to search for "pokemon" and find "Pokémon".
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
        if let Ok(mut s) = NO_FOLDER_ART.write() {
            *s = Some(without_folder_art(&tracks));
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

        // Albums: same album artist + same title.
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

        // Albums sorted by title; the references are renumbered.
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

        // (Album) artists.
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
            // Most recent albums first.
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
        let filter_text = tracks
            .iter()
            .map(|t| {
                let year = t.year.map(|y| y.to_string()).unwrap_or_default();
                let file = t.path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                fold(&format!(
                    "{} {} {} {} {} {year} {} {file}",
                    t.title, t.artist, t.album, t.album_artist, t.genre, t.composer
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
            filter_text,
            by_path,
        }
    }

    pub fn find(&self, path: &Path) -> Option<usize> {
        self.by_path.get(path).copied()
    }

    /// The file of track `i` was renamed (its tags, hence its place, don't change).
    pub fn rename(&mut self, i: usize, path: PathBuf) {
        self.by_path.remove(&self.tracks[i].path);
        self.by_path.insert(path.clone(), i);
        self.tracks[i].path = path;
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Library filter: the items of `list` whose text contains all the words. The tracks of `list`
    /// that contain all the words of `query`, anywhere: title, artist, album, genre, year, composer,
    /// file name.
    pub fn filter_tracks(&self, list: &[usize], query: &str) -> Vec<usize> {
        let words = words(query);
        list.iter()
            .copied()
            .filter(|&i| words.iter().all(|w| self.filter_text[i].contains(w.as_str())))
            .collect()
    }

    /// True if track `i` contains all the words (already folded by [`fold`]) of `query`.
    pub fn track_matches(&self, i: usize, folded_query: &str) -> bool {
        folded_query.split_whitespace().all(|w| self.filter_text[i].contains(w))
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

    /// Tracks, albums and artists that contain all the words of the search.
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
    /// Music folders that could not be reached: their tracks were kept from the cache.
    Offline(Vec<PathBuf>),
    Done(Vec<Track>),
}

/// What a music folder looks like right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirState {
    /// Missing or unreadable (disconnected disk, network share down…).
    Missing,
    /// Exists but is empty: often the bare mount point of a partition that isn't mounted.
    Empty,
    Present,
}

/// State of a music folder (to be called outside the interface thread: a network share that is
/// down can block).
pub fn dir_state(dir: &Path) -> DirState {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                DirState::Present
            } else {
                DirState::Empty
            }
        }
        Err(_) => DirState::Missing,
    }
}

/// True if a music folder counts as unreachable: missing, or empty while the library knew
/// tracks in it. Its tracks are then kept as they were, until it comes back.
pub fn dir_offline(state: DirState, had_tracks: bool) -> bool {
    match state {
        DirState::Missing => true,
        DirState::Empty => had_tracks,
        DirState::Present => false,
    }
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

/// Reads the tags of a file; without tags, the title is the file name.
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
    // MP3: the rating (POPM frame) doesn't go through lofty's generic tag; the file is read only
    // once, in its ID3v2 form, then converted.
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
        // Formats unknown to lofty (WMA, DSD…): ffprobe.
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

// ---------- CUE sheets ----------

/// The tracks described by a CUE sheet (an album in a single file).
pub fn read_cue(cue: &Path) -> Vec<Track> {
    let Ok(bytes) = std::fs::read(cue) else {
        return Vec::new();
    };
    // UTF-8 (with or without BOM), otherwise the Latin-1 / Windows-1252 of old rippers.
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
    // "03:25:40": minutes, seconds, frames (1/75 s).
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
                // "FILE "album.flac" WAVE": the name, without the type at the end.
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
        // End: the start of the next track of the same file, otherwise the end of the file.
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

/// True for a CUE sheet.
pub fn is_cue_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cue"))
}

/// Walks the folders; files unchanged since the last scan are taken from the cache. An album
/// described by a CUE sheet gives its tracks, instead of the big file.
pub fn scan(dirs: Vec<PathBuf>, cache: HashMap<PathBuf, Track>, tx: Sender<ScanMsg>) {
    let mut tracks = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut audio = Vec::new();
    let mut offline = Vec::new();
    for dir in &dirs {
        // Unreachable folder (partition not mounted, disk unplugged…): what the previous scans
        // found there stays in the library, and comes back to life with the folder.
        let cached: Vec<&Track> = cache
            .values()
            .filter(|t| t.remote.is_none() && t.path.starts_with(dir))
            .collect();
        if dir_offline(dir_state(dir), !cached.is_empty()) {
            offline.push(dir.clone());
            for t in cached {
                if seen.insert(t.path.clone()) {
                    tracks.push(t.clone());
                }
            }
            continue;
        }
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
    let _ = tx.send(ScanMsg::Offline(offline));
    let _ = tx.send(ScanMsg::Done(tracks));
}

// ---------- Disk cache ----------

/// `~/.local/share/stermusic` (or `$XDG_DATA_HOME/stermusic`).
pub fn data_dir() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share").join("stermusic")
}

/// `~/.cache/stermusic` (or `$XDG_CACHE_HOME/stermusic`).
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
    // Server tracks have their own cache (see `servers`).
    let local: Vec<&Track> = tracks.iter().filter(|t| t.remote.is_none()).collect();
    if let Ok(json) = serde_json::to_vec(&local) {
        let tmp = cache_file().with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, cache_file());
        }
    }
}

/// The user's "Music" folder (xdg-user-dirs), if it exists.
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

/// Formats a duration as `m:ss` (or `h:mm:ss`).
pub fn fmt_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Long, readable duration: "1 h 12 min", "42 min".
pub fn fmt_long(d: Duration) -> String {
    let m = d.as_secs().div_ceil(60);
    if m >= 60 {
        format!("{} h {:02} min", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

/// "3 files", "1 file", in the interface language (see `i18n::plural`).
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
        // 4:00 and 37 frames (1/75 s): 240,493 ms; the first track ends there.
        let c = t[1].cue.as_ref().unwrap();
        assert_eq!(c.start_ms, 240_493);
        assert_eq!(t[0].cue.as_ref().unwrap().end_ms, Some(240_493));
        assert_eq!(t[0].duration_ms, 240_493);
        assert!(t[1].path.to_string_lossy().ends_with("Album.flac#02"));
        assert!(!t[0].is_plain_file());
    }
}

#[cfg(test)]
mod folder_art_tests {
    use super::*;

    fn track(path: &str, album: &str) -> Track {
        Track {
            path: path.into(),
            title: String::new(),
            artist: String::new(),
            album: album.into(),
            album_artist: String::new(),
            genre: String::new(),
            year: None,
            track_no: None,
            disc_no: None,
            duration_ms: 0,
            mtime: 0,
            composer: String::new(),
            schema: TRACK_SCHEMA,
            cue: None,
            rating: None,
            remote: None,
        }
    }

    /// A folder's image only serves the album that occupies it: never the tracks of a catch-all
    /// folder, nor the few strays of an album folder.
    #[test]
    fn folder_art_only_for_the_album_of_the_folder() {
        let tracks = vec![
            // Single-album folder: every track gets it.
            track("/m/Album/01.mp3", "Album"),
            track("/m/Album/02.mp3", "album"),
            // Catch-all: nobody.
            track("/m/Vrac/a.mp3", "Un"),
            track("/m/Vrac/b.mp3", "Deux"),
            track("/m/Vrac/c.mp3", ""),
            // Album with a stray: the album yes, the stray no.
            track("/m/Live/1.mp3", "Live"),
            track("/m/Live/2.mp3", "Live"),
            track("/m/Live/3.mp3", "Live"),
            track("/m/Live/bonus.mp3", "Autre"),
        ];
        let denied = without_folder_art(&tracks);
        let no = |p: &str| denied.contains(Path::new(p));
        assert!(!no("/m/Album/01.mp3") && !no("/m/Album/02.mp3"));
        assert!(no("/m/Vrac/a.mp3") && no("/m/Vrac/b.mp3") && no("/m/Vrac/c.mp3"));
        assert!(!no("/m/Live/1.mp3") && no("/m/Live/bonus.mp3"));
    }
}

#[cfg(test)]
mod offline_tests {
    use super::*;

    /// An unmounted partition (missing folder, or bare empty mount point) keeps its tracks; an
    /// empty folder that never had any is simply empty.
    #[test]
    fn unreachable_folders() {
        let root = std::env::temp_dir().join(format!("stermusic-offline-{}", std::process::id()));
        let empty = root.join("empty");
        let full = root.join("full");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&full).unwrap();
        std::fs::write(full.join("a.mp3"), b"").unwrap();

        assert_eq!(dir_state(&root.join("missing")), DirState::Missing);
        assert_eq!(dir_state(&empty), DirState::Empty);
        assert_eq!(dir_state(&full), DirState::Present);

        assert!(dir_offline(DirState::Missing, false));
        assert!(dir_offline(DirState::Missing, true));
        assert!(dir_offline(DirState::Empty, true));
        assert!(!dir_offline(DirState::Empty, false));
        assert!(!dir_offline(DirState::Present, true));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
