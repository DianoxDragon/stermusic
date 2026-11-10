//! Podcasts : abonnements (flux RSS), épisodes, recherche dans l'annuaire d'Apple (public,
//! sans compte), téléchargement pour l'écoute hors ligne.
//!
//! Les épisodes de chaque flux sont gardés sur le disque (`podcasts/<clé>.json`) ; les fichiers
//! téléchargés vont dans `podcasts/fichiers/`.

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Podcast {
    /// Adresse du flux RSS (elle identifie le podcast).
    pub feed: String,
    pub title: String,
    pub author: String,
    pub image: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Episode {
    /// Identifiant de l'épisode dans le flux (sinon son adresse).
    pub guid: String,
    pub title: String,
    /// Date de publication (secondes depuis 1970).
    pub date: u64,
    pub duration_ms: u64,
    /// Adresse du son.
    pub url: String,
    pub description: String,
    /// Image propre à l'épisode (sinon celle du podcast).
    pub image: String,
}

impl Episode {
    /// Clé stable de l'épisode (avancement, fichier téléchargé).
    pub fn key(&self) -> String {
        hash(if self.guid.is_empty() {
            &self.url
        } else {
            &self.guid
        })
    }
}

/// Où en est l'écoute d'un épisode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
    pub position: f32,
    pub done: bool,
}

