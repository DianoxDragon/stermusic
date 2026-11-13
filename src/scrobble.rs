//! Scrobbling: sends plays to ListenBrainz and/or Last.fm.
//!
//! Rule shared by both services: a track of at least 30 seconds counts once half of it has
//! been heard, or 4 minutes of it. The current track is also reported ("now playing"). A thread
//! does the sending; what could not go out (no network) waits in `scrobbles.json` and leaves
//! with the next successful submission.
//!
//! ListenBrainz only needs a personal token. Last.fm asks for an API key (free, to be created
//! on last.fm) and an authorization given in the browser.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const LB_API: &str = "https://api.listenbrainz.org/1";
const LASTFM_API: &str = "https://ws.audioscrobbler.com/2.0/";
pub const LASTFM_NEW_KEY: &str = "https://www.last.fm/api/account/create";
pub const LB_TOKEN_PAGE: &str = "https://listenbrainz.org/settings/";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub listenbrainz: bool,
    pub lb_token: String,
    /// Account name, known once the token has been checked.
    pub lb_user: String,
    pub lastfm: bool,
    pub lastfm_key: String,
    pub lastfm_secret: String,
    /// Session key given by Last.fm after authorization, and account name.
    pub lastfm_session: String,
    pub lastfm_user: String,
}

impl Config {
    fn lb_ready(&self) -> bool {
        self.listenbrainz && !self.lb_token.is_empty()
    }

    fn lastfm_ready(&self) -> bool {
        self.lastfm && !self.lastfm_session.is_empty() && !self.lastfm_secret.is_empty()
    }

    pub fn any(&self) -> bool {
        self.lb_ready() || self.lastfm_ready()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Song {
    pub artist: String,
    pub title: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub album_artist: String,
    pub duration: u32,
    #[serde(default)]
    pub track_no: Option<u32>,
}

/// A play to submit: the track, when it started, and to which service(s).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pending {
    song: Song,
    time: u64,
    #[serde(default)]
    lb: bool,
    #[serde(default)]
    lastfm: bool,
}

/// Should this play be submitted? (`heard`: seconds really heard.)
pub fn counts(duration: f32, heard: f32) -> bool {
    duration >= 30.0 && heard >= (duration / 2.0).min(240.0)
}

/// State of the submissions, for the settings.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub sent: usize,
    pub pending: usize,
    pub error: Option<String>,
}

enum Msg {
    Config(Config),
    NowPlaying(Song),
    Scrobble(Song, u64),
}

pub struct Scrobbler {
    tx: Sender<Msg>,
    pub status: Arc<Mutex<Status>>,
}

impl Scrobbler {
    pub fn new(config: Config) -> Self {
        let (tx, rx) = channel();
        let status = Arc::new(Mutex::new(Status {
            pending: load_pending().len(),
            ..Default::default()
        }));
        let st = status.clone();
        std::thread::spawn(move || worker(rx, config, st));
        Self { tx, status }
    }

    pub fn set_config(&self, config: Config) {
        let _ = self.tx.send(Msg::Config(config));
    }

    pub fn now_playing(&self, song: Song) {
        let _ = self.tx.send(Msg::NowPlaying(song));
    }

