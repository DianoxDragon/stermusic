//! Pochettes d'albums : intégrées au fichier (tags) ou image du dossier (cover.jpg, folder.jpg…).
//! Chargées dans des threads à part, les plus récemment demandées d'abord (ce qui est à l'écran).

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::probe::Probe;

const THUMB: u32 = 256;
const LARGE: u32 = 640;
const BLUR: u32 = 48;

enum Job {
    Thumb(PathBuf),
    Large(PathBuf),
}

enum Done {
    Thumb(PathBuf, Option<ColorImage>),
    Large(
        PathBuf,
        Option<(ColorImage, ColorImage, Option<[Color32; 3]>)>,
        Option<PathBuf>,
    ),
}

struct Queue {
    jobs: Mutex<Vec<Job>>,
    cv: Condvar,
}

/// Grande pochette du morceau en cours, avec sa version floutée (fond de « En cours de lecture »).
pub struct LargeCover {
    pub path: PathBuf,
    pub cover: Option<TextureHandle>,
    pub blurred: Option<TextureHandle>,
    /// Copie PNG sur le disque, pour MPRIS.
    pub file: Option<PathBuf>,
    /// Couleurs dominantes de la pochette (visualiseur ; voir `palette`).
    pub palette: Option<[Color32; 3]>,
}

pub struct Covers {
    thumbs: HashMap<PathBuf, Option<TextureHandle>>,
    pending: HashSet<PathBuf>,
    large: Option<LargeCover>,
    large_pending: Option<PathBuf>,
    queue: Arc<Queue>,
    rx: Receiver<Done>,
}

impl Covers {
    pub fn new(ctx: &egui::Context) -> Self {
        let queue = Arc::new(Queue {
            jobs: Mutex::new(Vec::new()),
            cv: Condvar::new(),
        });
        let (tx, rx) = channel();
        for _ in 0..2 {
            let (queue, tx, ctx) = (queue.clone(), tx.clone(), ctx.clone());
            std::thread::spawn(move || worker(queue, tx, ctx));
        }
        Self {
            thumbs: HashMap::new(),
            pending: HashSet::new(),
            large: None,
            large_pending: None,
            queue,
            rx,
        }
    }

    fn push(&self, job: Job) {
        self.queue.jobs.lock().unwrap().push(job);
        self.queue.cv.notify_one();
    }

    /// Vignette de la pochette de l'album de ce morceau (`None` tant qu'elle charge, ou s'il n'y en a pas).
    pub fn thumb(&mut self, track: &Path) -> Option<&TextureHandle> {
        if self.thumbs.contains_key(track) {
            return self.thumbs[track].as_ref();
        }
        if self.pending.insert(track.to_path_buf()) {
            // Défilement rapide : on oublie les plus anciennes demandes (plus à l'écran).
            let mut jobs = self.queue.jobs.lock().unwrap();
            if jobs.len() > 300 {
                for job in jobs.drain(..150) {
                    if let Job::Thumb(p) = job {
                        self.pending.remove(&p);
                    }
                }
            }
            jobs.push(Job::Thumb(track.to_path_buf()));
            drop(jobs);
            self.queue.cv.notify_one();
        }
        None
    }

    /// Grande pochette du morceau en cours.
    pub fn large(&mut self, track: &Path) -> Option<&LargeCover> {
        if self.large.as_ref().is_some_and(|l| l.path == track) {
            return self.large.as_ref();
        }
        if self.large_pending.as_deref() != Some(track) {
            self.large_pending = Some(track.to_path_buf());
            self.push(Job::Large(track.to_path_buf()));
        }
        None
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.rx.try_recv() {
            match done {
                Done::Thumb(path, image) => {
                    self.pending.remove(&path);
                    let tex = image.map(|i| {
                        ctx.load_texture(format!("thumb:{}", path.display()), i, TextureOptions::LINEAR)
                    });
                    self.thumbs.insert(path, tex);
                }
                Done::Large(path, images, file) => {
                    if self.large_pending.as_ref() != Some(&path) {
                        continue;
                    }
                    self.large_pending = None;
                    let (cover, blurred, palette) = match images {
                        Some((c, b, p)) => (
                            Some(ctx.load_texture("large_cover", c, TextureOptions::LINEAR)),
                            Some(ctx.load_texture("large_blur", b, TextureOptions::LINEAR)),
                            p,
                        ),
                        None => (None, None, None),
                    };
                    self.large = Some(LargeCover {
                        path,
                        cover,
                        blurred,
                        file,
                        palette,
                    });
                }
            }
        }
    }

