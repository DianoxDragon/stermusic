//! Diffusion vers un appareil du réseau : téléviseurs, enceintes et amplis DLNA / UPnP, et
//! Chromecast (ou enceintes « Google Cast »).
//!
//! - Découverte : SSDP (UPnP) et mDNS (`_googlecast._tcp`), en même temps, deux secondes et demie.
//! - Les fichiers sont servis à l'appareil par un petit serveur HTTP local ; ce qu'il ne sait
//!   pas lire (morceau d'un fichier CUE, WMA, APE…) est converti à la volée en MP3 par ffmpeg.
//!   Un déplacement dans un flux converti le relance au bon endroit.
//! - Chromecast : protocole Cast v2 (TLS, messages protobuf portant du JSON), récepteur
//!   multimédia par défaut. DLNA : commandes SOAP AVTransport et RenderingControl.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::media::{Input, Media};

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Dlna { av: String, rendering: Option<String> },
    Chromecast { addr: SocketAddr },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub name: String,
    pub model: String,
    pub kind: Kind,
}

impl Device {
    pub fn is_chromecast(&self) -> bool {
        matches!(self.kind, Kind::Chromecast { .. })
    }

    fn host(&self) -> Option<IpAddr> {
        match &self.kind {
            Kind::Chromecast { addr } => Some(addr.ip()),
            Kind::Dlna { av, .. } => av.split("://").nth(1)?.split(['/', ':']).next()?.parse().ok(),
        }
    }
}

// ---------- Découverte ----------

/// Cherche les appareils du réseau (bloquant, deux secondes et demie).
pub fn discover() -> Vec<Device> {
    let (tx, rx) = channel();
    let t1 = tx.clone();
    std::thread::spawn(move || {
        for d in discover_dlna() {
            let _ = t1.send(d);
        }
    });
    std::thread::spawn(move || {
        for d in discover_cast() {
            let _ = tx.send(d);
        }
    });
    let mut out: Vec<Device> = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(3500);
    while let Ok(d) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        if !out.iter().any(|o| o.name == d.name && o.kind == d.kind) {
            out.push(d);
        }
    }
    out.sort_by_key(|d| d.name.to_lowercase());
    out
}

fn discover_dlna() -> Vec<Device> {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return Vec::new();
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
    let msg = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\n\
               ST: urn:schemas-upnp-org:service:AVTransport:1\r\n\r\n";
    for _ in 0..2 {
        let _ = sock.send_to(msg.as_bytes(), "239.255.255.250:1900");
    }
    let mut locations = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(2500);
    let mut buf = [0u8; 2048];
    while Instant::now() < deadline {
        if let Ok((n, _)) = sock.recv_from(&mut buf) {
            let text = String::from_utf8_lossy(&buf[..n]);
            for line in text.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    if k.trim().eq_ignore_ascii_case("location") {
                        let v = v.trim().to_owned();
                        if !locations.contains(&v) {
                            locations.push(v);
                        }
                    }
                }
            }
        }
    }
    locations.iter().filter_map(|l| describe_dlna(l)).collect()
}

/// Contenu de la première balise `<name>` (sans tenir compte d'un préfixe d'espace de noms).
fn xml_value(xml: &str, name: &str) -> Option<String> {
    let open = xml.find(&format!("<{name}>"))?;
    let rest = &xml[open + name.len() + 2..];
    let close = rest.find(&format!("</{name}>"))?;
    Some(rest[..close].trim().to_owned())
}

fn absolute(base: &str, url: &str) -> String {
    if url.starts_with("http") {
        return url.to_owned();
    }
    let root = base.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
    if url.starts_with('/') {
        format!("{root}{url}")
    } else {
        format!("{root}/{url}")
    }
}

fn describe_dlna(location: &str) -> Option<Device> {
    let xml = ureq::get(location)
        .timeout(Duration::from_secs(3))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let base = xml_value(&xml, "URLBase").unwrap_or_else(|| location.to_owned());
    let mut av = None;
    let mut rendering = None;
    let mut rest = xml.as_str();
    while let Some(i) = rest.find("<service>") {
        let Some(end) = rest[i..].find("</service>") else {
            break;
        };
        let service = &rest[i..i + end];
        rest = &rest[i + end..];
        let kind = xml_value(service, "serviceType").unwrap_or_default();
        let control = xml_value(service, "controlURL").map(|c| absolute(&base, &c));
        if kind.contains("AVTransport") {
            av = control;
        } else if kind.contains("RenderingControl") {
            rendering = control;
        }
    }
    Some(Device {
        name: xml_value(&xml, "friendlyName").unwrap_or_else(|| location.to_owned()),
        model: xml_value(&xml, "modelName").unwrap_or_default(),
        kind: Kind::Dlna { av: av?, rendering },
    })
}

