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
    /// Code du pays (« FR ») et région, pour le globe.
    pub countrycode: String,
    pub state: String,
    /// Latitude et longitude (degrés), si l'annuaire les connaît.
    pub geo: Option<[f32; 2]>,
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
            parts.insert(0, country_name(&self.country));
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

#[derive(Default, Deserialize)]
#[serde(default)]
struct ApiStation {
    stationuuid: String,
    name: String,
    url: String,
    url_resolved: String,
    homepage: String,
    favicon: String,
    tags: String,
    country: String,
    countrycode: String,
    state: String,
    codec: String,
    bitrate: u32,
    hls: u8,
    geo_lat: Option<f64>,
    geo_long: Option<f64>,
}

impl ApiStation {
    fn into_station(self) -> Option<Station> {
        if !playable(&self.codec, self.hls != 0) {
            return None;
        }
        let url = if self.url_resolved.is_empty() {
            self.url
        } else {
            self.url_resolved
        };
        let name = self.name.trim().to_owned();
        if name.is_empty() || url.is_empty() {
            return None;
        }
        let geo = match (self.geo_lat, self.geo_long) {
            (Some(lat), Some(lon))
                if lat.abs() <= 90.0 && lon.abs() <= 180.0 && (lat != 0.0 || lon != 0.0) =>
            {
                Some([lat as f32, lon as f32])
            }
            _ => None,
        };
        Some(Station {
            name,
            url,
            favicon: self.favicon,
            country: self.country,
            countrycode: self.countrycode,
            state: self.state.trim().to_owned(),
            tags: self.tags,
            codec: self.codec,
            bitrate: self.bitrate,
            homepage: self.homepage,
            uuid: self.stationuuid,
            geo,
        })
    }
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

/// Ce qu'on demande à l'annuaire : les filtres se combinent ; sans aucun, les stations les plus
/// écoutées du monde.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    /// Morceau du nom de la station.
    pub name: String,
    /// Genre (étiquette de l'annuaire : « jazz »).
    pub tag: String,
    /// Pays (code ISO : « FR »).
    pub country: String,
}

/// Appel à l'annuaire, sur le premier serveur qui répond.
fn directory_get(path: &str, params: &[(&str, &str)], timeout: Duration) -> Result<ureq::Response, String> {
    let agent = agent(timeout);
    let mut last_error = String::new();
    for server in DIRECTORY {
        let mut req = agent.get(&format!("{server}{path}"));
        for (k, v) in params {
            req = req.query(k, v);
        }
        match req.call() {
            Ok(resp) => return Ok(resp),
            Err(e) => last_error = e.to_string(),
        }
    }
    Err(last_error)
}

/// Interroge l'annuaire (bloquant : à appeler depuis un thread).
pub fn search(query: &Query) -> Result<Vec<Station>, String> {
    let mut params = vec![
        ("limit", "100"),
        ("hidebroken", "true"),
        ("order", "clickcount"),
        ("reverse", "true"),
    ];
    if !query.name.is_empty() {
        params.push(("name", &query.name));
    }
    if !query.tag.is_empty() {
        params.push(("tag", &query.tag));
    }
    if !query.country.is_empty() {
        params.push(("countrycode", &query.country));
    }
    let resp = directory_get("/json/stations/search", &params, Duration::from_secs(12))?;
    let list: Vec<ApiStation> = serde_json::from_reader(resp.into_reader()).map_err(|e| e.to_string())?;
    Ok(list.into_iter().filter_map(ApiStation::into_station).collect())
}

/// Un pays de l'annuaire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Country {
    pub code: String,
    pub name: String,
    pub stations: u32,
}

/// Les pays qui ont des stations, par ordre alphabétique (bloquant).
pub fn countries() -> Result<Vec<Country>, String> {
    #[derive(Deserialize)]
    struct Api {
        #[serde(default)]
        name: String,
        #[serde(default)]
        iso_3166_1: String,
        #[serde(default)]
        stationcount: u32,
    }
    let resp = directory_get("/json/countries", &[], Duration::from_secs(12))?;
    let list: Vec<Api> = serde_json::from_reader(resp.into_reader()).map_err(|e| e.to_string())?;
    let mut out: Vec<Country> = list
        .into_iter()
        .filter(|c| c.stationcount > 0 && c.iso_3166_1.len() == 2)
        .map(|c| Country {
            code: c.iso_3166_1.to_ascii_uppercase(),
            name: country_name(&c.name),
            stations: c.stationcount,
        })
        .collect();
    out.sort_by_cached_key(|c| crate::library::fold(&c.name));
    Ok(out)
}

