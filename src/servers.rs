//! Serveurs de musique : ceux qui parlent l'API Subsonic (Navidrome, Airsonic, Gonic,
//! Ampache…) et Jellyfin. Leurs morceaux entrent dans la bibliothèque comme des morceaux
//! « distants » : ils se lisent en flux (par ffmpeg), leurs pochettes viennent du serveur.
//!
//! La liste des morceaux est gardée sur le disque (`servers/<id>.json`) : la bibliothèque
//! s'affiche tout de suite au lancement, et « Actualiser » la redemande au serveur.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::library::{Remote, Track, TRACK_SCHEMA};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// Navidrome, Airsonic, Gonic, Ampache…
    #[default]
    Subsonic,
    Jellyfin,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Subsonic => "Navidrome / Subsonic",
            Kind::Jellyfin => "Jellyfin",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Server {
    /// Identifiant interne (il entre dans le chemin des morceaux).
    pub id: String,
    pub kind: Kind,
    /// Nom affiché (celui du serveur, ou son adresse).
    pub name: String,
    pub url: String,
    pub user: String,
    /// Subsonic : le mot de passe (chaque appel en tire un jeton salé).
    pub password: String,
    /// Jellyfin : le jeton d'accès et l'utilisateur, obtenus à la connexion.
    pub token: String,
    pub user_id: String,
    pub enabled: bool,
    /// Dernière actualisation (secondes) et nombre de morceaux trouvés.
    pub synced: u64,
    pub count: usize,
}

const CLIENT: &str = "SterMusic";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(concat!("SterMusic/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .build()
}

fn base(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    if u.starts_with("http://") || u.starts_with("https://") {
        u.to_owned()
    } else {
        format!("http://{u}")
    }
}

/// Message lisible d'une erreur de requête.
fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(401 | 403, _) => crate::i18n::tr("Identifiant ou mot de passe refusé").into(),
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(t) => t.to_string(),
    }
}

/// Vérifie l'accès et complète le serveur (nom, jeton Jellyfin). Bloquant.
pub fn connect(mut server: Server) -> Result<Server, String> {
    server.url = base(&server.url);
    if server.id.is_empty() {
        server.id = format!("{:08x}", fastrand::u32(..));
    }
    match server.kind {
        Kind::Subsonic => {
            let v = subsonic(&server, "ping", &[])?;
            let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("Subsonic");
            if server.name.is_empty() {
                server.name = format!("{} ({})", capitalize(kind), host(&server.url));
            }
        }
        Kind::Jellyfin => {
            let body = serde_json::json!({ "Username": server.user, "Pw": server.password });
            let v: serde_json::Value = agent()
                .post(&format!("{}/Users/AuthenticateByName", server.url))
                .set("Authorization", &jellyfin_auth(""))
                .set("Content-Type", "application/json")
                .send_string(&body.to_string())
                .map_err(describe)
                .and_then(|r| serde_json::from_reader(r.into_reader()).map_err(|e| e.to_string()))?;
            server.token = v["AccessToken"].as_str().unwrap_or_default().to_owned();
            server.user_id = v["User"]["Id"].as_str().unwrap_or_default().to_owned();
            if server.token.is_empty() {
                return Err(crate::i18n::tr("Réponse du serveur inattendue").into());
            }
            // Le jeton suffit désormais : le mot de passe n'est pas gardé.
            server.password.clear();
            if server.name.is_empty() {
                server.name = format!("Jellyfin ({})", host(&server.url));
            }
        }
    }
    server.enabled = true;
    Ok(server)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_owned()
}

/// Tous les morceaux du serveur. Bloquant (quelques secondes pour une grosse collection).
pub fn fetch(server: &Server) -> Result<Vec<Track>, String> {
    match server.kind {
        Kind::Subsonic => subsonic_tracks(server),
        Kind::Jellyfin => jellyfin_tracks(server),
    }
}

// ---------- Subsonic ----------