// mDNS : question PTR « _googlecast._tcp.local », réponse demandée en direct (bit QU).
fn discover_cast() -> Vec<Device> {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return Vec::new();
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
    let mut q = vec![0u8, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in ["_googlecast", "_tcp", "local"] {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 12, 0x80, 1]);
    for _ in 0..2 {
        let _ = sock.send_to(&q, "224.0.0.251:5353");
    }
    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(2500);
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        if let Ok((n, from)) = sock.recv_from(&mut buf) {
            if let Some(d) = parse_mdns(&buf[..n], from.ip()) {
                if !out.iter().any(|o: &Device| o.kind == d.kind) {
                    out.push(d);
                }
            }
        }
    }
    out
}

fn dns_name(pkt: &[u8], mut i: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut end = None;
    for _ in 0..64 {
        let len = *pkt.get(i)? as usize;
        if len == 0 {
            return Some((labels.join("."), end.unwrap_or(i + 1)));
        }
        if len & 0xC0 == 0xC0 {
            let ptr = ((len & 0x3F) << 8) | *pkt.get(i + 1)? as usize;
            end.get_or_insert(i + 2);
            i = ptr;
            continue;
        }
        labels.push(String::from_utf8_lossy(pkt.get(i + 1..i + 1 + len)?).into_owned());
        i += 1 + len;
    }
    None
}

fn parse_mdns(pkt: &[u8], from: IpAddr) -> Option<Device> {
    let count = |k: usize| u16::from_be_bytes([pkt[k], pkt[k + 1]]) as usize;
    if pkt.len() < 12 {
        return None;
    }
    let (qd, records) = (count(4), count(6) + count(8) + count(10));
    let mut i = 12;
    for _ in 0..qd {
        let (_, next) = dns_name(pkt, i)?;
        i = next + 4;
    }
    let (mut name, mut model, mut port, mut ip) = (None, String::new(), 8009u16, None);
    let mut is_cast = false;
    for _ in 0..records {
        let (rname, next) = dns_name(pkt, i)?;
        let rtype = u16::from_be_bytes([*pkt.get(next)?, *pkt.get(next + 1)?]);
        let len = u16::from_be_bytes([*pkt.get(next + 8)?, *pkt.get(next + 9)?]) as usize;
        let data = next + 10;
        let rdata = pkt.get(data..data + len)?;
        match rtype {
            12 if rname.starts_with("_googlecast") => is_cast = true,
            33 if len >= 6 => port = u16::from_be_bytes([rdata[4], rdata[5]]),
            1 if len == 4 => ip = Some(IpAddr::V4(Ipv4Addr::new(rdata[0], rdata[1], rdata[2], rdata[3]))),
            16 => {
                let mut k = 0;
                while k < rdata.len() {
                    let l = rdata[k] as usize;
                    let entry =
                        String::from_utf8_lossy(rdata.get(k + 1..k + 1 + l).unwrap_or(&[])).into_owned();
                    if let Some(v) = entry.strip_prefix("fn=") {
                        name = Some(v.to_owned());
                    } else if let Some(v) = entry.strip_prefix("md=") {
                        model = v.to_owned();
                    }
                    k += 1 + l;
                }
                if rname.contains("_googlecast") {
                    is_cast = true;
                }
            }
            _ => {}
        }
        i = data + len;
    }
    is_cast.then(|| Device {
        name: name.unwrap_or_else(|| "Chromecast".into()),
        model,
        kind: Kind::Chromecast {
            addr: SocketAddr::new(ip.unwrap_or(from), port),
        },
    })
}

// ---------- Serveur HTTP local ----------

#[derive(Clone)]
enum Entry {
    File(PathBuf, &'static str),
    /// Converti à la volée en MP3 : entrée ffmpeg, début, fin.
    Transcode(String, Duration, Option<Duration>),
}

struct Server {
    port: u16,
    entries: Mutex<HashMap<String, Entry>>,
}

fn server() -> Option<&'static Server> {
    static SERVER: OnceLock<Option<Server>> = OnceLock::new();
    SERVER
        .get_or_init(|| {
            let listener = TcpListener::bind("0.0.0.0:0").ok()?;
            let port = listener.local_addr().ok()?.port();
            std::thread::spawn(move || {
                for conn in listener.incoming().flatten() {
                    std::thread::spawn(move || {
                        let _ = serve(conn);
                    });
                }
            });
            Some(Server {
                port,
                entries: Mutex::new(HashMap::new()),
            })
        })
        .as_ref()
}

fn mime_of(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" | "aac" | "alac" => "audio/mp4",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => return None,
    })
}