/// « The United States Of America » → « United States Of America » : l'article gêne le tri.
fn country_name(name: &str) -> String {
    let name = name.strip_prefix("The ").unwrap_or(name).trim();
    // L'annuaire donne les noms officiels, parfois à rallonge.
    let short = match name {
        "United Kingdom Of Great Britain And Northern Ireland" => "United Kingdom",
        "United States Of America" => "United States",
        "Russian Federation" => "Russia",
        "Republic Of Korea" => "South Korea",
        "Democratic People's Republic Of Korea" => "North Korea",
        "Iran (Islamic Republic Of)" | "Islamic Republic Of Iran" => "Iran",
        "Bolivarian Republic Of Venezuela" | "Venezuela (Bolivarian Republic Of)" => "Venezuela",
        "Plurinational State Of Bolivia" | "Bolivia (Plurinational State Of)" => "Bolivia",
        "Republic Of Moldova" => "Moldova",
        "United Republic Of Tanzania" => "Tanzania",
        "Syrian Arab Republic" => "Syria",
        "Lao People's Democratic Republic" => "Laos",
        "Viet Nam" => "Vietnam",
        "Taiwan, Republic Of China" => "Taiwan",
        "Netherlands (Kingdom Of The)" => "Netherlands",
        other => other,
    };
    short.to_owned()
}

// ---------- Stations du monde (globe) ----------

/// Les stations géolocalisées du monde entier, gardées une semaine sur le disque.
const WORLD_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

fn world_file() -> std::path::PathBuf {
    crate::library::cache_dir().join("radio-world.json")
}

/// Stations du cache, et vrai s'il est encore frais ; `None` s'il n'y en a pas.
pub fn load_world() -> Option<(Vec<Station>, bool)> {
    let file = world_file();
    let fresh = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < WORLD_MAX_AGE);
    let list: Vec<Station> = serde_json::from_slice(&std::fs::read(&file).ok()?).ok()?;
    (!list.is_empty()).then_some((list, fresh))
}

/// Télécharge toutes les stations géolocalisées (une dizaine de mégaoctets) et les garde en cache.
pub fn fetch_world() -> Result<Vec<Station>, String> {
    let params = [
        ("has_geo_info", "true"),
        ("hidebroken", "true"),
        ("order", "clickcount"),
        ("reverse", "true"),
        ("limit", "100000"),
    ];
    let resp = directory_get("/json/stations/search", &params, Duration::from_secs(60))?;
    let list: Vec<ApiStation> = serde_json::from_reader(resp.into_reader()).map_err(|e| e.to_string())?;
    let stations: Vec<Station> = list
        .into_iter()
        .filter_map(ApiStation::into_station)
        .filter(|s| s.geo.is_some())
        .collect();
    if stations.is_empty() {
        return Err(crate::i18n::tr("Réponse de l'annuaire vide").into());
    }
    let _ = std::fs::create_dir_all(crate::library::cache_dir());
    if let Ok(json) = serde_json::to_vec(&stations) {
        let tmp = world_file().with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, world_file());
        }
    }
    Ok(stations)
}

/// Un lieu du globe : une ville et ses stations (ou, loin de toute ville connue, une zone d'un
/// demi-degré).
#[derive(Clone, Debug)]
pub struct Place {
    pub lat: f32,
    pub lon: f32,
    /// La ville (« Lorient »), ou la région quand il n'y en a pas.
    pub name: String,
    /// La région et le pays (« Bretagne, France »), ou le pays seul.
    pub region: String,
    /// Indices dans la liste des stations du monde, les plus écoutées d'abord.
    pub stations: Vec<usize>,
}

impl Place {
    /// « Lorient, France » : pour une bulle d'aide.
    pub fn full_name(&self) -> String {
        if self.region.is_empty() || self.region == self.name {
            self.name.clone()
        } else {
            let country = self.region.rsplit(", ").next().unwrap_or(&self.region);
            format!("{}, {country}", self.name)
        }
    }
}

// ---------- Villes (GeoNames) ----------

/// Villes de plus de 15 000 habitants (GeoNames, CC BY 4.0) : nom, latitude, longitude, pays,
/// population.
const CITIES_TSV: &str = include_str!("../assets/cities.tsv");