    /// Oublie les pochettes de ces morceaux (leurs tags viennent d'être modifiés).
    pub fn forget(&mut self, paths: &[PathBuf]) {
        for p in paths {
            self.thumbs.remove(p);
            self.pending.remove(p);
        }
        if self.large.as_ref().is_some_and(|l| paths.contains(&l.path)) {
            self.large = None;
            self.large_pending = None;
        }
    }

    /// Oublie les vignettes (après un rescan, les pochettes ont pu changer).
    pub fn clear(&mut self) {
        self.thumbs.clear();
        self.pending.clear();
        self.large = None;
        self.large_pending = None;
        self.queue.jobs.lock().unwrap().clear();
    }
}

fn worker(queue: Arc<Queue>, tx: Sender<Done>, ctx: egui::Context) {
    loop {
        let job = {
            let mut jobs = queue.jobs.lock().unwrap();
            while jobs.is_empty() {
                jobs = queue.cv.wait(jobs).unwrap();
            }
            // Les grandes pochettes passent avant tout le reste.
            match jobs.iter().rposition(|j| matches!(j, Job::Large(_))) {
                Some(i) => jobs.remove(i),
                None => jobs.pop().unwrap(),
            }
        };
        let done = match job {
            Job::Thumb(path) => {
                let image = load_source(&path).map(|img| to_color(&square(&img, THUMB)));
                Done::Thumb(path, image)
            }
            Job::Large(path) => match load_source(&path) {
                Some(img) => {
                    let large = square(&img, LARGE);
                    let small = square(&img, BLUR);
                    let blurred = image::imageops::blur(&small, 4.0);
                    let file = save_png(&path, &img);
                    let palette = palette(&small);
                    Done::Large(path, Some((to_color(&large), to_color(&blurred), palette)), file)
                }
                None => Done::Large(path, None, None),
            },
        };
        if tx.send(done).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

fn load_source(track: &Path) -> Option<DynamicImage> {
    // Morceau d'un serveur : sa pochette est en ligne ; morceau CUE : celle de son vrai fichier.
    if let Some(url) = crate::library::cover_url(track) {
        return remote_image(&url);
    }
    let file = crate::library::real_file(track)?;
    embedded(&file).or_else(|| {
        // Dossier où se mélangent plusieurs albums : son image n'est pas celle de ce morceau.
        crate::library::folder_art_allowed(track)
            .then(|| folder_image(&file))
            .flatten()
    })
}

/// Pochette téléchargée (gardée dans le cache : elle ne change pas d'un lancement à l'autre).
fn remote_image(url: &str) -> Option<DynamicImage> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    let file = crate::library::cache_dir()
        .join("remote-covers")
        .join(format!("{:016x}", h.finish()));
    if let Ok(img) = image::open(&file) {
        return Some(img);
    }
    let resp = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .get(url)
        .call()
        .ok()?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(resp.into_reader(), 8 << 20), &mut bytes).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let _ = std::fs::create_dir_all(file.parent()?);
    let _ = img.save_with_format(&file, image::ImageFormat::Png);
    Some(img)
}

fn embedded(track: &Path) -> Option<DynamicImage> {
    let tagged = Probe::open(track).ok()?.read().ok()?;
    let pictures: Vec<_> = tagged.tags().iter().flat_map(|t| t.pictures().iter()).collect();
    let pic = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first())?;
    image::load_from_memory(pic.data()).ok()
}

/// Vrai si le morceau a une pochette à montrer : intégrée à ses tags, ou image de son dossier.
pub fn has_cover(track: &Path) -> bool {
    if crate::library::cover_url(track).is_some() {
        return true;
    }
    let Some(file) = crate::library::real_file(track) else {
        return false;
    };
    let folder_ok = crate::library::folder_art_allowed(track);
    let track = file.as_path();
    let embedded = Probe::open(track)
        .ok()
        .and_then(|p| p.read().ok())
        .is_some_and(|tagged| tagged.tags().iter().any(|t| !t.pictures().is_empty()));
    embedded || (folder_ok && !folder_candidates(track).is_empty())
}