fn serve(mut conn: TcpStream) -> std::io::Result<()> {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    let mut range: Option<u64> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                range = v
                    .trim()
                    .strip_prefix("bytes=")
                    .and_then(|r| r.split('-').next()?.parse().ok());
            }
        }
    }
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let token = path
        .trim_start_matches('/')
        .split(['.', '?'])
        .next()
        .unwrap_or("");
    let entry = server().and_then(|s| s.entries.lock().ok()?.get(token).cloned());
    let dlna = "transferMode.dlna.org: Streaming\r\ncontentFeatures.dlna.org: DLNA.ORG_OP=01;DLNA.ORG_CI=0;\
                DLNA.ORG_FLAGS=01700000000000000000000000000000\r\nAccess-Control-Allow-Origin: *\r\n";
    match entry {
        None => conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"),
        Some(Entry::File(file, mime)) => {
            let mut f = std::fs::File::open(&file)?;
            let len = f.metadata()?.len();
            let start = range.unwrap_or(0).min(len);
            let head = if range.is_some() {
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n\
                     Content-Range: bytes {start}-{}/{len}\r\nAccept-Ranges: bytes\r\n{dlna}Connection: close\r\n\r\n",
                    len - start,
                    len.saturating_sub(1)
                )
            } else {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {len}\r\nAccept-Ranges: bytes\r\n\
                     {dlna}Connection: close\r\n\r\n"
                )
            };
            conn.write_all(head.as_bytes())?;
            if method == "HEAD" {
                return Ok(());
            }
            std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(start))?;
            std::io::copy(&mut f, &mut conn).map(|_| ())
        }
        Some(Entry::Transcode(input, start, end)) => {
            let head =
                format!("HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\n{dlna}Connection: close\r\n\r\n");
            conn.write_all(head.as_bytes())?;
            if method == "HEAD" {
                return Ok(());
            }
            let mut cmd = std::process::Command::new("ffmpeg");
            cmd.args(["-nostdin", "-v", "error"]);
            if !start.is_zero() {
                cmd.args(["-ss", &format!("{:.3}", start.as_secs_f64())]);
            }
            if let Some(e) = end {
                cmd.args(["-to", &format!("{:.3}", e.as_secs_f64())]);
            }
            cmd.args([
                "-i",
                &input,
                "-vn",
                "-c:a",
                "libmp3lame",
                "-b:a",
                "320k",
                "-f",
                "mp3",
                "-",
            ]);
            let mut child = cmd
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            let mut out = child.stdout.take().unwrap();
            let r = std::io::copy(&mut out, &mut conn);
            let _ = child.kill();
            let _ = child.wait();
            r.map(|_| ())
        }
    }
}

/// Adresse de cette machine vue depuis `peer` (celle qu'on donne à l'appareil).
fn local_ip(peer: IpAddr) -> Option<IpAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect(SocketAddr::new(peer, 9)).ok()?;
    Some(sock.local_addr().ok()?.ip())
}

fn publish(entry: Entry, peer: IpAddr, ext: &str) -> Option<String> {
    let s = server()?;
    let token = format!("{:016x}", fastrand::u64(..));
    s.entries.lock().ok()?.insert(token.clone(), entry);
    Some(format!("http://{}:{}/{token}.{ext}", local_ip(peer)?, s.port))
}

// ---------- Session ----------

/// Ce qu'on montre sur l'appareil.
#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Pochette : un fichier local ou une adresse.
    pub cover_file: Option<PathBuf>,
    pub cover_url: Option<String>,
    /// Direct (webradio) : pas de durée, pas de déplacement.
    pub live: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum State {
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
    Finished,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub state: State,
    /// Position annoncée par l'appareil, et quand elle l'a été (on l'avance entre deux relevés).
    pub position: f32,
    pub at: Option<Instant>,
    pub error: Option<String>,
}

enum Cmd {
    Load {
        url: String,
        mime: String,
        meta: Meta,
        start: f32,
        paused: bool,
    },
    Play,
    Pause,
    Stop,
    Seek(f32),
    Volume(f32),
}

/// Une diffusion en cours vers un appareil.
pub struct Session {
    pub device: Device,
    tx: Sender<Cmd>,
    status: Arc<Mutex<Status>>,
    /// Média chargé, et décalage d'un flux converti relancé plus loin.
    media: Option<(Media, Meta)>,
    offset: f32,
    transcoded: bool,
    peer: IpAddr,
}