/// Paramètres d'authentification : jeton = MD5(mot de passe + sel).
fn subsonic_auth(server: &Server) -> String {
    let salt = format!("{:012x}", fastrand::u64(..) & 0xffff_ffff_ffff);
    let token = crate::scrobble::md5_hex(format!("{}{salt}", server.password).as_bytes());
    format!(
        "u={}&t={token}&s={salt}&v=1.16.1&c={CLIENT}&f=json",
        urlencode(&server.user)
    )
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn subsonic(server: &Server, method: &str, params: &[(&str, String)]) -> Result<serde_json::Value, String> {
    let mut url = format!("{}/rest/{method}?{}", server.url, subsonic_auth(server));
    for (k, v) in params {
        url.push_str(&format!("&{k}={}", urlencode(v)));
    }
    let v: serde_json::Value = agent()
        .get(&url)
        .call()
        .map_err(describe)
        .and_then(|r| serde_json::from_reader(r.into_reader()).map_err(|e| e.to_string()))?;
    let resp = v.get("subsonic-response").cloned().unwrap_or_default();
    if resp.get("status").and_then(|s| s.as_str()) != Some("ok") {
        let msg = resp["error"]["message"].as_str().unwrap_or("erreur du serveur");
        return Err(msg.to_owned());
    }
    Ok(resp)
}

fn subsonic_tracks(server: &Server) -> Result<Vec<Track>, String> {
    // La recherche vide rend tous les morceaux (Navidrome, Gonic…), par pages.
    let mut songs: Vec<serde_json::Value> = Vec::new();
    let page = 500;
    loop {
        let r = subsonic(
            server,
            "search3",
            &[
                ("query", String::new()),
                ("songCount", page.to_string()),
                ("songOffset", songs.len().to_string()),
                ("artistCount", "0".into()),
                ("albumCount", "0".into()),
            ],
        )?;
        let batch = r["searchResult3"]["song"].as_array().cloned().unwrap_or_default();
        let n = batch.len();
        songs.extend(batch);
        if n < page || songs.len() > 500_000 {
            break;
        }
    }
    // Serveurs qui refusent la recherche vide : album par album.
    if songs.is_empty() {
        let mut offset = 0;
        loop {
            let r = subsonic(
                server,
                "getAlbumList2",
                &[
                    ("type", "alphabeticalByName".into()),
                    ("size", "500".into()),
                    ("offset", offset.to_string()),
                ],
            )?;
            let albums = r["albumList2"]["album"].as_array().cloned().unwrap_or_default();
            for a in &albums {
                if let Some(id) = a["id"].as_str() {
                    let al = subsonic(server, "getAlbum", &[("id", id.to_owned())])?;
                    songs.extend(al["album"]["song"].as_array().cloned().unwrap_or_default());
                }
            }
            if albums.len() < 500 {
                break;
            }
            offset += 500;
        }
    }
    Ok(songs.iter().filter_map(|s| subsonic_track(server, s)).collect())
}

fn subsonic_track(server: &Server, s: &serde_json::Value) -> Option<Track> {
    let id = s["id"].as_str()?;
    let text = |k: &str| s[k].as_str().unwrap_or_default().trim().to_owned();
    let num = |k: &str| s[k].as_u64().map(|n| n as u32).filter(|&n| n > 0);
    let cover = s["coverArt"].as_str().map(|c| {
        format!(
            "{}/rest/getCoverArt?id={}&size=600&{}",
            server.url,
            urlencode(c),
            subsonic_auth(server)
        )
    });
    let album_artist = s["displayAlbumArtist"]
        .as_str()
        .or_else(|| s["albumArtists"][0]["name"].as_str())
        .unwrap_or_default()
        .to_owned();
    Some(Track {
        path: PathBuf::from(format!("remote:{}/{id}", server.id)),
        title: text("title"),
        artist: text("artist"),
        album: text("album"),
        album_artist,
        genre: text("genre"),
        year: num("year"),
        track_no: num("track"),
        disc_no: num("discNumber"),
        duration_ms: s["duration"].as_u64().unwrap_or(0) * 1000,
        mtime: s["created"].as_str().and_then(parse_iso).unwrap_or(0),
        composer: text("displayComposer"),
        schema: TRACK_SCHEMA,
        cue: None,
        rating: None,
        remote: Some(Remote {
            stream: format!(
                "{}/rest/stream?id={}&{}",
                server.url,
                urlencode(id),
                subsonic_auth(server)
            ),
            cover: cover.unwrap_or_default(),
            server: server.id.clone(),
        }),
    })
}

// ---------- Jellyfin ----------

fn jellyfin_auth(token: &str) -> String {
    let mut h = format!(
        "MediaBrowser Client=\"{CLIENT}\", Device=\"PC\", DeviceId=\"stermusic-{}\", Version=\"{}\"",
        whoami(),
        env!("CARGO_PKG_VERSION")
    );
    if !token.is_empty() {
        h.push_str(&format!(", Token=\"{token}\""));
    }
    h
}

/// Identifiant d'appareil stable (le nom de la machine), sans rien révéler de l'utilisateur.
fn whoami() -> u32 {
    let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
    host.bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
}

fn jellyfin_tracks(server: &Server) -> Result<Vec<Track>, String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    loop {
        let url = format!(
            "{}/Users/{}/Items?IncludeItemTypes=Audio&Recursive=true&StartIndex={start}&Limit=1000\
             &Fields=Genres,ProductionYear,AlbumArtist,ParentIndexNumber,IndexNumber,DateCreated",
            server.url, server.user_id
        );
        let v: serde_json::Value = agent()
            .get(&url)
            .set("Authorization", &jellyfin_auth(&server.token))
            .call()
            .map_err(describe)
            .and_then(|r| serde_json::from_reader(r.into_reader()).map_err(|e| e.to_string()))?;
        let items = v["Items"].as_array().cloned().unwrap_or_default();
        let n = items.len();
        out.extend(items.iter().filter_map(|i| jellyfin_track(server, i)));
        start += n;
        let total = v["TotalRecordCount"].as_u64().unwrap_or(0) as usize;
        if n == 0 || start >= total {
            break;
        }
    }
    Ok(out)
}