/// cover.jpg, folder.png, front.jpg… dans le dossier du morceau.
fn folder_image(track: &Path) -> Option<DynamicImage> {
    folder_candidates(track)
        .into_iter()
        .find_map(|p| image::open(p).ok())
}

/// Images du dossier qui peuvent servir de pochette, les noms les plus courants d'abord.
fn folder_candidates(track: &Path) -> Vec<PathBuf> {
    const NAMES: &[&str] = &[
        "cover",
        "folder",
        "front",
        "album",
        "albumart",
        "artwork",
        "albumartsmall",
    ];
    let Some(entries) = track.parent().and_then(|dir| std::fs::read_dir(dir).ok()) else {
        return Vec::new();
    };
    let mut candidates: Vec<(usize, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter_map(|p| {
            let ext = p.extension()?.to_str()?.to_ascii_lowercase();
            if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "bmp") {
                return None;
            }
            let stem = p.file_stem()?.to_str()?.to_ascii_lowercase();
            let rank = NAMES
                .iter()
                .position(|n| stem == *n || stem.starts_with(&format!("{n}_")))?;
            Some((rank, p))
        })
        .collect();
    candidates.sort();
    candidates.into_iter().map(|(_, p)| p).collect()
}

/// Recadrage carré au centre, puis mise à la taille voulue.
fn square(img: &DynamicImage, size: u32) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    let s = w.min(h).max(1);
    let cropped = img.crop_imm((w - s) / 2, (h - s) / 2, s, s);
    let target = size.min(s);
    cropped
        .resize_exact(target, target, FilterType::Triangle)
        .to_rgba8()
}

