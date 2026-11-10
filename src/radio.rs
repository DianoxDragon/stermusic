//! Webradios : annuaire, connexion au flux, titre diffusé.
//!
//! L'annuaire est celui de radio-browser.info (libre, sans clé). Un flux est lu par un thread
//! qui remplit un tampon ; le décodeur (Symphonia, via rodio) y puise comme dans un fichier qu'on
//! ne peut pas rembobiner. Les serveurs Icecast et Shoutcast glissent le titre en cours dans le
//! flux (métadonnées ICY) : le thread les retire du son et les garde pour l'affichage.

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const USER_AGENT: &str = concat!("SterMusic/", env!("CARGO_PKG_VERSION"));

/// Serveurs de l'annuaire, essayés dans l'ordre.
const DIRECTORY: &[&str] = &[
    "https://de1.api.radio-browser.info",
    "https://de2.api.radio-browser.info",
    "https://fi1.api.radio-browser.info",
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Station {
    pub name: String,
    /// Adresse du flux (ou d'une playlist .pls / .m3u qui y mène).
    pub url: String,
    /// Logo de la station (adresse d'une image).
    pub favicon: String,
    pub country: String,
    /// Genres, séparés par des virgules (« jazz,lounge »).
    pub tags: String,
    pub codec: String,
    pub bitrate: u32,
    pub homepage: String,
    /// Identifiant dans l'annuaire (vide pour une station ajoutée à la main).
    pub uuid: String,
}

impl Station {
    /// « Jazz · Lounge · 128 kb/s » : de quoi reconnaître la station dans une liste.
    pub fn details(&self) -> String {
        let mut parts: Vec<String> = self
            .tags
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .take(3)
            .map(capitalize)
            .collect();
        if !self.country.is_empty() {
            parts.insert(0, self.country.clone());
        }
        if self.bitrate > 0 {
            parts.push(format!("{} kb/s", self.bitrate));
        }
        parts.join(" · ")
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

// ---------- Annuaire ----------

#[derive(Deserialize)]
struct ApiStation {
    #[serde(default)]
    stationuuid: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    url_resolved: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    favicon: String,
    #[serde(default)]
    tags: String,
    #[serde(default)]
    country: String,
    #[serde(default)]
    codec: String,
    #[serde(default)]
    bitrate: u32,
    #[serde(default)]
    hls: u8,
}

/// Formats que le décodeur sait lire (Opus et HLS ne le sont pas).
fn playable(codec: &str, hls: bool) -> bool {
    !hls && matches!(
        codec.to_ascii_uppercase().as_str(),
        "MP3" | "AAC" | "AAC+" | "OGG" | "FLAC" | "UNKNOWN" | ""
    )
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(USER_AGENT)
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(timeout)
        .build()
}

/// Ce qu'on demande à l'annuaire.
#[derive(Clone, Debug, PartialEq)]
pub enum Query {
    /// Les stations les plus écoutées d'un pays (code ISO : « FR »).
    Top(String),
    /// Recherche par nom.
    Name(String),
    /// Recherche par genre.
    Tag(String),
}

/// Interroge l'annuaire (bloquant : à appeler depuis un thread).
pub fn search(query: &Query) -> Result<Vec<Station>, String> {
    let (key, value) = match query {
        Query::Top(country) => ("countrycode", country.as_str()),
        Query::Name(name) => ("name", name.as_str()),
        Query::Tag(tag) => ("tag", tag.as_str()),
    };
    let agent = agent(Duration::from_secs(12));
    let mut last_error = String::new();
    for server in DIRECTORY {
        let result = agent
            .get(&format!("{server}/json/stations/search"))
            .query(key, value)
            .query("limit", "80")
            .query("hidebroken", "true")
            .query("order", "clickcount")
            .query("reverse", "true")
            .call();
        match result {
            Ok(resp) => {
                let list: Vec<ApiStation> =
                    serde_json::from_reader(resp.into_reader()).map_err(|e| e.to_string())?;
                return Ok(list
                    .into_iter()
                    .filter(|s| playable(&s.codec, s.hls != 0))
                    .map(|s| Station {
                        name: s.name.trim().to_owned(),
                        url: if s.url_resolved.is_empty() {
                            s.url
                        } else {
                            s.url_resolved
                        },
                        favicon: s.favicon,
                        country: s.country,
                        tags: s.tags,
                        codec: s.codec,
                        bitrate: s.bitrate,
                        homepage: s.homepage,
                        uuid: s.stationuuid,
                    })
                    .filter(|s| !s.name.is_empty() && !s.url.is_empty())
                    .collect());
            }
            Err(e) => last_error = e.to_string(),
        }
    }
    Err(last_error)
}

/// Signale à l'annuaire qu'une station a été écoutée (son classement en dépend).
fn count_click(uuid: &str) {
    if uuid.is_empty() {
        return;
    }
    let agent = agent(Duration::from_secs(5));
    for server in DIRECTORY {
        if agent.get(&format!("{server}/json/url/{uuid}")).call().is_ok() {
            return;
        }
    }
}

/// Code pays de la langue du bureau (`fr_FR.UTF-8` → `FR`), pour les stations proposées.
pub fn desktop_country() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .and_then(|l| {
            let region = l.split(['.', '@']).next()?.split('_').nth(1)?;
            (region.len() == 2).then(|| region.to_ascii_uppercase())
        })
        .unwrap_or_else(|| "FR".into())
}

/// Télécharge le logo d'une station (petite image : 512 Ko au plus).
pub fn fetch_image(url: &str) -> Option<Vec<u8>> {
    if !url.starts_with("http") {
        return None;
    }
    let resp = agent(Duration::from_secs(8)).get(url).call().ok()?;
    let mut bytes = Vec::new();
    resp.into_reader().take(512 * 1024).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

// ---------- Flux ----------

/// Tampon partagé entre le thread réseau (qui écrit) et le décodeur (qui lit).
struct Pipe {
    buf: Mutex<Buffer>,
    cond: Condvar,
}

#[derive(Default)]
struct Buffer {
    data: VecDeque<u8>,
    /// Le flux est terminé (coupure, serveur arrêté) : plus rien n'arrivera.
    ended: bool,
}

/// Au-delà, le thread réseau attend que le décodeur consomme.
const MAX_BUFFER: usize = 2 * 1024 * 1024;
/// Le décodeur attend au plus ce temps des données avant de tenir le flux pour terminé.
const STALL_TIMEOUT: Duration = Duration::from_secs(12);

/// Ce que le décodeur lit : le son du flux, sans les métadonnées ICY.
pub struct StreamReader {
    pipe: Arc<Pipe>,
    pos: u64,
    stop: Arc<AtomicBool>,
}

impl Read for StreamReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let mut buf = self.pipe.buf.lock().unwrap();
        let deadline = Instant::now() + STALL_TIMEOUT;
        while buf.data.is_empty() && !buf.ended {
            let now = Instant::now();
            if now >= deadline || self.stop.load(Ordering::Relaxed) {
                return Ok(0);
            }
            buf = self.pipe.cond.wait_timeout(buf, deadline - now).unwrap().0;
        }
        let n = out.len().min(buf.data.len());
        for (o, b) in out.iter_mut().zip(buf.data.drain(..n)) {
            *o = b;
        }
        drop(buf);
        self.pipe.cond.notify_all();
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for StreamReader {
    /// Un direct ne se rembobine pas : seule la position courante peut être demandée.
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match pos {
            SeekFrom::Current(0) => Ok(self.pos),
            SeekFrom::Start(p) if p == self.pos => Ok(self.pos),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "flux en direct",
            )),
        }
    }
}