impl Session {
    pub fn start(device: Device) -> Result<Self, String> {
        let peer = device
            .host()
            .ok_or_else(|| crate::i18n::tr("Adresse de l'appareil inconnue").to_owned())?;
        server().ok_or_else(|| crate::i18n::tr("Serveur local impossible à démarrer").to_owned())?;
        let (tx, rx) = channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let st = status.clone();
        let kind = device.kind.clone();
        std::thread::spawn(move || match kind {
            Kind::Dlna { av, rendering } => dlna_worker(&av, rendering.as_deref(), rx, &st),
            Kind::Chromecast { addr } => {
                if let Err(e) = cast_worker(addr, rx, &st) {
                    if let Ok(mut s) = st.lock() {
                        s.error = Some(e);
                        s.state = State::Idle;
                    }
                }
            }
        });
        Ok(Self {
            device,
            tx,
            status,
            media: None,
            offset: 0.0,
            transcoded: false,
            peer,
        })
    }

    /// Ce que l'appareil peut lire tel quel ; le reste est converti en MP3.
    fn native(&self, media: &Media) -> bool {
        if !media.start.is_zero() || media.end.is_some() {
            return false;
        }
        let Some(path) = media.path() else { return true };
        match mime_of(path) {
            Some("audio/ogg") => self.device.is_chromecast(),
            Some(m) => m.starts_with("audio"),
            None => false,
        }
    }

    pub fn load(&mut self, media: &Media, meta: Meta, start: f32, paused: bool) {
        let transcode = !self.native(media);
        let (url, mime) = match (&media.input, transcode) {
            // Adresse en ligne (radio, serveur, podcast) : l'appareil la lit lui-même.
            (Input::Url(u), _) => (u.clone(), "audio/mpeg".to_owned()),
            (Input::File(p), false) => {
                let mime = mime_of(p).unwrap_or("audio/mpeg");
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("mp3").to_owned();
                match publish(Entry::File(p.clone(), mime), self.peer, &ext) {
                    Some(u) => (u, mime.to_owned()),
                    None => return,
                }
            }
            _ => {
                let at = media.start + Duration::from_secs_f32(start.max(0.0));
                match publish(
                    Entry::Transcode(media.ffmpeg_input(), at, media.end),
                    self.peer,
                    "mp3",
                ) {
                    Some(u) => (u, "audio/mpeg".to_owned()),
                    None => return,
                }
            }
        };
        let mut meta = meta;
        if meta.cover_url.is_none() {
            if let Some(f) = &meta.cover_file {
                let mime = mime_of(f).unwrap_or("image/png");
                meta.cover_url = publish(Entry::File(f.clone(), mime), self.peer, "png");
            }
        }
        // Un flux converti part déjà du bon endroit : l'appareil commence à 0.
        let transcoded = transcode && media.path().is_some();
        self.transcoded = transcoded;
        self.offset = if transcoded { start } else { 0.0 };
        let device_start = if transcoded { 0.0 } else { start };
        self.media = Some((media.clone(), meta.clone()));
        if let Ok(mut s) = self.status.lock() {
            *s = Status {
                state: State::Loading,
                ..Default::default()
            };
        }
        let _ = self.tx.send(Cmd::Load {
            url,
            mime,
            meta,
            start: device_start,
            paused,
        });
    }

    pub fn play(&self) {
        let _ = self.tx.send(Cmd::Play);
    }

    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }

    pub fn stop(&mut self) {
        self.media = None;
        let _ = self.tx.send(Cmd::Stop);
        if let Ok(mut s) = self.status.lock() {
            *s = Status::default();
        }
    }

    pub fn seek(&mut self, pos: f32) {
        if self.transcoded {
            // Flux converti : relancé au nouvel endroit.
            if let Some((media, meta)) = self.media.clone() {
                let paused = self.state() == State::Paused;
                self.load(&media, meta, pos, paused);
            }
            return;
        }
        let _ = self.tx.send(Cmd::Seek(pos));
    }

    pub fn set_volume(&self, v: f32) {
        let _ = self.tx.send(Cmd::Volume(v));
    }

    pub fn state(&self) -> State {
        self.status.lock().map(|s| s.state).unwrap_or_default()
    }

    pub fn has_media(&self) -> bool {
        self.media.is_some()
    }

    pub fn is_live(&self) -> bool {
        self.media.as_ref().is_some_and(|(_, m)| m.live)
    }

    pub fn position(&self) -> f32 {
        let s = self.status.lock().map(|s| s.clone()).unwrap_or_default();
        let running = match (s.state, s.at) {
            (State::Playing, Some(at)) => at.elapsed().as_secs_f32(),
            _ => 0.0,
        };
        self.offset + s.position + running
    }

    pub fn error(&self) -> Option<String> {
        self.status.lock().ok().and_then(|s| s.error.clone())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
    }
}