    pub fn scrobble(&self, song: Song, started: u64) {
        let _ = self.tx.send(Msg::Scrobble(song, started));
    }

    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn worker(rx: Receiver<Msg>, mut config: Config, status: Arc<Mutex<Status>>) {
    let agent = ureq::AgentBuilder::new()
        .user_agent(concat!("SterMusic/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .build();
    let mut pending = load_pending();
    let mut next_retry = Instant::now() + Duration::from_secs(20);
    let set_status = |pending: &[Pending], sent: usize, error: Option<String>| {
        if let Ok(mut s) = status.lock() {
            s.sent += sent;
            s.pending = pending.len();
            if error.is_some() || sent > 0 {
                s.error = error;
            }
        }
    };
    loop {
        let msg = match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        match msg {
            Some(Msg::Config(c)) => {
                config = c;
                next_retry = Instant::now();
            }
            Some(Msg::NowPlaying(song)) => {
                // It doesn't matter if this fails: nothing is kept.
                if config.lb_ready() {
                    let _ = lb_submit(&agent, &config, "playing_now", &[(&song, None)]);
                }
                if config.lastfm_ready() {
                    let _ = lastfm_now_playing(&agent, &config, &song);
                }
            }
            Some(Msg::Scrobble(song, time)) if config.any() => {
                pending.push(Pending {
                    song,
                    time,
                    lb: config.lb_ready(),
                    lastfm: config.lastfm_ready(),
                });
                save_pending(&pending);
                next_retry = Instant::now();
            }
            Some(Msg::Scrobble(..)) | None => {}
        }
        if pending.is_empty() || Instant::now() < next_retry {
            set_status(&pending, 0, None);
            continue;
        }
        let before = pending.len();
        let error = flush(&agent, &config, &mut pending);
        let sent = before - pending.len();
        save_pending(&pending);
        // Failure: try again in five minutes (or at the next track played).
        next_retry = Instant::now()
            + if error.is_some() {
                Duration::from_secs(300)
            } else {
                Duration::ZERO
            };
        set_status(&pending, sent, error);
    }
}

/// Submits what is waiting, in batches; keeps what didn't go out. Returns the last error.
fn flush(agent: &ureq::Agent, config: &Config, pending: &mut Vec<Pending>) -> Option<String> {
    let mut error = None;
    // A play meant for a service disabled since then no longer waits for it.
    for p in pending.iter_mut() {
        p.lb &= config.lb_ready();
        p.lastfm &= config.lastfm_ready();
    }
    if config.lb_ready() {
        let batch: Vec<usize> = (0..pending.len()).filter(|&i| pending[i].lb).take(500).collect();
        if !batch.is_empty() {
            let items: Vec<(&Song, Option<u64>)> = batch
                .iter()
                .map(|&i| (&pending[i].song, Some(pending[i].time)))
                .collect();
            let kind = if items.len() == 1 { "single" } else { "import" };
            match lb_submit(agent, config, kind, &items) {
                Ok(()) => batch.iter().for_each(|&i| pending[i].lb = false),
                Err(e) => error = Some(format!("ListenBrainz : {e}")),
            }
        }
    }
    if config.lastfm_ready() {
        let batch: Vec<usize> = (0..pending.len())
            .filter(|&i| pending[i].lastfm)
            .take(50)
            .collect();
        if !batch.is_empty() {
            let items: Vec<(&Song, u64)> = batch
                .iter()
                .map(|&i| (&pending[i].song, pending[i].time))
                .collect();
            match lastfm_scrobble(agent, config, &items) {
                Ok(()) => batch.iter().for_each(|&i| pending[i].lastfm = false),
                Err(e) => error = Some(format!("Last.fm : {e}")),
            }
        }
    }
    pending.retain(|p| p.lb || p.lastfm);
    error
}

// ---------- ListenBrainz ----------

fn lb_submit(
    agent: &ureq::Agent,
    config: &Config,
    kind: &str,
    items: &[(&Song, Option<u64>)],
) -> Result<(), String> {
    let payload: Vec<serde_json::Value> = items
        .iter()
        .map(|(song, time)| {
            let mut info = serde_json::json!({
                "media_player": "SterMusic",
                "submission_client": "SterMusic",
                "submission_client_version": env!("CARGO_PKG_VERSION"),
                "duration_ms": song.duration as u64 * 1000,
            });
            if let Some(n) = song.track_no {
                info["tracknumber"] = n.into();
            }
            let mut meta = serde_json::json!({
                "artist_name": song.artist,
                "track_name": song.title,
                "additional_info": info,
            });
            if !song.album.is_empty() {
                meta["release_name"] = song.album.clone().into();
            }
            let mut listen = serde_json::json!({ "track_metadata": meta });
            if let Some(t) = time {
                listen["listened_at"] = (*t).into();
            }
            listen
        })
        .collect();
    let body = serde_json::json!({ "listen_type": kind, "payload": payload });
    agent
        .post(&format!("{LB_API}/submit-listens"))
        .set("Authorization", &format!("Token {}", config.lb_token.trim()))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map(|_| ())
        .map_err(describe)
}

/// Checks a ListenBrainz token; returns the account name (blocking).
pub fn lb_validate(token: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Reply {
        valid: bool,
        #[serde(default)]
        user_name: String,
    }
    let reply: Reply = ureq::get(&format!("{LB_API}/validate-token"))
        .set("Authorization", &format!("Token {}", token.trim()))
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(describe)
        .and_then(|r| serde_json::from_reader(r.into_reader()).map_err(|e| e.to_string()))?;
    if reply.valid {
        Ok(reply.user_name)
    } else {
        Err(crate::i18n::tr("Jeton refusé").into())
    }
}

// ---------- Last.fm ----------

/// Signature of a call: MD5 of the sorted parameters (key then value), followed by the secret.
fn lastfm_sign(params: &[(String, String)], secret: &str) -> String {
    let mut sorted: Vec<&(String, String)> = params.iter().filter(|(k, _)| k != "format").collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut text = String::new();
    for (k, v) in sorted {
        text.push_str(k);
        text.push_str(v);
    }
    text.push_str(secret);
    md5_hex(text.as_bytes())
}

fn lastfm_call(
    agent: &ureq::Agent,
    key: &str,
    secret: &str,
    mut params: Vec<(String, String)>,
) -> Result<serde_json::Value, String> {
    params.push(("api_key".into(), key.trim().into()));
    let sig = lastfm_sign(&params, secret.trim());
    params.push(("api_sig".into(), sig));
    params.push(("format".into(), "json".into()));
    let form: Vec<(&str, &str)> = params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let value: serde_json::Value = agent
        .post(LASTFM_API)
        .send_form(&form)
        .map_err(describe)
        .and_then(|r| serde_json::from_reader(r.into_reader()).map_err(|e| e.to_string()))?;
    if let Some(msg) = value.get("message").and_then(|m| m.as_str()) {
        if value.get("error").is_some() {
            return Err(msg.to_owned());
        }
    }
    Ok(value)
}

fn song_params(song: &Song, suffix: &str) -> Vec<(String, String)> {
    let mut p = vec![
        (format!("artist{suffix}"), song.artist.clone()),
        (format!("track{suffix}"), song.title.clone()),
    ];
    if !song.album.is_empty() {
        p.push((format!("album{suffix}"), song.album.clone()));
    }
    if !song.album_artist.is_empty() && song.album_artist != song.artist {
        p.push((format!("albumArtist{suffix}"), song.album_artist.clone()));
    }
    if song.duration > 0 {
        p.push((format!("duration{suffix}"), song.duration.to_string()));
    }
    if let Some(n) = song.track_no {
        p.push((format!("trackNumber{suffix}"), n.to_string()));
    }
    p
}

fn lastfm_now_playing(agent: &ureq::Agent, config: &Config, song: &Song) -> Result<(), String> {
    let mut params = vec![
        ("method".into(), "track.updateNowPlaying".into()),
        ("sk".into(), config.lastfm_session.clone()),
    ];
    params.extend(song_params(song, ""));
    lastfm_call(agent, &config.lastfm_key, &config.lastfm_secret, params).map(|_| ())
}

fn lastfm_scrobble(agent: &ureq::Agent, config: &Config, items: &[(&Song, u64)]) -> Result<(), String> {
    let mut params = vec![
        ("method".into(), "track.scrobble".into()),
        ("sk".into(), config.lastfm_session.clone()),
    ];
    for (i, (song, time)) in items.iter().enumerate() {
        let suffix = format!("[{i}]");
        params.extend(song_params(song, &suffix));
        params.push((format!("timestamp{suffix}"), time.to_string()));
    }
    lastfm_call(agent, &config.lastfm_key, &config.lastfm_secret, params).map(|_| ())
}

/// First step of authorization: a token, and the page to open in the browser so that the user
/// authorizes SterMusic (blocking).
pub fn lastfm_begin(key: &str, secret: &str) -> Result<(String, String), String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build();
    let reply = lastfm_call(
        &agent,
        key,
        secret,
        vec![("method".into(), "auth.getToken".into())],
    )?;
    let token = reply
        .get("token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| crate::i18n::tr("Réponse de Last.fm inattendue").to_owned())?
        .to_owned();
    let url = format!(
        "https://www.last.fm/api/auth/?api_key={}&token={}",
        key.trim(),
        token
    );
    Ok((token, url))
}

/// Second step, once authorization is given: the session key and the account name.
pub fn lastfm_finish(key: &str, secret: &str, token: &str) -> Result<(String, String), String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build();
    let reply = lastfm_call(
        &agent,
        key,
        secret,
        vec![
            ("method".into(), "auth.getSession".into()),
            ("token".into(), token.into()),
        ],
    )?;
    let session = reply
        .get("session")
        .ok_or_else(|| crate::i18n::tr("Réponse de Last.fm inattendue").to_owned())?;
    let sk = session.get("key").and_then(|v| v.as_str()).unwrap_or_default();
    let name = session.get("name").and_then(|v| v.as_str()).unwrap_or_default();
    if sk.is_empty() {
        return Err(crate::i18n::tr("Réponse de Last.fm inattendue").into());
    }
    Ok((sk.to_owned(), name.to_owned()))
}

/// Readable message for an HTTP error (the response body often says why).
fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("message")
                        .or_else(|| v.get("error"))
                        .and_then(|m| m.as_str().map(str::to_owned))
                })
                .unwrap_or_default();
            if msg.is_empty() {
                format!("HTTP {code}")
            } else {
                format!("{msg} (HTTP {code})")
            }
        }
        ureq::Error::Transport(t) => t.to_string(),
    }
}