impl Drop for StreamReader {
    /// Décodeur arrêté (pause, autre station) : le thread réseau se termine.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.pipe.cond.notify_all();
    }
}

/// Ce que l'interface suit d'une station en cours d'écoute.
#[derive(Clone)]
pub struct LiveInfo {
    /// Titre diffusé (« Artiste - Titre »), mis à jour par le thread réseau.
    pub title: Arc<Mutex<String>>,
    /// Nom donné par le serveur (en-tête `icy-name`), à défaut du nom de l'annuaire.
    pub server_name: String,
    pub stop: Arc<AtomicBool>,
}

impl LiveInfo {
    pub fn title(&self) -> String {
        self.title.lock().map(|t| t.clone()).unwrap_or_default()
    }
}

/// Une station prête à jouer : le décodeur et ce que l'interface en suit.
pub type Connected = (rodio::Decoder<StreamReader>, LiveInfo);

/// Se connecte au flux et prépare le décodeur (bloquant : quelques secondes au plus).
pub fn connect(station: &Station) -> Result<Connected, String> {
    let url = resolve(&station.url)?;
    let agent = ureq::AgentBuilder::new()
        .user_agent(USER_AGENT)
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(15))
        .build();
    let resp = agent
        .get(&url)
        .set("Icy-MetaData", "1")
        .call()
        .map_err(|e| crate::trf!("Connexion impossible : {e}", e))?;
    let content_type = resp.header("content-type").unwrap_or("").to_ascii_lowercase();
    if content_type.contains("mpegurl") && url.contains(".m3u8") {
        return Err(crate::i18n::tr("Ce flux (HLS) n'est pas pris en charge").into());
    }
    let metaint: usize = resp
        .header("icy-metaint")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let server_name = resp.header("icy-name").unwrap_or("").trim().to_owned();
    let hint = codec_hint(&content_type, &station.codec, &url);

    let pipe = Arc::new(Pipe {
        buf: Mutex::new(Buffer::default()),
        cond: Condvar::new(),
    });
    let stop = Arc::new(AtomicBool::new(false));
    let title = Arc::new(Mutex::new(String::new()));
    {
        let (pipe, stop, title) = (pipe.clone(), stop.clone(), title.clone());
        let reader = resp.into_reader();
        std::thread::spawn(move || pump(reader, metaint, &pipe, &stop, &title));
    }
    let uuid = station.uuid.clone();
    std::thread::spawn(move || count_click(&uuid));

    // Un peu d'avance avant de lancer le son : le réseau peut hoqueter.
    let start = Instant::now();
    loop {
        let (len, ended) = {
            let b = pipe.buf.lock().unwrap();
            (b.data.len(), b.ended)
        };
        if len >= 48 * 1024 || (ended && len > 0) {
            break;
        }
        if ended || start.elapsed() > Duration::from_secs(10) {
            stop.store(true, Ordering::Relaxed);
            return Err(crate::i18n::tr("La station ne répond pas").into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let reader = StreamReader {
        pipe,
        pos: 0,
        stop: stop.clone(),
    };
    let mut builder = rodio::Decoder::builder()
        .with_data(reader)
        .with_seekable(false)
        .with_gapless(false);
    if let Some(h) = hint {
        builder = builder.with_hint(h);
    }
    if !content_type.is_empty() {
        builder = builder.with_mime_type(content_type.split(';').next().unwrap_or("").trim());
    }
    let decoder = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| builder.build()))
        .map_err(|_| crate::i18n::tr("Format non pris en charge (le décodeur a échoué)").to_owned())?
        .map_err(|e| crate::trf!("Format non lisible : {e}", e))?;
    Ok((
        decoder,
        LiveInfo {
            title,
            server_name,
            stop,
        },
    ))
}