pub fn hash(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(concat!("SterMusic/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(timeout)
        .redirects(8)
        .build()
}

// ---------- Recherche ----------

/// Recherche dans l'annuaire des podcasts d'Apple (bloquant).
pub fn search(term: &str, country: &str) -> Result<Vec<Podcast>, String> {
    let v: serde_json::Value = agent(Duration::from_secs(15))
        .get("https://itunes.apple.com/search")
        .query("media", "podcast")
        .query("limit", "40")
        .query("country", country)
        .query("term", term)
        .call()
        .map_err(|e| e.to_string())
        .and_then(|r| serde_json::from_reader(r.into_reader()).map_err(|e| e.to_string()))?;
    Ok(v["results"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    Some(Podcast {
                        feed: r["feedUrl"].as_str()?.to_owned(),
                        title: r["collectionName"].as_str().unwrap_or_default().to_owned(),
                        author: r["artistName"].as_str().unwrap_or_default().to_owned(),
                        image: r["artworkUrl600"]
                            .as_str()
                            .or_else(|| r["artworkUrl100"].as_str())
                            .unwrap_or_default()
                            .to_owned(),
                        description: String::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

// ---------- Flux RSS ----------

/// Télécharge et lit un flux (bloquant).
pub fn fetch(feed: &str) -> Result<(Podcast, Vec<Episode>), String> {
    let mut text = String::new();
    agent(Duration::from_secs(30))
        .get(feed.trim())
        .call()
        .map_err(|e| e.to_string())?
        .into_reader()
        .take(32 << 20)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    let (mut podcast, episodes) =
        parse(&text).ok_or_else(|| crate::i18n::tr("Ce n'est pas un flux de podcast").to_owned())?;
    podcast.feed = feed.trim().to_owned();
    Ok((podcast, episodes))
}

/// Contenu du premier élément `<name …>…</name>` de `xml`.
fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    loop {
        let i = from + xml[from..].find(&format!("<{name}"))?;
        let after = &xml[i + name.len() + 1..];
        // « <title> » ou « <title attr> », pas « <titles> ».
        match after.chars().next() {
            Some('>') | Some(' ') | Some('\t') | Some('\n') | Some('\r') | Some('/') => {}
            _ => {
                from = i + 1;
                continue;
            }
        }
        let open_end = after.find('>')?;
        if after[..open_end].ends_with('/') {
            return Some("");
        }
        let body = &after[open_end + 1..];
        let close = body.find(&format!("</{name}>"))?;
        return Some(&body[..close]);
    }
}

/// Attribut `attr` de la première balise `<name …>`.
fn attribute(xml: &str, name: &str, attr: &str) -> Option<String> {
    let mut from = 0;
    loop {
        let i = from + xml[from..].find(&format!("<{name}"))?;
        let rest = &xml[i + name.len() + 1..];
        if !rest.starts_with([' ', '\t', '\n', '\r']) {
            from = i + 1;
            continue;
        }
        let tag = &rest[..rest.find('>')?];
        for quote in ['"', '\''] {
            let key = format!("{attr}={quote}");
            if let Some(k) = tag.find(&key) {
                let v = &tag[k + key.len()..];
                return v.find(quote).map(|e| decode(&v[..e]));
            }
        }
        return None;
    }
}

/// Texte lisible : CDATA ôté, balises HTML retirées, entités décodées, espaces resserrés.
fn text(raw: &str) -> String {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix("<![CDATA[")
        .and_then(|r| r.strip_suffix("]]>"))
        .unwrap_or(raw);
    let decoded = decode(raw);
    let mut out = String::new();
    let mut in_tag = false;
    for c in decoded.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let Some(end) = after.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &after[1..];
            continue;
        };
        let entity = &after[1..end];
        let c = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => out.push(c),
            None => out.push_str(&after[..=end]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// « 01:02:03 », « 62:03 » ou « 3723 » → millisecondes.
fn duration(s: &str) -> u64 {
    let parts: Vec<f64> = s
        .trim()
        .split(':')
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    let secs = parts.iter().fold(0.0, |acc, p| acc * 60.0 + p);
    (secs * 1000.0) as u64
}

/// Date RFC 2822 : « Tue, 05 Mar 2024 18:22:10 +0100 » → secondes depuis 1970.
fn date(s: &str) -> u64 {
    let s = s.trim();
    let s = s.split_once(',').map(|(_, r)| r).unwrap_or(s);
    let mut p = s.split_whitespace();
    let (Some(day), Some(mon), Some(year)) = (p.next(), p.next(), p.next()) else {
        return 0;
    };
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let Some(m) = months
        .iter()
        .position(|x| mon.to_ascii_lowercase().starts_with(x))
    else {
        return 0;
    };
    let time = p.next().unwrap_or("00:00:00");
    let iso = format!("{year}-{:02}-{:0>2}T{time}", m + 1, day);
    let base = crate::servers::parse_iso(&iso).unwrap_or(0) as i64;
    // Fuseau : « +0100 », « -0500 » (les noms comme GMT valent zéro).
    let offset = p
        .next()
        .filter(|z| z.len() == 5 && (z.starts_with('+') || z.starts_with('-')))
        .and_then(|z| {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let h: i64 = z[1..3].parse().ok()?;
            let mi: i64 = z[3..5].parse().ok()?;
            Some(sign * (h * 3600 + mi * 60))
        })
        .unwrap_or(0);
    (base - offset).max(0) as u64
}

pub fn parse(xml: &str) -> Option<(Podcast, Vec<Episode>)> {
    let channel = element(xml, "channel")?;
    // Les informations du podcast sont avant le premier épisode.
    let head = &channel[..channel.find("<item").unwrap_or(channel.len())];
    let image = attribute(head, "itunes:image", "href")
        .or_else(|| element(head, "image").and_then(|i| element(i, "url")).map(text))
        .unwrap_or_default();
    let podcast = Podcast {
        feed: String::new(),
        title: element(head, "title").map(text).unwrap_or_default(),
        author: element(head, "itunes:author")
            .or_else(|| element(head, "author"))
            .map(text)
            .unwrap_or_default(),
        image,
        description: element(head, "description")
            .or_else(|| element(head, "itunes:summary"))
            .map(text)
            .unwrap_or_default(),
    };
    let mut episodes = Vec::new();
    let mut rest = &channel[head.len()..];
    while let Some(start) = rest.find("<item") {
        let Some(end) = rest[start..].find("</item>") else {
            break;
        };
        let item = &rest[start..start + end];
        rest = &rest[start + end + 7..];
        let Some(url) = attribute(item, "enclosure", "url") else {
            continue;
        };
        let description = element(item, "itunes:summary")
            .or_else(|| element(item, "description"))
            .map(text)
            .unwrap_or_default();
        episodes.push(Episode {
            guid: element(item, "guid").map(text).unwrap_or_default(),
            title: element(item, "title").map(text).unwrap_or_default(),
            date: element(item, "pubDate").map(|d| date(&text(d))).unwrap_or(0),
            duration_ms: element(item, "itunes:duration")
                .map(|d| duration(&text(d)))
                .unwrap_or(0),
            url,
            description: description.chars().take(600).collect(),
            image: attribute(item, "itunes:image", "href").unwrap_or_default(),
        });
    }
    episodes.sort_by_key(|e| std::cmp::Reverse(e.date));
    Some((podcast, episodes))
}

// ---------- Disque ----------

fn dir() -> PathBuf {
    crate::library::data_dir().join("podcasts")
}

pub fn load_episodes(feed: &str) -> Vec<Episode> {
    std::fs::read(dir().join(format!("{}.json", hash(feed))))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_episodes(feed: &str, episodes: &[Episode]) {
    let _ = std::fs::create_dir_all(dir());
    if let Ok(json) = serde_json::to_vec(episodes) {
        let _ = std::fs::write(dir().join(format!("{}.json", hash(feed))), json);
    }
}

/// Fichier d'un épisode téléchargé (qu'il existe ou non).
pub fn download_path(ep: &Episode) -> PathBuf {
    let ext = ep
        .url
        .split('?')
        .next()
        .and_then(|u| u.rsplit('.').next())
        .filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("mp3");
    dir().join("fichiers").join(format!("{}.{ext}", ep.key()))
}

pub fn downloaded(ep: &Episode) -> Option<PathBuf> {
    let p = download_path(ep);
    p.is_file().then_some(p)
}

/// Télécharge un épisode ; `progress` reçoit la part faite (0 → 1). Bloquant.
pub fn download(ep: &Episode, progress: &std::sync::atomic::AtomicU32) -> Result<PathBuf, String> {
    let target = download_path(ep);
    let resp = agent(Duration::from_secs(60))
        .get(&ep.url)
        .call()
        .map_err(|e| e.to_string())?;
    let total: u64 = resp
        .header("content-length")
        .and_then(|l| l.parse().ok())
        .unwrap_or(0);
    let _ = std::fs::create_dir_all(target.parent().unwrap_or(&dir()));
    let tmp = target.with_extension("part");
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if total > 0 {
            let part = (done as f64 / total as f64).min(1.0);
            progress.store((part * 1000.0) as u32, std::sync::atomic::Ordering::Relaxed);
        }
    }
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss() {
        let xml = r#"<?xml version="1.0"?><rss xmlns:itunes="x"><channel>
            <title>Mon émission</title><itunes:author>Radio &amp; Co</itunes:author>
            <itunes:image href="https://ex.org/cover.jpg"/>
            <description><![CDATA[<p>Un podcast <b>génial</b>.</p>]]></description>
            <item><title>Épisode 1</title><guid isPermaLink="false">ep-1</guid>
              <pubDate>Tue, 05 Mar 2024 19:22:10 +0100</pubDate>
              <enclosure url="https://ex.org/1.mp3?x=1" type="audio/mpeg" length="12"/>
              <itunes:duration>01:02:03</itunes:duration></item>
            <item><title>Épisode 2</title><pubDate>Wed, 06 Mar 2024 10:00:00 GMT</pubDate>
              <enclosure url='https://ex.org/2.m4a' type='audio/mp4'/><itunes:duration>754</itunes:duration></item>
            <item><title>Sans son</title></item>
        </channel></rss>"#;
        let (p, eps) = parse(xml).unwrap();
        assert_eq!(p.title, "Mon émission");
        assert_eq!(p.author, "Radio & Co");
        assert_eq!(p.image, "https://ex.org/cover.jpg");
        assert_eq!(p.description, "Un podcast génial .");
        assert_eq!(eps.len(), 2);
        // Le plus récent d'abord.
        assert_eq!(eps[0].title, "Épisode 2");
        assert_eq!(eps[0].duration_ms, 754_000);
        assert_eq!(eps[1].duration_ms, 3_723_000);
        assert_eq!(eps[1].date, 1_709_662_930);
        assert_eq!(eps[1].guid, "ep-1");
        assert_eq!(eps[1].url, "https://ex.org/1.mp3?x=1");
        assert!(download_path(&eps[1]).to_string_lossy().ends_with(".mp3"));
        assert!(download_path(&eps[0]).to_string_lossy().ends_with(".m4a"));
    }
}

#[cfg(test)]
mod live {
    /// Réseau : `cargo test live_podcast -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_podcast() {
        let hits = super::search("radio france", "FR").expect("annuaire");
        eprintln!(
            "{} résultats ; 1er : {} — {}",
            hits.len(),
            hits[0].title,
            hits[0].author
        );
        let (p, eps) = super::fetch(&hits[0].feed).expect("flux");
        eprintln!(
            "« {} » : {} épisodes, dernier « {} » ({} ms)",
            p.title,
            eps.len(),
            eps[0].title,
            eps[0].duration_ms
        );
        assert!(!eps.is_empty());
        let src = crate::ffdecode::FfmpegSource::open(&eps[0].url, std::time::Duration::ZERO, None).unwrap();
        let n = src.take(44_100 * 2).count();
        eprintln!("{n} échantillons lus");
        assert_eq!(n, 44_100 * 2);
    }
}