struct City {
    name: &'static str,
    lat: f32,
    lon: f32,
    population: u32,
}

struct Cities {
    list: Vec<City>,
    /// Villes rangées par case d'un degré, pour chercher seulement autour d'un point.
    grid: std::collections::HashMap<(i32, i32), Vec<u32>>,
}

/// Distance maximale entre une station et la ville à laquelle elle est rattachée (km).
const CITY_RADIUS_KM: f32 = 40.0;

fn cities() -> &'static Cities {
    static CITIES: std::sync::OnceLock<Cities> = std::sync::OnceLock::new();
    CITIES.get_or_init(|| {
        let mut list = Vec::new();
        let mut grid: std::collections::HashMap<(i32, i32), Vec<u32>> = Default::default();
        for line in CITIES_TSV.lines() {
            let mut f = line.split('\t');
            let (Some(name), Some(lat), Some(lon), _, Some(pop)) =
                (f.next(), f.next(), f.next(), f.next(), f.next())
            else {
                continue;
            };
            let (Ok(lat), Ok(lon)) = (lat.parse::<f32>(), lon.parse::<f32>()) else {
                continue;
            };
            grid.entry((lat.floor() as i32, lon.floor() as i32))
                .or_default()
                .push(list.len() as u32);
            list.push(City {
                name,
                lat,
                lon,
                population: pop.parse().unwrap_or(0),
            });
        }
        Cities { list, grid }
    })
}

impl Cities {
    /// La ville à laquelle rattacher un point : la plus proche, les grandes villes comptant un
    /// peu plus (une station « de Paris » placée en banlieue reste à Paris).
    fn nearest(&self, lat: f32, lon: f32) -> Option<usize> {
        let (cy, cx) = (lat.floor() as i32, lon.floor() as i32);
        let reach = if lat.abs() > 60.0 { 2 } else { 1 };
        let coslat = lat.to_radians().cos().max(0.1);
        let mut best: Option<(usize, f32)> = None;
        for dy in -1..=1 {
            for dx in -reach..=reach {
                let Some(cell) = self.grid.get(&(cy + dy, cx + dx)) else {
                    continue;
                };
                for &k in cell {
                    let c = &self.list[k as usize];
                    let km = 111.2 * ((c.lat - lat).powi(2) + ((c.lon - lon) * coslat).powi(2)).sqrt();
                    if km > CITY_RADIUS_KM {
                        continue;
                    }
                    let weight = 1.0 + 0.3 * (c.population.max(15_000) as f32 / 15_000.0).ln();
                    let score = km / weight;
                    if best.is_none_or(|(_, b)| score < b) {
                        best = Some((k as usize, score));
                    }
                }
            }
        }
        best.map(|(k, _)| k)
    }
}

/// « Bretagne, France » : la région et le pays le plus souvent donnés par ces stations.
fn region_of(world: &[Station], stations: &[usize]) -> String {
    let mut names: std::collections::HashMap<String, usize> = Default::default();
    for &i in stations {
        let s = &world[i];
        let country = country_name(&s.country);
        let name = if s.state.is_empty() || s.state.eq_ignore_ascii_case(&country) {
            country
        } else {
            format!("{}, {country}", s.state)
        };
        *names.entry(name).or_default() += 1;
    }
    names
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(n, _)| n)
        .unwrap_or_default()
}

/// Regroupe les stations en lieux : par ville, sinon par zone d'un demi-degré.
pub fn places(world: &[Station]) -> Vec<Place> {
    #[derive(Hash, PartialEq, Eq)]
    enum Key {
        City(usize),
        Cell(i32, i32),
    }
    let cities = cities();
    let mut groups: std::collections::HashMap<Key, Vec<usize>> = Default::default();
    for (i, s) in world.iter().enumerate() {
        if let Some([lat, lon]) = s.geo {
            let key = match cities.nearest(lat, lon) {
                Some(c) => Key::City(c),
                None => Key::Cell((lat * 2.0).floor() as i32, (lon * 2.0).floor() as i32),
            };
            groups.entry(key).or_default().push(i);
        }
    }
    let mut out: Vec<Place> = groups
        .into_iter()
        .map(|(key, stations)| {
            let region = region_of(world, &stations);
            match key {
                Key::City(c) => {
                    let city = &cities.list[c];
                    Place {
                        lat: city.lat,
                        lon: city.lon,
                        name: city.name.to_owned(),
                        region,
                        stations,
                    }
                }
                Key::Cell(..) => {
                    let n = stations.len() as f32;
                    Place {
                        lat: stations.iter().map(|&i| world[i].geo.unwrap()[0]).sum::<f32>() / n,
                        lon: stations.iter().map(|&i| world[i].geo.unwrap()[1]).sum::<f32>() / n,
                        name: region.clone(),
                        region,
                        stations,
                    }
                }
            }
        })
        .collect();
    // Les gros lieux en dernier : dessinés par-dessus les petits.
    out.sort_by_key(|p| p.stations.len());
    out
}

