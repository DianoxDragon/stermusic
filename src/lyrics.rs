//! Paroles des morceaux, synchronisées si possible (format LRC : « [01:23.45] une ligne »).
//!
//! Dans l'ordre : un fichier `.lrc` posé à côté du morceau, les paroles intégrées aux tags, puis
//! (si c'est permis) LRCLIB, une base de paroles libre et gratuite (https://lrclib.net). Les
//! réponses de LRCLIB sont gardées dans `~/.cache/stermusic/lyrics`, « rien trouvé » compris :
//! un morceau n'est demandé qu'une fois.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lofty::prelude::*;
use serde::{Deserialize, Serialize};

use crate::library::Track;

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Début de la ligne, en secondes ; `None` pour des paroles non synchronisées.
    pub time: Option<f32>,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    File,
    Tags,
    Online,
}

#[derive(Clone, Debug)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    pub synced: bool,
    pub source: Source,
}

impl Lyrics {
    /// Ligne en cours à `pos` secondes (légèrement en avance : on lit la ligne au moment où elle
    /// commence à être chantée).
    pub fn current(&self, pos: f32) -> Option<usize> {
        if !self.synced {
            return None;
        }
        let pos = pos + 0.25;
        let i = self.lines.partition_point(|l| l.time.is_some_and(|t| t <= pos));
        i.checked_sub(1)
    }
}

/// « 01:23.45 », « 1:23 », « 01:23:450 » → secondes.
fn timestamp(text: &str) -> Option<f32> {
    let (min, rest) = text.split_once(':')?;
    let min: f32 = min.trim().parse().ok()?;
    // Certains fichiers séparent les centièmes par « : » au lieu de « . ».
    let rest = match rest.split_once(':') {
        Some((s, frac)) => format!("{s}.{frac}"),
        None => rest.to_owned(),
    };
    let sec: f32 = rest.trim().parse().ok()?;
    (min >= 0.0 && sec >= 0.0).then_some(min * 60.0 + sec)
}

/// Lit des paroles au format LRC ; sans horodatage, ce sont des paroles simples.
pub fn parse(text: &str, source: Source) -> Option<Lyrics> {
    let mut lines: Vec<Line> = Vec::new();
    let mut plain: Vec<Line> = Vec::new();
    let mut offset = 0.0;
    for raw in text.lines() {
        let raw = raw.trim_end_matches('\r');
        let mut rest = raw.trim_start();
        let mut times = Vec::new();
        while let Some(inner) = rest.strip_prefix('[') {
            let Some(end) = inner.find(']') else { break };
            let tag = &inner[..end];
            match timestamp(tag) {
                Some(t) => times.push(t),
                None => {
                    // Métadonnées : [ar:…], [ti:…], [offset:+250]…
                    if let Some(ms) = tag.strip_prefix("offset:") {
                        offset = ms.trim().parse::<f32>().unwrap_or(0.0) / 1000.0;
                    }
                }
            }
            rest = &inner[end + 1..];
        }
        // Format « enrichi » : un horodatage par mot (<01:02.03>), retiré.
        let mut text = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '<' {
                let tag: String = chars.by_ref().take_while(|&c| c != '>').collect();
                if timestamp(&tag).is_some() {
                    continue;
                }
                text.push('<');
                text.push_str(&tag);
                text.push('>');
            } else {
                text.push(c);
            }
        }
        let text = text.trim().to_owned();
        if times.is_empty() {
            if !raw.trim_start().starts_with('[') {
                plain.push(Line { time: None, text });
            }
        } else {
            for t in times {
                lines.push(Line {
                    time: Some((t - offset).max(0.0)),
                    text: text.clone(),
                });
            }
        }
    }
    if !lines.is_empty() {
        lines.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
        return Some(Lyrics {
            lines,
            synced: true,
            source,
        });
    }
    // Paroles simples : les lignes vides du début et de la fin ne servent à rien.
    while plain.first().is_some_and(|l| l.text.is_empty()) {
        plain.remove(0);
    }
    while plain.last().is_some_and(|l| l.text.is_empty()) {
        plain.pop();
    }
    (!plain.is_empty()).then_some(Lyrics {
        lines: plain,
        synced: false,
        source,
    })
}

/// Paroles d'un fichier `.lrc` à côté du morceau (même nom).
fn from_file(path: &Path) -> Option<Lyrics> {
    let lrc = ["lrc", "LRC"]
        .iter()
        .map(|e| path.with_extension(e))
        .find(|p| p.is_file())?;
    let bytes = std::fs::read(lrc).ok()?;
    // Les vieux fichiers .lrc sont souvent en Latin-1 / Windows-1252.
    let text =
        String::from_utf8(bytes.clone()).unwrap_or_else(|_| bytes.iter().map(|&b| b as char).collect());
    parse(text.trim_start_matches('\u{feff}'), Source::File)
}

/// Paroles intégrées aux tags (USLT en ID3, LYRICS en Vorbis, ©lyr en MP4).
fn from_tags(path: &Path) -> Option<Lyrics> {
    let tagged = lofty::probe::Probe::open(path).ok()?.read().ok()?;
    tagged
        .tags()
        .iter()
        .find_map(|t| t.get_string(&ItemKey::Lyrics).map(str::to_owned))
        .and_then(|text| parse(&text, Source::Tags))
}

