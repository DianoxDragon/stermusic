//! Second moteur de décodage : `ffmpeg`, pour ce que Symphonia ne sait pas lire (Opus, WMA,
//! APE, WavPack, DSD, Musepack…) et pour les morceaux lus à distance (serveurs, podcasts).
//!
//! `ffmpeg` décode vers du son brut (flottants 32 bits, stéréo, 44,1 kHz) sur sa sortie ; un
//! thread le lit et le passe au lecteur par paquets, pour que le son ne dépende jamais d'une
//! lecture bloquante. Un déplacement dans le morceau relance `ffmpeg` au bon endroit (`-ss`).

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};
use std::time::Duration;

const RATE: u32 = 44_100;
const CHANNELS: u16 = 2;

/// Extensions confiées d'office à `ffmpeg`.
pub const EXTENSIONS: &[&str] = &[
    "opus", "wma", "ape", "wv", "dsf", "dff", "mpc", "tta", "ac3", "mka", "webm",
];

/// Vrai si ce fichier doit être lu par `ffmpeg`.
pub fn wants(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Vrai si `ffmpeg` est installé (vérifié une fois).
pub fn available() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Le son d'un fichier ou d'une adresse, décodé par `ffmpeg`.
pub struct FfmpegSource {
    input: String,
    /// Début du morceau dans le fichier (morceau d'un fichier CUE), et sa durée.
    start: Duration,
    total: Option<Duration>,
    child: Option<Child>,
    rx: Receiver<Vec<f32>>,
    chunk: Vec<f32>,
    at: usize,
    ended: bool,
}

impl FfmpegSource {
    /// Ouvre `input` (chemin ou adresse), à partir de `start`.
    pub fn open(input: &str, start: Duration, total: Option<Duration>) -> Result<Self, String> {
        if !available() {
            return Err(crate::i18n::tr("Ce format demande ffmpeg (paquet « ffmpeg »)").into());
        }
        let (child, rx) = spawn(input, start)?;
        Ok(Self {
            input: input.to_owned(),
            start,
            total,
            child: Some(child),
            rx,
            chunk: Vec::new(),
            at: 0,
            ended: false,
        })
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn spawn(input: &str, at: Duration) -> Result<(Child, Receiver<Vec<f32>>), String> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-hide_banner", "-v", "error"]);
    if input.starts_with("http") {
        // Coupure réseau : ffmpeg se reconnecte au lieu d'abandonner.
        cmd.args([
            "-reconnect",
            "1",
            "-reconnect_streamed",
            "1",
            "-reconnect_delay_max",
            "5",
        ]);
    }
    if !at.is_zero() {
        cmd.args(["-ss", &format!("{:.3}", at.as_secs_f64())]);
    }
    cmd.args(["-i", input, "-vn", "-f", "f32le", "-ac", "2", "-ar", "44100", "-"]);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| crate::trf!("ffmpeg ne démarre pas : {e}", e))?;
    let mut out = child.stdout.take().ok_or("ffmpeg")?;
    // Environ trois secondes d'avance au plus (paquets de 0,1 s).
    let (tx, rx) = sync_channel::<Vec<f32>>(30);
    std::thread::spawn(move || {
        let mut bytes = vec![0u8; 4410 * 2 * 4];
        let mut carry: Vec<u8> = Vec::new();
        loop {
            let n = match out.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            carry.extend_from_slice(&bytes[..n]);
            let whole = carry.len() / 4 * 4;
            let samples: Vec<f32> = carry[..whole]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            carry.drain(..whole);
            if tx.send(samples).is_err() {
                break;
            }
        }
    });
    Ok((child, rx))
}

impl Iterator for FfmpegSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        while self.at >= self.chunk.len() {
            if self.ended {
                return None;
            }
            // Un réseau lent peut faire attendre : au-delà de vingt secondes, le morceau s'arrête.
            match self.rx.recv_timeout(Duration::from_secs(20)) {
                Ok(c) => {
                    self.chunk = c;
                    self.at = 0;
                }
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                    self.ended = true;
                    return None;
                }
            }
        }
        let s = self.chunk[self.at];
        self.at += 1;
        Some(s)
    }
}