/// Extension qui aide le décodeur à reconnaître le format.
fn codec_hint(content_type: &str, codec: &str, url: &str) -> Option<&'static str> {
    let codec = codec.to_ascii_lowercase();
    if content_type.contains("mpeg") || codec == "mp3" {
        Some("mp3")
    } else if content_type.contains("aac") || codec.starts_with("aac") {
        Some("aac")
    } else if content_type.contains("ogg") || codec == "ogg" {
        Some("ogg")
    } else if content_type.contains("flac") || codec == "flac" {
        Some("flac")
    } else {
        let ext = url.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        ["mp3", "aac", "ogg", "flac"]
            .into_iter()
            .find(|e| ext.starts_with(e))
    }
}

/// Thread réseau : copie le flux dans le tampon, en retirant les métadonnées ICY (un bloc tous
/// les `metaint` octets de son).
fn pump(mut reader: impl Read, metaint: usize, pipe: &Pipe, stop: &AtomicBool, title: &Mutex<String>) {
    let mut chunk = vec![0u8; 16 * 1024];
    let mut until_meta = metaint;
    'outer: while !stop.load(Ordering::Relaxed) {
        let want = if metaint > 0 {
            chunk.len().min(until_meta)
        } else {
            chunk.len()
        };
        let n = match reader.read(&mut chunk[..want]) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        {
            let mut buf = pipe.buf.lock().unwrap();
            while buf.data.len() > MAX_BUFFER {
                if stop.load(Ordering::Relaxed) {
                    break 'outer;
                }
                buf = pipe.cond.wait_timeout(buf, Duration::from_millis(200)).unwrap().0;
            }
            buf.data.extend(&chunk[..n]);
        }
        pipe.cond.notify_all();
        if metaint == 0 {
            continue;
        }
        until_meta -= n;
        if until_meta == 0 {
            until_meta = metaint;
            let mut len = [0u8; 1];
            if reader.read_exact(&mut len).is_err() {
                break;
            }
            let size = len[0] as usize * 16;
            if size > 0 {
                let mut meta = vec![0u8; size];
                if reader.read_exact(&mut meta).is_err() {
                    break;
                }
                if let Some(t) = stream_title(&meta) {
                    if let Ok(mut current) = title.lock() {
                        *current = t;
                    }
                }
            }
        }
    }
    pipe.buf.lock().unwrap().ended = true;
    pipe.cond.notify_all();
}