fn jellyfin_track(server: &Server, i: &serde_json::Value) -> Option<Track> {
    let id = i["Id"].as_str()?;
    let text = |k: &str| i[k].as_str().unwrap_or_default().trim().to_owned();
    let num = |k: &str| i[k].as_u64().map(|n| n as u32).filter(|&n| n > 0);
    let artist = i["Artists"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| text("AlbumArtist"));
    let genre = i["Genres"]
        .as_array()
        .map(|g| g.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("; "))
        .unwrap_or_default();
    // Pochette de l'album, sinon celle du morceau.
    let cover = match (i["AlbumId"].as_str(), i["AlbumPrimaryImageTag"].as_str()) {
        (Some(album), Some(_)) => Some(album.to_owned()),
        _ => i["ImageTags"]["Primary"].as_str().map(|_| id.to_owned()),
    }
    .map(|item| {
        format!(
            "{}/Items/{item}/Images/Primary?maxHeight=600&api_key={}",
            server.url, server.token
        )
    });
    Some(Track {
        path: PathBuf::from(format!("remote:{}/{id}", server.id)),
        title: text("Name"),
        artist,
        album: text("Album"),
        album_artist: text("AlbumArtist"),
        genre,
        year: num("ProductionYear"),
        track_no: num("IndexNumber"),
        disc_no: num("ParentIndexNumber"),
        duration_ms: i["RunTimeTicks"].as_u64().unwrap_or(0) / 10_000,
        mtime: i["DateCreated"].as_str().and_then(parse_iso).unwrap_or(0),
        composer: String::new(),
        schema: TRACK_SCHEMA,
        cue: None,
        rating: None,
        remote: Some(Remote {
            stream: format!(
                "{}/Audio/{id}/stream?static=true&api_key={}",
                server.url, server.token
            ),
            cover: cover.unwrap_or_default(),
            server: server.id.clone(),
        }),
    })
}