// ---------- DLNA ----------

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn soap(url: &str, service: &str, action: &str, args: &str) -> Result<String, String> {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} \
         xmlns:u=\"urn:schemas-upnp-org:service:{service}:1\">{args}</u:{action}></s:Body></s:Envelope>"
    );
    ureq::post(url)
        .timeout(Duration::from_secs(5))
        .set("Content-Type", "text/xml; charset=\"utf-8\"")
        .set(
            "SOAPAction",
            &format!("\"urn:schemas-upnp-org:service:{service}:1#{action}\""),
        )
        .send_string(&body)
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())
}

fn hms(secs: f32) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn parse_hms(s: &str) -> f32 {
    s.split(':')
        .filter_map(|p| p.split('.').next()?.parse::<f32>().ok())
        .fold(0.0, |acc, p| acc * 60.0 + p)
}

fn dlna_worker(av: &str, rendering: Option<&str>, rx: Receiver<Cmd>, status: &Mutex<Status>) {
    let set = |f: &dyn Fn(&mut Status)| {
        if let Ok(mut s) = status.lock() {
            f(&mut s);
        }
    };
    let mut loaded = false;
    let mut was_playing = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(1000)) {
            Ok(Cmd::Load {
                url,
                mime,
                meta,
                start,
                paused,
            }) => {
                let mut didl = format!(
                    "<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" \
                     xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\">\
                     <item id=\"1\" parentID=\"0\" restricted=\"1\"><dc:title>{}</dc:title><upnp:artist>{}</upnp:artist>\
                     <upnp:album>{}</upnp:album><upnp:class>object.item.audioItem.musicTrack</upnp:class>",
                    xml_escape(&meta.title),
                    xml_escape(&meta.artist),
                    xml_escape(&meta.album)
                );
                if let Some(c) = &meta.cover_url {
                    didl.push_str(&format!("<upnp:albumArtURI>{}</upnp:albumArtURI>", xml_escape(c)));
                }
                didl.push_str(&format!(
                    "<res protocolInfo=\"http-get:*:{mime}:*\">{}</res></item></DIDL-Lite>",
                    xml_escape(&url)
                ));
                let _ = soap(av, "AVTransport", "Stop", "<InstanceID>0</InstanceID>");
                let r = soap(
                    av,
                    "AVTransport",
                    "SetAVTransportURI",
                    &format!(
                        "<InstanceID>0</InstanceID><CurrentURI>{}</CurrentURI><CurrentURIMetaData>{}</CurrentURIMetaData>",
                        xml_escape(&url),
                        xml_escape(&didl)
                    ),
                );
                if let Err(e) = r {
                    set(&|s| s.error = Some(e.clone()));
                    continue;
                }
                if !paused {
                    let _ = soap(
                        av,
                        "AVTransport",
                        "Play",
                        "<InstanceID>0</InstanceID><Speed>1</Speed>",
                    );
                }
                if start > 1.0 {
                    std::thread::sleep(Duration::from_millis(800));
                    let _ = soap(
                        av,
                        "AVTransport",
                        "Seek",
                        &format!(
                            "<InstanceID>0</InstanceID><Unit>REL_TIME</Unit><Target>{}</Target>",
                            hms(start)
                        ),
                    );
                }
                loaded = true;
                was_playing = false;
            }
            Ok(Cmd::Play) => {
                let _ = soap(
                    av,
                    "AVTransport",
                    "Play",
                    "<InstanceID>0</InstanceID><Speed>1</Speed>",
                );
            }
            Ok(Cmd::Pause) => {
                let _ = soap(av, "AVTransport", "Pause", "<InstanceID>0</InstanceID>");
            }
            Ok(Cmd::Seek(p)) => {
                let _ = soap(
                    av,
                    "AVTransport",
                    "Seek",
                    &format!(
                        "<InstanceID>0</InstanceID><Unit>REL_TIME</Unit><Target>{}</Target>",
                        hms(p)
                    ),
                );
            }
            Ok(Cmd::Volume(v)) => {
                if let Some(rc) = rendering {
                    let _ = soap(
                        rc,
                        "RenderingControl",
                        "SetVolume",
                        &format!(
                            "<InstanceID>0</InstanceID><Channel>Master</Channel><DesiredVolume>{}</DesiredVolume>",
                            (v.clamp(0.0, 1.0) * 100.0).round()
                        ),
                    );
                }
            }
            Ok(Cmd::Stop) => {
                let _ = soap(av, "AVTransport", "Stop", "<InstanceID>0</InstanceID>");
                loaded = false;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        if !loaded {
            continue;
        }
        // Relevé de l'état et de la position.
        let state = soap(
            av,
            "AVTransport",
            "GetTransportInfo",
            "<InstanceID>0</InstanceID>",
        )
        .ok()
        .and_then(|x| xml_value(&x, "CurrentTransportState"))
        .unwrap_or_default();
        let pos = soap(av, "AVTransport", "GetPositionInfo", "<InstanceID>0</InstanceID>")
            .ok()
            .and_then(|x| xml_value(&x, "RelTime"))
            .map(|t| parse_hms(&t))
            .unwrap_or(0.0);
        let st = match state.as_str() {
            "PLAYING" => {
                was_playing = true;
                State::Playing
            }
            "PAUSED_PLAYBACK" => State::Paused,
            "TRANSITIONING" => State::Loading,
            // Arrêt après avoir joué : le morceau est fini.
            "STOPPED" | "NO_MEDIA_PRESENT" if was_playing => {
                loaded = false;
                State::Finished
            }
            _ => State::Loading,
        };
        set(&|s| {
            s.state = st;
            s.position = pos;
            s.at = Some(Instant::now());
        });
    }
}

