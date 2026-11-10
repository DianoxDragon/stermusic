//! Conversion de morceaux par ffmpeg (MP3, Opus, AAC, Vorbis, FLAC) : outil « Convertir »,
//! extraction de CD et synchronisation vers un appareil. Les tags suivent, la pochette aussi
//! quand le format la garde.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::library::Track;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    #[default]
    Mp3,
    Mp3V0,
    Opus,
    Aac,
    Vorbis,
    Flac,
}

impl Format {
    pub const ALL: [Format; 6] = [
        Format::Mp3,
        Format::Mp3V0,
        Format::Opus,
        Format::Aac,
        Format::Vorbis,
        Format::Flac,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Format::Mp3 => "MP3 320 kb/s",
            Format::Mp3V0 => "MP3 V0",
            Format::Opus => "Opus 160 kb/s",
            Format::Aac => "AAC 256 kb/s",
            Format::Vorbis => "Ogg Vorbis q6",
            Format::Flac => "FLAC",
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Mp3 | Format::Mp3V0 => "mp3",
            Format::Opus => "opus",
            Format::Aac => "m4a",
            Format::Vorbis => "ogg",
            Format::Flac => "flac",
        }
    }

    fn codec(self) -> &'static [&'static str] {
        match self {
            Format::Mp3 => &["-c:a", "libmp3lame", "-b:a", "320k", "-id3v2_version", "3"],
            Format::Mp3V0 => &["-c:a", "libmp3lame", "-q:a", "0", "-id3v2_version", "3"],
            Format::Opus => &["-c:a", "libopus", "-b:a", "160k"],
            Format::Aac => &["-c:a", "aac", "-b:a", "256k", "-movflags", "+faststart"],
            Format::Vorbis => &["-c:a", "libvorbis", "-q:a", "6"],
            Format::Flac => &["-c:a", "flac"],
        }
    }

    /// ffmpeg sait-il recopier la pochette dans ce format ?
    fn keeps_cover(self) -> bool {
        matches!(self, Format::Mp3 | Format::Mp3V0 | Format::Aac | Format::Flac)
    }
}

/// Ce qu'on donne à ffmpeg comme entrée.
pub struct Source {
    pub input: String,
    /// Format d'entrée forcé (« libcdio » pour un CD).
    pub format: Option<&'static str>,
    pub start: Option<Duration>,
    pub end: Option<Duration>,
}

impl Source {
    /// La source d'un morceau de la bibliothèque (fichier, morceau CUE, morceau distant).
    pub fn of(t: &Track) -> Self {
        let m = crate::library::media(t);
        Self {
            input: m.ffmpeg_input(),
            format: None,
            start: (!m.start.is_zero()).then_some(m.start),
            end: m.end,
        }
    }
}

/// Tags écrits dans le fichier converti.
pub fn tags_of(t: &Track) -> Vec<(&'static str, String)> {
    let mut v = vec![
        ("title", t.title.clone()),
        ("artist", t.artist.clone()),
        ("album", t.album.clone()),
    ];
    if !t.album_artist.is_empty() {
        v.push(("album_artist", t.album_artist.clone()));
    }
    if !t.genre.is_empty() {
        v.push(("genre", t.genre.clone()));
    }
    if !t.composer.is_empty() {
        v.push(("composer", t.composer.clone()));
    }
    if let Some(y) = t.year {
        v.push(("date", y.to_string()));
    }
    if let Some(n) = t.track_no {
        v.push(("track", n.to_string()));
    }
    if let Some(d) = t.disc_no {
        v.push(("disc", d.to_string()));
    }
    v
}

/// Nom de fichier sans caractères gênants (pour Windows, Android et les cartes FAT).
pub fn safe_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    let mut out: String = trimmed.chars().take(120).collect();
    if out.is_empty() {
        out = "_".into();
    }
    out
}

/// « Artiste/Album/03 - Titre.mp3 ».
pub fn relative_path(t: &Track, ext: &str) -> PathBuf {
    let artist = safe_name(t.album_artist_or_artist());
    let album = safe_name(t.album_or_unknown());
    let name = match t.track_no {
        Some(n) => format!("{n:02} - {}", t.title),
        None => t.title.clone(),
    };
    PathBuf::from(artist)
        .join(album)
        .join(format!("{}.{ext}", safe_name(&name)))
}