fn to_color(img: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

/// Copie de la pochette dans `~/.cache/stermusic/covers/` (le bureau l'affiche via MPRIS).
fn save_png(track: &Path, img: &DynamicImage) -> Option<PathBuf> {
    let dir = crate::library::cache_dir().join("covers");
    std::fs::create_dir_all(&dir).ok()?;
    let mut h = DefaultHasher::new();
    track.parent().unwrap_or(track).hash(&mut h);
    let file = dir.join(format!("{:016x}.png", h.finish()));
    if !file.exists() {
        square(img, 512).save(&file).ok()?;
    }
    Some(file)
}

/// Couleurs dominantes d'une pochette, pour le visualiseur : la teinte la plus présente, puis
/// deux autres (ou des voisines). Une pochette aux couleurs éteintes (pastel, sépia, sombre) donne
/// quand même ses teintes, ravivées ; une pochette en noir et blanc, une lumière argentée. Le
/// visualiseur ne retombe ainsi jamais sur les couleurs du thème quand il y a une pochette.
fn palette(img: &RgbaImage) -> Option<[Color32; 3]> {
    let pixels = img.pixels().len() as f32;
    let (mut weight, mut sum) = hue_histogram(img, 0.18, 0.12, true);
    let mut total: f32 = weight.iter().sum();
    // Moins de ~3 % de pixels francs : on écoute les couleurs faibles.
    let muted = total < pixels * 0.02;
    if muted {
        (weight, sum) = hue_histogram(img, 0.06, 0.08, false);
        total = weight.iter().sum();
        if total < pixels * 0.004 {
            return Some(NEUTRAL);
        }
    }
    let mut order: Vec<usize> = (0..BINS).collect();
    order.sort_by(|&a, &b| weight[b].total_cmp(&weight[a]));
    let color = |bin: usize| -> Color32 {
        let w = weight[bin].max(1e-6);
        vivid(sum[bin][0] / w, sum[bin][1] / w, sum[bin][2] / w)
    };
    let first = order[0];
    let distance = |a: usize, b: usize| {
        let d = a.abs_diff(b);
        d.min(BINS - d)
    };
    let second = order
        .iter()
        .copied()
        .find(|&b| distance(b, first) >= 3 && weight[b] > total * 0.04);
    let a = color(first);
    let b = second.map(color).unwrap_or_else(|| shift(a));
    let third = order
        .iter()
        .copied()
        .find(|&c| Some(c) != second && distance(c, first) >= 2 && weight[c] > total * 0.02)
        .map(color)
        .unwrap_or_else(|| shift(b));
    Some([a, b, third])
}

const BINS: usize = 18;

/// Lumière argentée, pour une pochette en noir et blanc.
const NEUTRAL: [Color32; 3] = [
    Color32::from_rgb(214, 220, 236),
    Color32::from_rgb(150, 162, 190),
    Color32::from_rgb(96, 106, 132),
];

/// Poids et somme des couleurs par teinte, pour les pixels au moins aussi saturés que `min_sat`
/// et aussi clairs que `min_light`. `strict` favorise nettement les couleurs franches.
fn hue_histogram(img: &RgbaImage, min_sat: f32, min_light: f32, strict: bool) -> ([f32; BINS], [[f32; 3]; BINS]) {
    let mut weight = [0f32; BINS];
    let mut sum = [[0f32; 3]; BINS];
    for p in img.pixels() {
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
        if max < min_light || sat < min_sat || max == min {
            continue;
        }
        let hue = if max == r {
            ((g - b) / (max - min)).rem_euclid(6.0)
        } else if max == g {
            (b - r) / (max - min) + 2.0
        } else {
            (r - g) / (max - min) + 4.0
        } / 6.0;
        let bin = ((hue * BINS as f32) as usize).min(BINS - 1);
        let w = if strict { sat * sat } else { sat } * (0.4 + max);
        weight[bin] += w;
        for (k, c) in [r, g, b].into_iter().enumerate() {
            sum[bin][k] += c * w;
        }
    }
    (weight, sum)
}

/// Couleur ravivée : saturation et luminosité remontées, pour une lumière sur fond sombre.
/// Une couleur pâle ou éteinte est d'abord relevée jusqu'à une demi-saturation : sa teinte se
/// voit, au lieu d'une lumière blanchâtre.
fn vivid(r: f32, g: f32, b: f32) -> Color32 {
    let min_sat = 0.5;
    let max = r.max(g).max(b).max(1e-3);
    let (mut r, mut g, mut b) = (r / max, g / max, b / max);
    let sat = 1.0 - r.min(g).min(b);
    if sat > 1e-3 && sat < min_sat {
        let k = min_sat / sat;
        (r, g, b) = (1.0 - (1.0 - r) * k, 1.0 - (1.0 - g) * k, 1.0 - (1.0 - b) * k);
    }
    let l = (r + g + b) / 3.0;
    let boost = |c: f32| ((l + (c - l) * 1.35).clamp(0.0, 1.0) * 0.92 * 255.0) as u8;
    Color32::from_rgb(boost(r), boost(g), boost(b))
}

/// Variante voisine d'une couleur (quand la pochette n'a qu'une teinte).
fn shift(c: Color32) -> Color32 {
    let mix = |x: u8, y: u8| ((x as u16 + y as u16) / 2) as u8;
    Color32::from_rgb(mix(c.r(), c.b()), mix(c.g(), c.r()), mix(c.b(), c.g()))
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    fn flat(r: u8, g: u8, b: u8) -> RgbaImage {
        RgbaImage::from_pixel(16, 16, image::Rgba([r, g, b, 255]))
    }

    fn hue_of(c: Color32) -> (bool, bool, bool) {
        (c.r() > c.g(), c.g() > c.b(), c.b() > c.r())
    }

    /// Une pochette donne toujours ses propres couleurs : franches, éteintes (ravivées), ou une
    /// lumière argentée pour du noir et blanc ; jamais « rien » (les couleurs du thème).
    #[test]
    fn every_cover_has_a_palette() {
        // Rouge franc.
        let p = palette(&flat(200, 30, 20)).unwrap();
        assert!(p[0].r() > 200 && p[0].g() < 90);
        // Sépia éteint : une teinte chaude, nettement plus saturée que l'image.
        let p = palette(&flat(120, 105, 90)).unwrap();
        assert_eq!(hue_of(p[0]), (true, true, false));
        assert!(p[0].r() as i32 - p[0].b() as i32 > 80, "{:?}", p[0]);
        // Bleu nuit très sombre mais coloré.
        let p = palette(&flat(20, 26, 44)).unwrap();
        assert!(p[0].b() > p[0].r() && p[0].b() > 150, "{:?}", p[0]);
        // Noir et blanc : argenté, pas de teinte marquée.
        assert_eq!(palette(&flat(128, 128, 128)), Some(NEUTRAL));
        assert_eq!(palette(&flat(0, 0, 0)), Some(NEUTRAL));
    }
}