/// Centre approximatif d'un pays (moyenne de ses stations), pour y tourner le globe.
pub fn country_center(world: &[Station], code: &str) -> Option<(f32, f32)> {
    let pts: Vec<[f32; 2]> = world
        .iter()
        .filter(|s| s.countrycode.eq_ignore_ascii_case(code))
        .filter_map(|s| s.geo)
        .collect();
    if pts.is_empty() {
        return None;
    }
    // Médiane plutôt que moyenne : quelques territoires lointains ne déplacent pas le centre.
    let mut lats: Vec<f32> = pts.iter().map(|p| p[0]).collect();
    let mut lons: Vec<f32> = pts.iter().map(|p| p[1]).collect();
    lats.sort_by(f32::total_cmp);
    lons.sort_by(f32::total_cmp);
    Some((lats[lats.len() / 2], lons[lons.len() / 2]))
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
            let query = Query {
                tag: tag.into(),
                ..Default::default()
            };
            let list = search(&query).expect("annuaire");
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
    fn world_places() {
        let st = |name: &str, cc: &str, state: &str, lat: f32, lon: f32| Station {
            name: name.into(),
            country: if cc == "FR" {
                "France".into()
            } else {
                "The United Kingdom".into()
            },
            countrycode: cc.into(),
            state: state.into(),
            geo: Some([lat, lon]),
            ..Default::default()
        };
        let world = vec![
            st("A", "FR", "Île-de-France", 48.86, 2.35),
            st("B", "FR", "Île-de-France", 48.85, 2.33),
            st("C", "FR", "", 43.30, 5.37),
            st("D", "GB", "", 51.50, -0.12),
        ];
        let places = places(&world);
        let names: Vec<String> = places.iter().map(|p| p.full_name()).collect();
        assert_eq!(places.len(), 3, "{names:?}");
        // Deux stations de la région parisienne : un seul lieu, Paris.
        let paris = places.iter().find(|p| p.stations.len() == 2).unwrap();
        assert_eq!(paris.name, "Paris");
        assert_eq!(paris.region, "Île-de-France, France");
        assert_eq!(paris.full_name(), "Paris, France");
        assert!(places.iter().any(|p| p.name == "Marseille"));
        assert!(places
            .iter()
            .any(|p| p.name == "London" && p.full_name() == "London, United Kingdom"));
        assert_eq!(
            country_name("The United Kingdom Of Great Britain And Northern Ireland"),
            "United Kingdom"
        );
        // Loin de toute ville : une zone, nommée d'après son pays.
        let sea = super::places(&[st("E", "FR", "", 46.0, -12.0)]);
        assert_eq!(sea[0].name, "France");
        let (lat, lon) = country_center(&world, "fr").unwrap();
        assert!((lat - 48.85).abs() < 0.01 && (lon - 2.35).abs() < 0.01);
    }

    #[test]
    fn country() {
        assert!(desktop_country().len() == 2);
    }
}

#[cfg(test)]
mod real_world {
    /// Sur le cache réel des stations du monde : `cargo test real_places -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_places() {
        let (world, _) = super::load_world().expect("cache des stations du monde");
        let t = std::time::Instant::now();
        let places = super::places(&world);
        let without_city = places.iter().filter(|p| p.name == p.region).count();
        eprintln!(
            "{} stations, {} lieux ({} sans ville) en {:?}",
            world.len(),
            places.len(),
            without_city,
            t.elapsed()
        );
        for p in places.iter().rev().take(8) {
            eprintln!("  {} — {} stations", p.full_name(), p.stations.len());
        }
        for p in &places {
            if p.stations.iter().any(|&i| world[i].name.contains("Jade FM")) {
                eprintln!("Jade FM : {} ({})", p.full_name(), p.region);
            }
        }
    }
}