/// Convertit `src` vers `out` (dossiers créés au besoin). Bloquant ; `stop` interrompt.
pub fn run(
    src: &Source,
    out: &Path,
    format: Format,
    tags: &[(&str, String)],
    stop: &AtomicBool,
) -> Result<(), String> {
    if !crate::ffdecode::available() {
        return Err(crate::i18n::tr("ffmpeg est nécessaire (paquet « ffmpeg »)").into());
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = out.with_extension(format!("part.{}", format.ext()));
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-hide_banner", "-v", "error", "-y"]);
    if let Some(f) = src.format {
        cmd.args(["-f", f]);
    }
    if let Some(s) = src.start {
        cmd.args(["-ss", &format!("{:.3}", s.as_secs_f64())]);
    }
    if let Some(e) = src.end {
        cmd.args(["-to", &format!("{:.3}", e.as_secs_f64())]);
    }
    cmd.args(["-i", &src.input, "-map", "0:a:0"]);
    if format.keeps_cover() && src.format.is_none() {
        cmd.args(["-map", "0:v?", "-c:v", "copy", "-disposition:v", "attached_pic"]);
    } else {
        cmd.arg("-vn");
    }
    cmd.args(["-map_metadata", "-1"]);
    for (k, v) in tags {
        cmd.args(["-metadata", &format!("{k}={v}")]);
    }
    cmd.args(format.codec()).arg(&tmp);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&tmp);
            return Err(crate::i18n::tr("interrompu").into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return std::fs::rename(&tmp, out).map_err(|e| e.to_string());
                }
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut e, &mut err);
                }
                let _ = std::fs::remove_file(&tmp);
                let line = err.lines().last().unwrap_or("ffmpeg").to_owned();
                return Err(line);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(60)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

// ---------- CD audio ----------

/// Le lecteur de CD, s'il y en a un.
pub fn cd_drive() -> Option<PathBuf> {
    ["/dev/sr0", "/dev/cdrom", "/dev/sr1"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

/// Les pistes du CD (début et fin), lues par ffprobe (libcdio). Bloquant.
pub fn cd_tracks(drive: &Path) -> Result<Vec<(Duration, Duration)>, String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-f",
            "libcdio",
            "-show_chapters",
            "-of",
            "json",
            "-i",
        ])
        .arg(drive)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if err.is_empty() {
            crate::i18n::tr("Aucun CD audio dans le lecteur").to_owned()
        } else {
            err
        }
    })?;
    let secs = |c: &serde_json::Value, k: &str| -> Duration {
        Duration::from_secs_f64(c[k].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0))
    };
    let chapters: Vec<(Duration, Duration)> = v["chapters"]
        .as_array()
        .map(|c| {
            c.iter()
                .map(|c| (secs(c, "start_time"), secs(c, "end_time")))
                .collect()
        })
        .unwrap_or_default();
    if chapters.is_empty() {
        return Err(crate::i18n::tr("Aucun CD audio dans le lecteur").into());
    }
    Ok(chapters)
}

/// Table des matières pour MusicBrainz : débuts des pistes et fin du disque, en secteurs.
pub fn toc(tracks: &[(Duration, Duration)]) -> (Vec<u32>, u32) {
    let sector = |d: Duration| (d.as_secs_f64() * 75.0).round() as u32 + 150;
    let offsets = tracks.iter().map(|t| sector(t.0)).collect();
    let leadout = tracks.last().map(|t| sector(t.1)).unwrap_or(150);
    (offsets, leadout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(safe_name(" AC/DC: Live? "), "AC_DC_ Live_");
        assert_eq!(safe_name("..."), "_");
        let t = Track {
            path: "/x.flac".into(),
            title: "Hells Bells".into(),
            artist: "AC/DC".into(),
            album: "Back in Black".into(),
            album_artist: String::new(),
            genre: String::new(),
            year: Some(1980),
            track_no: Some(1),
            disc_no: None,
            duration_ms: 0,
            mtime: 0,
            composer: String::new(),
            schema: 0,
            cue: None,
            remote: None,
        };
        assert_eq!(
            relative_path(&t, "mp3"),
            PathBuf::from("AC_DC/Back in Black/01 - Hells Bells.mp3")
        );
        let tags = tags_of(&t);
        assert!(tags.contains(&("date", "1980".into())));
    }

    #[test]
    fn cd_toc() {
        let tracks = [
            (Duration::ZERO, Duration::from_secs(200)),
            (Duration::from_secs(200), Duration::from_secs(450)),
        ];
        assert_eq!(toc(&tracks), (vec![150, 15_150], 33_900));
    }

    /// Avec ffmpeg : un FLAC avec pochette devient un MP3 qui la garde, tags compris.
    #[test]
    fn mp3_with_tags() {
        if !crate::ffdecode::available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("stermusic-conv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.flac");
        assert!(Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=duration=2"])
            .arg(&input)
            .status()
            .unwrap()
            .success());
        let out = dir.join("A/B/01 - C.mp3");
        let src = Source {
            input: input.to_string_lossy().into(),
            format: None,
            start: Some(Duration::from_millis(500)),
            end: None,
        };
        run(
            &src,
            &out,
            Format::Mp3,
            &[("title", "C".into())],
            &AtomicBool::new(false),
        )
        .unwrap();
        let p = crate::ffdecode::probe(&out).unwrap();
        assert_eq!(p.title, "C");
        assert!((1300..1700).contains(&p.duration_ms), "{}", p.duration_ms);
        let _ = std::fs::remove_dir_all(dir);
    }
}