/// Titre en cours dans un bloc ICY : `StreamTitle='Artiste - Titre';StreamUrl='…';`.
fn stream_title(meta: &[u8]) -> Option<String> {
    let end = meta.iter().position(|&b| b == 0).unwrap_or(meta.len());
    let raw = &meta[..end];
    // UTF-8 le plus souvent ; les vieux serveurs envoient du Latin-1.
    let text = match std::str::from_utf8(raw) {
        Ok(s) => s.to_owned(),
        Err(_) => raw.iter().map(|&b| b as char).collect(),
    };
    let start = text.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &text[start..];
    let stop = rest
        .find("';")
        .unwrap_or(rest.len().saturating_sub(rest.ends_with('\'') as usize));
    let title = rest[..stop].trim();
    // Certaines stations n'envoient qu'un tiret ou des espaces entre deux titres.
    let meaningful = title.chars().any(char::is_alphanumeric);
    Some(if meaningful {
        title.to_owned()
    } else {
        String::new()
    })
}

/// Une adresse de playlist (.pls, .m3u) mène au vrai flux : on prend sa première entrée.
fn resolve(url: &str) -> Result<String, String> {
    let lower = url.to_ascii_lowercase();
    let path = lower.split('?').next().unwrap_or("");
    let playlist = path.ends_with(".pls") || path.ends_with(".m3u") || path.ends_with(".asx");
    if !playlist {
        return Ok(url.to_owned());
    }
    let text = agent(Duration::from_secs(10))
        .get(url)
        .call()
        .map_err(|e| crate::trf!("Connexion impossible : {e}", e))?
        .into_string()
        .map_err(|e| e.to_string())?;
    playlist_entry(&text).ok_or_else(|| crate::i18n::tr("Playlist de la station illisible").into())
}