// ---------- Queue on disk ----------

fn pending_file() -> PathBuf {
    crate::library::data_dir().join("scrobbles.json")
}

fn load_pending() -> Vec<Pending> {
    std::fs::read(pending_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_pending(pending: &[Pending]) {
    let file = pending_file();
    if pending.is_empty() {
        let _ = std::fs::remove_file(file);
        return;
    }
    let _ = std::fs::create_dir_all(crate::library::data_dir());
    if let Ok(json) = serde_json::to_vec(pending) {
        let tmp = file.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, file);
        }
    }
}

// ---------- MD5 (signing Last.fm calls) ----------

pub(crate) fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
        5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15,
        21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32)
        .collect();
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    [a0, b0, c0, d0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        // Longer than a 64-byte block.
        assert_eq!(md5_hex(&[b'a'; 100]), "36a92cc94a9e0fa21f625f8bfb007adf");
    }

    #[test]
    fn signature() {
        let params = vec![
            ("method".to_owned(), "auth.getToken".to_owned()),
            ("api_key".to_owned(), "abc".to_owned()),
            ("format".to_owned(), "json".to_owned()),
        ];
        assert_eq!(
            lastfm_sign(&params, "s3cr3t"),
            md5_hex(b"api_keyabcmethodauth.getTokens3cr3t")
        );
    }

    #[test]
    fn rule() {
        assert!(!counts(25.0, 25.0));
        assert!(counts(200.0, 100.0));
        assert!(!counts(200.0, 99.0));
        assert!(counts(900.0, 240.0));
    }
}