// ---------- Chromecast (Cast v2) ----------

#[derive(Debug)]
struct AcceptAny;

impl rustls::client::danger::ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        // Les Chromecast ont un certificat qu'elles signent elles-mêmes : réseau local.
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

const NS_CONN: &str = "urn:x-cast:com.google.cast.tp.connection";
const NS_HEART: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
const NS_RECV: &str = "urn:x-cast:com.google.cast.receiver";
const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
const DEFAULT_RECEIVER: &str = "CC1AD845";

fn varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// Message Cast : protobuf `CastMessage` (version, source, destination, espace, texte JSON).
fn encode(src: &str, dst: &str, ns: &str, json: &str) -> Vec<u8> {
    let mut m = vec![0x08, 0x00];
    for (tag, s) in [(0x12u8, src), (0x1a, dst), (0x22, ns)] {
        m.push(tag);
        varint(&mut m, s.len() as u64);
        m.extend_from_slice(s.as_bytes());
    }
    m.extend_from_slice(&[0x28, 0x00, 0x32]);
    varint(&mut m, json.len() as u64);
    m.extend_from_slice(json.as_bytes());
    let mut frame = (m.len() as u32).to_be_bytes().to_vec();
    frame.extend(m);
    frame
}

/// Espace de noms et JSON d'un message reçu.
fn decode(m: &[u8]) -> Option<(String, String)> {
    let mut i = 0;
    let (mut ns, mut payload) = (String::new(), String::new());
    while i < m.len() {
        let key = m[i];
        i += 1;
        let wire = key & 7;
        let mut v: u64 = 0;
        let mut shift = 0;
        loop {
            let b = *m.get(i)?;
            i += 1;
            v |= ((b & 0x7F) as u64) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        if wire == 2 {
            let s = String::from_utf8_lossy(m.get(i..i + v as usize)?).into_owned();
            i += v as usize;
            match key >> 3 {
                4 => ns = s,
                6 => payload = s,
                _ => {}
            }
        }
    }
    Some((ns, payload))
}

fn cast_worker(addr: SocketAddr, rx: Receiver<Cmd>, status: &Mutex<Status>) -> Result<(), String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny))
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::IpAddress(addr.ip().into());
    let conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| e.to_string())?;
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| e.to_string())?;
    tcp.set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|e| e.to_string())?;
    let mut tls = rustls::StreamOwned::new(conn, tcp);
    let send = |tls: &mut rustls::StreamOwned<rustls::ClientConnection, TcpStream>,
                dst: &str,
                ns: &str,
                json: String| {
        let _ = tls.write_all(&encode("sender-0", dst, ns, &json));
        let _ = tls.flush();
    };
    send(&mut tls, "receiver-0", NS_CONN, r#"{"type":"CONNECT"}"#.into());
    let mut request = 1u64;
    let mut transport: Option<String> = None;
    let mut session: Option<i64> = None;
    let mut pending: Option<Cmd> = None;
    let mut buf: Vec<u8> = Vec::new();
    let mut last_ping = Instant::now();
    let mut last_poll = Instant::now();
    loop {
        // Commandes de l'application.
        match rx.try_recv() {
            Ok(cmd) => pending = Some(cmd),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
            Err(_) => {}
        }
        if let Some(cmd) = pending.take() {
            request += 1;
            match cmd {
                Cmd::Load { .. } if transport.is_none() => {
                    // Le récepteur multimédia se lance d'abord ; le chargement attend sa réponse.
                    send(
                        &mut tls,
                        "receiver-0",
                        NS_RECV,
                        format!(r#"{{"type":"LAUNCH","appId":"{DEFAULT_RECEIVER}","requestId":{request}}}"#),
                    );
                    pending = Some(cmd);
                }
                Cmd::Load {
                    url,
                    mime,
                    meta,
                    start,
                    paused,
                } => {
                    let t = transport.clone().unwrap();
                    let mut images = Vec::new();
                    if let Some(c) = &meta.cover_url {
                        images.push(serde_json::json!({ "url": c }));
                    }
                    let load = serde_json::json!({
                        "type": "LOAD", "requestId": request, "autoplay": !paused, "currentTime": start,
                        "media": {
                            "contentId": url, "contentType": mime,
                            "streamType": if meta.live { "LIVE" } else { "BUFFERED" },
                            "metadata": { "metadataType": 3, "title": meta.title, "artist": meta.artist,
                                          "albumName": meta.album, "images": images }
                        }
                    });
                    send(&mut tls, &t, NS_MEDIA, load.to_string());
                    session = None;
                }
                Cmd::Volume(v) => send(
                    &mut tls,
                    "receiver-0",
                    NS_RECV,
                    format!(
                        r#"{{"type":"SET_VOLUME","volume":{{"level":{}}},"requestId":{request}}}"#,
                        v.clamp(0.0, 1.0)
                    ),
                ),
                other => {
                    if let (Some(t), Some(s)) = (&transport, session) {
                        let json = match other {
                            Cmd::Play => {
                                format!(r#"{{"type":"PLAY","mediaSessionId":{s},"requestId":{request}}}"#)
                            }
                            Cmd::Pause => {
                                format!(r#"{{"type":"PAUSE","mediaSessionId":{s},"requestId":{request}}}"#)
                            }
                            Cmd::Stop => {
                                format!(r#"{{"type":"STOP","mediaSessionId":{s},"requestId":{request}}}"#)
                            }
                            Cmd::Seek(p) => format!(
                                r#"{{"type":"SEEK","mediaSessionId":{s},"currentTime":{p},"requestId":{request}}}"#
                            ),
                            _ => String::new(),
                        };
                        let t = t.clone();
                        send(&mut tls, &t, NS_MEDIA, json);
                    }
                }
            }
        }
        if last_ping.elapsed() > Duration::from_secs(5) {
            last_ping = Instant::now();
            send(&mut tls, "receiver-0", NS_HEART, r#"{"type":"PING"}"#.into());
        }
        if last_poll.elapsed() > Duration::from_secs(1) {
            last_poll = Instant::now();
            if let Some(t) = transport.clone() {
                request += 1;
                send(
                    &mut tls,
                    &t,
                    NS_MEDIA,
                    format!(r#"{{"type":"GET_STATUS","requestId":{request}}}"#),
                );
            }
        }
        // Messages de l'appareil.
        let mut chunk = [0u8; 8192];
        match tls.read(&mut chunk) {
            Ok(0) => return Err(crate::i18n::tr("L'appareil a fermé la connexion").into()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.to_string()),
        }
        while buf.len() >= 4 {
            let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
            if buf.len() < 4 + len {
                break;
            }
            let msg: Vec<u8> = buf.drain(..4 + len).skip(4).collect();
            let Some((ns, payload)) = decode(&msg) else {
                continue;
            };
            let v: serde_json::Value = serde_json::from_str(&payload).unwrap_or_default();
            match (ns.as_str(), v["type"].as_str().unwrap_or("")) {
                (NS_HEART, "PING") => send(&mut tls, "receiver-0", NS_HEART, r#"{"type":"PONG"}"#.into()),
                (NS_RECV, "RECEIVER_STATUS") => {
                    let app = v["status"]["applications"]
                        .as_array()
                        .and_then(|a| a.iter().find(|a| a["appId"] == DEFAULT_RECEIVER));
                    match app.and_then(|a| a["transportId"].as_str()) {
                        Some(t) if transport.as_deref() != Some(t) => {
                            transport = Some(t.to_owned());
                            send(&mut tls, t, NS_CONN, r#"{"type":"CONNECT"}"#.into());
                        }
                        Some(_) => {}
                        None => {
                            transport = None;
                            session = None;
                        }
                    }
                }
                (NS_MEDIA, "MEDIA_STATUS") => {
                    let Some(st) = v["status"].as_array().and_then(|s| s.first()) else {
                        continue;
                    };
                    if let Some(id) = st["mediaSessionId"].as_i64() {
                        session = Some(id);
                    }
                    let state = match st["playerState"].as_str().unwrap_or("") {
                        "PLAYING" => State::Playing,
                        "PAUSED" => State::Paused,
                        "BUFFERING" => State::Loading,
                        "IDLE" if st["idleReason"] == "FINISHED" => State::Finished,
                        _ => State::Loading,
                    };
                    let pos = st["currentTime"].as_f64().unwrap_or(0.0) as f32;
                    let error = (st["idleReason"] == "ERROR")
                        .then(|| crate::i18n::tr("L'appareil n'a pas pu lire ce morceau").to_owned());
                    if let Ok(mut s) = status.lock() {
                        s.state = state;
                        s.position = pos;
                        s.at = Some(Instant::now());
                        if error.is_some() {
                            s.error = error;
                        }
                    }
                }
                (NS_MEDIA, "LOAD_FAILED") | (NS_MEDIA, "INVALID_REQUEST") => {
                    if let Ok(mut s) = status.lock() {
                        s.error = Some(crate::i18n::tr("L'appareil n'a pas pu lire ce morceau").into());
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cast_messages() {
        let frame = encode("sender-0", "receiver-0", NS_HEART, r#"{"type":"PING"}"#);
        let len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert_eq!(len, frame.len() - 4);
        let (ns, payload) = decode(&frame[4..]).unwrap();
        assert_eq!(ns, NS_HEART);
        assert_eq!(payload, r#"{"type":"PING"}"#);
    }

    #[test]
    fn mdns_answer() {
        // Réponse fabriquée : PTR, TXT (fn=Salon) et A (192.168.1.20).
        let mut p = vec![0, 0, 0x84, 0, 0, 0, 0, 3, 0, 0, 0, 0];
        let name = |p: &mut Vec<u8>, parts: &[&str]| {
            for l in parts {
                p.push(l.len() as u8);
                p.extend_from_slice(l.as_bytes());
            }
            p.push(0);
        };
        name(&mut p, &["_googlecast", "_tcp", "local"]);
        p.extend_from_slice(&[0, 12, 0, 1, 0, 0, 0, 120, 0, 2, 0xC0, 12]);
        name(&mut p, &["Salon", "_googlecast", "_tcp", "local"]);
        let txt = b"\x08fn=Salon\x0bmd=Nest Hub";
        p.extend_from_slice(&[0, 16, 0, 1, 0, 0, 0, 120, 0, txt.len() as u8]);
        p.extend_from_slice(txt);
        name(&mut p, &["salon", "local"]);
        p.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 120, 0, 4, 192, 168, 1, 20]);
        let d = parse_mdns(&p, "10.0.0.1".parse().unwrap()).unwrap();
        assert_eq!(d.name, "Salon");
        assert_eq!(d.model, "Nest Hub");
        assert_eq!(
            d.kind,
            Kind::Chromecast {
                addr: "192.168.1.20:8009".parse().unwrap()
            }
        );
    }

    #[test]
    fn dlna_helpers() {
        assert_eq!(hms(3725.0), "1:02:05");
        assert_eq!(parse_hms("0:03:20.500"), 200.0);
        assert_eq!(
            absolute("http://192.168.1.5:8080/desc.xml", "/ctl/av"),
            "http://192.168.1.5:8080/ctl/av"
        );
        assert_eq!(
            absolute("http://192.168.1.5:8080/", "ctl/av"),
            "http://192.168.1.5:8080/ctl/av"
        );
    }

    /// Réseau local : `cargo test cast_discovery -- --ignored --nocapture` liste les appareils.
    #[test]
    #[ignore]
    fn cast_discovery() {
        for d in discover() {
            eprintln!("{} ({}) : {:?}", d.name, d.model, d.kind);
        }
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Réseau local : connexion TLS à un Chromecast et état du récepteur, sans rien lancer.
    /// `STERMUSIC_CAST=192.168.1.128 cargo test cast_handshake -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn cast_handshake() {
        let ip: IpAddr = std::env::var("STERMUSIC_CAST").unwrap().parse().unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny))
            .with_no_client_auth();
        let conn = rustls::ClientConnection::new(
            Arc::new(config),
            rustls::pki_types::ServerName::IpAddress(ip.into()),
        )
        .unwrap();
        let tcp = TcpStream::connect_timeout(&SocketAddr::new(ip, 8009), Duration::from_secs(5)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut tls = rustls::StreamOwned::new(conn, tcp);
        tls.write_all(&encode(
            "sender-0",
            "receiver-0",
            NS_CONN,
            r#"{"type":"CONNECT"}"#,
        ))
        .unwrap();
        tls.write_all(&encode(
            "sender-0",
            "receiver-0",
            NS_RECV,
            r#"{"type":"GET_STATUS","requestId":1}"#,
        ))
        .unwrap();
        let mut buf = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let mut chunk = [0u8; 4096];
            if let Ok(n) = tls.read(&mut chunk) {
                buf.extend_from_slice(&chunk[..n]);
            }
            while buf.len() >= 4 {
                let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
                if buf.len() < 4 + len {
                    break;
                }
                let msg: Vec<u8> = buf.drain(..4 + len).skip(4).collect();
                let (ns, payload) = decode(&msg).unwrap();
                eprintln!("{ns} : {}", &payload[..payload.len().min(160)]);
                if payload.contains("RECEIVER_STATUS") {
                    return;
                }
            }
        }
        panic!("pas de RECEIVER_STATUS");
    }
}