// ---------- Cache ----------

fn cache_file(id: &str) -> PathBuf {
    crate::library::data_dir()
        .join("servers")
        .join(format!("{id}.json"))
}

pub fn load_cache(id: &str) -> Vec<Track> {
    std::fs::read(cache_file(id))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_cache(id: &str, tracks: &[Track]) {
    let file = cache_file(id);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec(tracks) {
        let _ = std::fs::write(file, json);
    }
}

pub fn forget(id: &str) {
    let _ = std::fs::remove_file(cache_file(id));
}

/// « 2024-03-05T18:22:10Z » (ou avec fraction et fuseau) → secondes depuis 1970.
pub fn parse_iso(s: &str) -> Option<u64> {
    let (date, time) = s.split_once('T').unwrap_or((s, "00:00:00"));
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time
        .trim_end_matches('Z')
        .split(['+', 'Z'])
        .next()?
        .split(':')
        .map(|x| x.split('.').next().and_then(|x| x.parse::<i64>().ok()));
    let (hh, mm, ss) = (
        t.next().flatten().unwrap_or(0),
        t.next().flatten().unwrap_or(0),
        t.next().flatten().unwrap_or(0),
    );
    // Jours depuis 1970 (algorithme des dates civiles).
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2024-03-05T18:22:10.123Z"), Some(1_709_662_930));
        assert_eq!(parse_iso("2000-02-29"), Some(951_782_400));
    }

    #[test]
    fn subsonic_song() {
        let server = Server {
            id: "s1".into(),
            url: "http://nas:4533".into(),
            user: "moi".into(),
            password: "secret".into(),
            ..Default::default()
        };
        let song = serde_json::json!({
            "id": "abc", "title": "Lithium", "album": "Nevermind", "artist": "Nirvana",
            "track": 5, "year": 1991, "duration": 257, "coverArt": "al-9", "genre": "Grunge"
        });
        let t = subsonic_track(&server, &song).unwrap();
        assert_eq!(t.path, PathBuf::from("remote:s1/abc"));
        assert_eq!(
            (t.title.as_str(), t.duration_ms, t.track_no),
            ("Lithium", 257_000, Some(5))
        );
        let r = t.remote.unwrap();
        assert!(r
            .stream
            .starts_with("http://nas:4533/rest/stream?id=abc&u=moi&t="));
        assert!(r.cover.contains("getCoverArt?id=al-9"));
        assert!(!r.stream.contains("secret"));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Réseau : serveurs de démonstration publics. `cargo test demo_servers -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn demo_servers() {
        for (kind, url, user, pw) in [
            (Kind::Subsonic, "https://demo.navidrome.org", "demo", "demo"),
            (Kind::Jellyfin, "https://demo.jellyfin.org/stable", "demo", ""),
        ] {
            let s = Server {
                kind,
                url: url.into(),
                user: user.into(),
                password: pw.into(),
                ..Default::default()
            };
            let s = match connect(s) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{kind:?} : connexion impossible : {e}");
                    continue;
                }
            };
            let t = std::time::Instant::now();
            let tracks = fetch(&s).expect("morceaux");
            eprintln!("{} : {} morceaux en {:?}", s.name, tracks.len(), t.elapsed());
            let first = tracks
                .iter()
                .find(|t| t.duration_ms > 30_000)
                .expect("un morceau");
            eprintln!("  {} — {} ({})", first.artist, first.title, first.album);
            let m = crate::library::media(first);
            let src =
                crate::ffdecode::FfmpegSource::open(&m.ffmpeg_input(), Duration::ZERO, m.duration).unwrap();
            let n = src.take(44_100 * 2 * 2).count();
            eprintln!("  {n} échantillons lus");
            assert!(n > 44_100 * 2);
        }
    }
}
