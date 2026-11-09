//! Import depuis MusicBrainz (recherche d'albums, liste des pistes) et le Cover Art Archive
//! (pochettes). API publique, sans clé ; MusicBrainz demande au plus une requête par seconde
//! et un User-Agent qui nomme l'application.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

const API: &str = "https://musicbrainz.org/ws/2";
const USER_AGENT: &str = concat!(
    "SterMusic/",
    env!("CARGO_PKG_VERSION"),
    " (lecteur de musique pour Linux)"
);

#[derive(Clone, Debug)]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub date: String,
    pub country: String,
    pub tracks: u32,
    pub format: String,
    /// Précision (« Deluxe », « remaster »…) et type (album, single…).
    pub extra: String,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub disc: u32,
    pub discs: u32,
    pub position: u32,
    /// Nombre de pistes du disque.
    pub total: u32,
    pub title: String,
    pub artist: String,
    pub length_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub title: String,
    pub artist: String,
    pub year: String,
    pub genre: String,
    pub tracks: Vec<Track>,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
}

/// Une requête par seconde au plus, toutes recherches confondues.
fn throttle() {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap();
    if let Some(t) = *last {
        let wait = Duration::from_millis(1100).saturating_sub(t.elapsed());
        std::thread::sleep(wait);
    }
    *last = Some(Instant::now());
}

fn get_json(url: &str) -> Result<Value, String> {
    throttle();
    let resp = agent().get(url).call().map_err(describe)?;
    let body = resp.into_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(503, _) => {
            crate::i18n::tr("MusicBrainz est surchargé, réessaie dans un instant").into()
        }
        ureq::Error::Status(code, _) => crate::trf!("MusicBrainz a répondu {code}", code),
        ureq::Error::Transport(t) => crate::trf!("Pas de connexion à MusicBrainz ({0})", t.kind()),
    }
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Terme de recherche Lucene entre guillemets (les caractères spéciaux sont échappés).
fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.trim().chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Typographie simple : tirets et apostrophes ASCII, comme dans la plupart des bibliothèques.
fn plain(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' => '-',
            '\u{2019}' | '\u{2018}' => '\'',
            c => c,
        })
        .collect()
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).map(plain).unwrap_or_default()
}

fn credit(v: &Value) -> String {
    v.get("artist-credit")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|c| format!("{}{}", text(c, "name"), text(c, "joinphrase")))
                .collect::<String>()
        })
        .unwrap_or_default()
}

/// Albums correspondant à l'artiste et au titre (l'un des deux peut être vide).
pub fn search(artist: &str, album: &str) -> Result<Vec<Hit>, String> {
    let mut terms = Vec::new();
    if !album.trim().is_empty() {
        terms.push(format!("release:{}", quoted(album)));
    }
    if !artist.trim().is_empty() {
        terms.push(format!("artist:{}", quoted(artist)));
    }
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!(
        "{API}/release/?query={}&fmt=json&limit=25",
        encode(&terms.join(" AND "))
    );
    let json = get_json(&url)?;
    let hits = json
        .get("releases")
        .and_then(Value::as_array)
        .map(|r| {
            r.iter()
                .map(|r| {
                    let media: Vec<&Value> = r
                        .get("media")
                        .and_then(Value::as_array)
                        .map(|m| m.iter().collect())
                        .unwrap_or_default();
                    let mut formats: Vec<String> = media.iter().map(|m| text(m, "format")).collect();
                    formats.dedup();
                    let format = match (media.len(), formats.join(", ")) {
                        (n, f) if n > 1 => format!("{n} × {f}"),
                        (_, f) => f,
                    };
                    let kind = r
                        .get("release-group")
                        .map(|g| text(g, "primary-type"))
                        .unwrap_or_default();
                    let extra = [text(r, "disambiguation"), kind]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                    Hit {
                        id: text(r, "id"),
                        title: text(r, "title"),
                        artist: credit(r),
                        date: text(r, "date"),
                        country: text(r, "country"),
                        tracks: r.get("track-count").and_then(Value::as_u64).unwrap_or(0) as u32,
                        format,
                        extra,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(hits)
}

/// Le genre le plus voté (celui du groupe de sorties, sinon de la sortie), en « Title Case ».
fn genre(json: &Value) -> String {
    let best = |v: Option<&Value>| -> Option<String> {
        v?.get("genres")?
            .as_array()?
            .iter()
            .max_by_key(|g| g.get("count").and_then(Value::as_u64).unwrap_or(0))
            .map(|g| text(g, "name"))
    };
    let name = best(json.get("release-group"))
        .or_else(|| best(Some(json)))
        .unwrap_or_default();
    name.split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn release(id: &str) -> Result<Release, String> {
    let url = format!("{API}/release/{id}?inc=recordings+artist-credits+genres+release-groups&fmt=json");
    let json = get_json(&url)?;
    let media = json
        .get("media")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let discs = media.len() as u32;
    let mut tracks = Vec::new();
    for (d, m) in media.iter().enumerate() {
        let list = m
            .get("tracks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let total = list.len() as u32;
        for (i, t) in list.iter().enumerate() {
            tracks.push(Track {
                disc: m.get("position").and_then(Value::as_u64).unwrap_or(d as u64 + 1) as u32,
                discs,
                position: t.get("position").and_then(Value::as_u64).unwrap_or(i as u64 + 1) as u32,
                total,
                title: text(t, "title"),
                artist: credit(t),
                length_ms: t.get("length").and_then(Value::as_u64),
            });
        }
    }
    Ok(Release {
        title: text(&json, "title"),
        artist: credit(&json),
        year: text(&json, "date").chars().take(4).collect(),
        genre: genre(&json),
        tracks,
    })
}

/// Pochette avant (500 px) du Cover Art Archive ; `None` s'il n'y en a pas.
pub fn cover(id: &str) -> Result<Option<Vec<u8>>, String> {
    let url = format!("https://coverartarchive.org/release/{id}/front-500");
    match agent().get(&url).call() {
        Ok(resp) => {
            let mut data = Vec::new();
            use std::io::Read;
            resp.into_reader()
                .take(20 << 20)
                .read_to_end(&mut data)
                .map_err(|e| e.to_string())?;
            Ok(Some(data))
        }
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => Err(describe(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_encoding() {
        assert_eq!(quoted("A \"b\""), "\"A \\\"b\\\"\"");
        assert_eq!(encode("é a"), "%C3%A9%20a");
        assert_eq!(plain("alt‐J’s"), "alt-J's");
    }

    /// Requête réelle : `cargo test musicbrainz -- --ignored`.
    #[test]
    #[ignore]
    fn live_lookup() {
        let hits = search("Alt-J", "This Is All Yours").unwrap();
        assert!(!hits.is_empty());
        let r = release(&hits[0].id).unwrap();
        println!(
            "{} — {} ({}) {} : {} pistes",
            r.artist,
            r.title,
            r.year,
            r.genre,
            r.tracks.len()
        );
        assert!(r.tracks.len() >= 13);
        let c = cover(&hits[0].id).unwrap();
        println!("pochette : {:?} octets", c.map(|c| c.len()));
    }
}