impl rodio::Source for FfmpegSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        CHANNELS
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.stop();
        let (child, rx) =
            spawn(&self.input, self.start + pos).map_err(|_| rodio::source::SeekError::NotSupported {
                underlying_source: "ffmpeg",
            })?;
        self.child = Some(child);
        self.rx = rx;
        self.chunk.clear();
        self.at = 0;
        self.ended = false;
        Ok(())
    }
}

impl Drop for FfmpegSource {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------- Informations (ffprobe) ----------

/// Tags et durée lus par `ffprobe`, pour les formats que lofty ne connaît pas (WMA, DSD…).
#[derive(Default, Debug)]
pub struct Probe {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub composer: String,
    pub year: Option<u32>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub duration_ms: u64,
}

pub fn probe(path: &std::path::Path) -> Option<Probe> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_format", "-show_streams", "-of", "json"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let format = v.get("format")?;
    // Les tags sont dans le conteneur (WMA, MKA…) ou dans le flux audio (Ogg, Opus).
    let audio = v.get("streams").and_then(|s| s.as_array()).and_then(|s| {
        s.iter()
            .find(|st| st.get("codec_type").and_then(|c| c.as_str()) == Some("audio"))
    });
    let sets: Vec<&serde_json::Map<String, serde_json::Value>> =
        [format.get("tags"), audio.and_then(|a| a.get("tags"))]
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_object())
            .collect();
    let tag = |keys: &[&str]| -> String {
        for t in &sets {
            for (k, val) in t.iter() {
                if keys.iter().any(|key| k.eq_ignore_ascii_case(key)) {
                    if let Some(s) = val.as_str().filter(|s| !s.trim().is_empty()) {
                        return s.trim().to_owned();
                    }
                }
            }
        }
        String::new()
    };
    // « 3/12 » → 3 ; « 2004-05-01 » → 2004.
    let number = |s: String| s.split(['/', '-']).next().and_then(|n| n.trim().parse().ok());
    let duration = format
        .get("duration")
        .and_then(|d| d.as_str())
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);
    Some(Probe {
        title: tag(&["title"]),
        artist: tag(&["artist", "author"]),
        album: tag(&["album"]),
        album_artist: tag(&["album_artist", "albumartist", "WM/AlbumArtist"]),
        genre: tag(&["genre", "WM/Genre"]),
        composer: tag(&["composer", "WM/Composer"]),
        year: number(tag(&["date", "year", "WM/Year"])),
        track: number(tag(&["track", "WM/TrackNumber"])),
        disc: number(tag(&["disc", "WM/PartOfSet"])),
        duration_ms: (duration * 1000.0) as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source;

    /// Avec ffmpeg : un fichier Opus fabriqué pour l'occasion se lit et se déplace.
    #[test]
    fn opus_round_trip() {
        if !available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("stermusic-ff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("essai.opus");
        let ok = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=3",
            ])
            .args(["-metadata", "title=Essai", "-metadata", "artist=Moi"])
            .arg(&file)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok);
        let p = probe(&file).unwrap();
        assert_eq!((p.title.as_str(), p.artist.as_str()), ("Essai", "Moi"));
        assert!((2900..3100).contains(&p.duration_ms), "{}", p.duration_ms);
        let mut src = FfmpegSource::open(file.to_str().unwrap(), Duration::ZERO, None).unwrap();
        let n = src.by_ref().take(44_100 * 2).count();
        assert_eq!(n, 44_100 * 2);
        src.try_seek(Duration::from_secs(2)).unwrap();
        let rest = src.count();
        assert!((60_000..110_000).contains(&rest), "{rest}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