/// Réponse de LRCLIB gardée sur le disque.
#[derive(Serialize, Deserialize, Default)]
struct Cached {
    synced: Option<String>,
    plain: Option<String>,
    instrumental: bool,
}

fn cache_file(t: &Track) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&t.artist, &t.title, &t.album, t.duration_ms / 2000).hash(&mut h);
    crate::library::cache_dir()
        .join("lyrics")
        .join(format!("{:016x}.json", h.finish()))
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

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(concat!(
            "SterMusic/",
            env!("CARGO_PKG_VERSION"),
            " (lecteur de musique pour Linux)"
        ))
        .timeout(Duration::from_secs(15))
        .build()
}

#[derive(Deserialize)]
struct Answer {
    #[serde(rename = "syncedLyrics")]
    synced: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain: Option<String>,
    #[serde(default)]
    instrumental: bool,
    #[serde(default)]
    duration: f64,
}

/// Demande les paroles à LRCLIB : la correspondance exacte (titre, artiste, album, durée), sinon
/// une recherche dont on garde le résultat à la durée la plus proche. `Err` : pas de réseau.
fn ask_lrclib(t: &Track) -> Result<Cached, String> {
    let artist = if t.artist.is_empty() {
        &t.album_artist
    } else {
        &t.artist
    };
    if t.title.trim().is_empty() || artist.trim().is_empty() {
        return Ok(Cached::default());
    }
    let secs = (t.duration_ms as f64 / 1000.0).round();
    let get = format!(
        "https://lrclib.net/api/get?track_name={}&artist_name={}&album_name={}&duration={secs}",
        encode(&t.title),
        encode(artist),
        encode(&t.album),
    );
    let found = |a: Answer| Cached {
        synced: a.synced.filter(|s| !s.trim().is_empty()),
        plain: a.plain.filter(|s| !s.trim().is_empty()),
        instrumental: a.instrumental,
    };
    match agent().get(&get).call() {
        Ok(resp) => {
            if let Some(a) = resp
                .into_string()
                .ok()
                .and_then(|b| serde_json::from_str::<Answer>(&b).ok())
            {
                return Ok(found(a));
            }
        }
        Err(ureq::Error::Status(404, _)) => {}
        Err(ureq::Error::Status(code, _)) => return Err(format!("LRCLIB {code}")),
        Err(e) => return Err(e.to_string()),
    }
    let search = format!(
        "https://lrclib.net/api/search?track_name={}&artist_name={}",
        encode(&t.title),
        encode(artist)
    );
    let body = agent()
        .get(&search)
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let list: Vec<Answer> = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    // Une version trop différente en durée (live, remix) aurait des paroles décalées.
    let best = list
        .into_iter()
        .filter(|a| secs <= 0.0 || (a.duration - secs).abs() <= 4.0)
        .min_by(|a, b| {
            let score = |x: &Answer| {
                (
                    x.synced.is_none() as u8,
                    ((x.duration - secs).abs() * 10.0) as i64,
                )
            };
            score(a).cmp(&score(b))
        });
    Ok(best.map(found).unwrap_or_default())
}

fn from_cached(c: &Cached) -> Option<Lyrics> {
    c.synced
        .as_deref()
        .and_then(|s| parse(s, Source::Online))
        .or_else(|| c.plain.as_deref().and_then(|s| parse(s, Source::Online)))
}

/// Cherche les paroles d'un morceau (à appeler hors du fil de l'interface).
pub fn find(t: &Track, online: bool) -> Option<Lyrics> {
    if let Some(l) = from_file(&t.path).or_else(|| from_tags(&t.path)) {
        return Some(l);
    }
    if !online {
        return None;
    }
    let file = cache_file(t);
    if let Some(c) = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Cached>(&b).ok())
    {
        return from_cached(&c);
    }
    match ask_lrclib(t) {
        Ok(c) => {
            let _ = std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")));
            if let Ok(json) = serde_json::to_vec(&c) {
                let _ = std::fs::write(&file, json);
            }
            from_cached(&c)
        }
        // Pas de réseau : rien n'est gardé, on réessaiera au prochain passage du morceau.
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc() {
        let text = "[ar:Quelqu'un]\n[offset:+500]\n[00:12.50]Première\n[00:05.00][00:30.00]Refrain\n\
                    [01:02:50]<01:02.50>Mot <01:03.00>à mot\n[00:40.00]\n";
        let l = parse(text, Source::File).unwrap();
        assert!(l.synced);
        let times: Vec<f32> = l.lines.iter().map(|l| l.time.unwrap()).collect();
        assert_eq!(times, vec![4.5, 12.0, 29.5, 39.5, 62.0]);
        assert_eq!(l.lines[0].text, "Refrain");
        assert_eq!(l.lines[4].text, "Mot à mot");
        assert_eq!(l.current(0.0), None);
        assert_eq!(l.current(12.0), Some(1));
        assert_eq!(l.current(1000.0), Some(4));
    }

    #[test]
    fn plain() {
        let l = parse("\n\nUne ligne\n\nUne autre\n\n", Source::Tags).unwrap();
        assert!(!l.synced);
        assert_eq!(l.lines.len(), 3);
        assert_eq!(l.current(10.0), None);
        assert!(parse("[ar:Seulement des tags]\n", Source::File).is_none());
        assert_eq!(timestamp("1:02.5"), Some(62.5));
    }
}