fn playlist_entry(text: &str) -> Option<String> {
    text.lines().map(str::trim).find_map(|l| {
        // .pls : « File1=http://… » ; .m3u : une adresse par ligne ; .asx : href="…".
        let candidate = if let Some((key, value)) = l.split_once('=') {
            if key.to_ascii_lowercase().starts_with("file") {
                value.trim()
            } else if let Some(i) = l.to_ascii_lowercase().find("href=\"") {
                l[i + 6..].split('"').next().unwrap_or("")
            } else {
                ""
            }
        } else {
            l
        };
        candidate.starts_with("http").then(|| candidate.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icy_title() {
        let meta = b"StreamTitle='Daft Punk - One More Time';StreamUrl='';\0\0\0";
        assert_eq!(stream_title(meta).as_deref(), Some("Daft Punk - One More Time"));
        assert_eq!(stream_title(b"StreamTitle=' - ';").as_deref(), Some(""));
        // Latin-1 : « é » sur un octet.
        let latin = b"StreamTitle='Caf\xe9';";
        assert_eq!(stream_title(latin).as_deref(), Some("Café"));
        assert_eq!(stream_title(b"rien"), None);
    }

    #[test]
    fn playlists() {
        let pls = "[playlist]\nNumberOfEntries=1\nFile1=http://stream.example/live.mp3\nTitle1=Radio";
        assert_eq!(
            playlist_entry(pls).as_deref(),
            Some("http://stream.example/live.mp3")
        );
        let m3u = "#EXTM3U\n#EXTINF:-1,Radio\nhttps://stream.example/aac\n";
        assert_eq!(playlist_entry(m3u).as_deref(), Some("https://stream.example/aac"));
        let asx = "<asx><entry><ref href=\"http://s.example/x\"/></entry></asx>";
        assert_eq!(playlist_entry(asx).as_deref(), Some("http://s.example/x"));
    }

    #[test]
    fn stripping_metadata() {
        // Deux blocs de 4 octets de son, séparés par un bloc de métadonnées de 16 octets.
        let mut stream = b"abcd".to_vec();
        let mut meta = b"StreamTitle='X';".to_vec();
        meta.resize(16, 0);
        stream.push(1);
        stream.extend(&meta);
        stream.extend(b"efgh");
        stream.push(0);
        let pipe = Pipe {
            buf: Mutex::new(Buffer::default()),
            cond: Condvar::new(),
        };
        let title = Mutex::new(String::new());
        pump(&stream[..], 4, &pipe, &AtomicBool::new(false), &title);
        let buf = pipe.buf.lock().unwrap();
        assert_eq!(buf.data.iter().copied().collect::<Vec<u8>>(), b"abcdefgh");
        assert!(buf.ended);
        assert_eq!(*title.lock().unwrap(), "X");
    }

    /// Réseau : annuaire, puis quelques secondes de son d'une vraie station (MP3 et AAC).
    #[test]
    #[ignore]
    fn live_radio() {
        use rodio::Source;
        for tag in ["jazz", "aac"] {
            let list = search(&Query::Tag(tag.into())).expect("annuaire");
            assert!(!list.is_empty());
            let mut ok = false;
            for s in list.iter().take(6) {
                match connect(s) {
                    Ok((decoder, info)) => {
                        let rate = decoder.sample_rate();
                        let samples = decoder.take(rate as usize * 2 * 3).count();
                        eprintln!(
                            "{} ({}) : {samples} échantillons, titre « {} »",
                            s.name,
                            s.codec,
                            info.title()
                        );
                        info.stop.store(true, Ordering::Relaxed);
                        if samples > rate as usize {
                            ok = true;
                            break;
                        }
                    }
                    Err(e) => eprintln!("{} ({}) : {e}", s.name, s.codec),
                }
            }
            assert!(ok, "aucune station {tag} lisible");
        }
    }

    #[test]
    fn country() {
        assert!(desktop_country().len() == 2);
    }
}
