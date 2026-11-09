//! Visualiseur : analyse du son (spectre) et lumières qui suivent la musique, volontairement
//! vagues — lumière diffuse, flou, couleurs de la pochette, dans l'esprit du Mica.
//!
//! - « Aurore » (En cours de lecture) : des nappes de lumière peintes dans une petite image,
//!   floutée puis agrandie (egui ne sait pas flouter ce qu'il dessine) ; elles flamboient sur les
//!   coups de grosse caisse ;
//! - halo doux derrière la pochette (la pochette elle-même ne bouge pas) ;
//! - taches de lumière qui dérivent dans le bandeau de l'accueil.

use eframe::egui::{self, pos2, Color32, Rect};

use crate::dsp::Tap;
use crate::widgets;

const FFT_SIZE: usize = 2048;
pub const BANDS: usize = 40;

/// Spectre lissé, mis à jour à chaque image quand le visualiseur est à l'écran.
pub struct Spectrum {
    pub bands: [f32; BANDS],
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
    pub energy: f32,
    /// Moyennes lentes (halo, lumières de l'accueil) : elles suivent l'ambiance, pas chaque coup.
    pub bass_slow: f32,
    pub energy_slow: f32,
    /// Coup de grosse caisse : monte d'un coup sur une attaque des basses, retombe en ~0,3 s.
    pub kick: f32,
    /// « Temps » des animations : avance plus vite quand la musique est intense.
    pub flow: f32,
    raw_bass_avg: f32,
    last_written: u64,
    silent_for: f32,
    samples: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    window: Vec<f32>,
}

impl Default for Spectrum {
    fn default() -> Self {
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (FFT_SIZE - 1) as f32).cos())
            .collect();
        Self {
            bands: [0.0; BANDS],
            bass: 0.0,
            mid: 0.0,
            high: 0.0,
            energy: 0.0,
            bass_slow: 0.0,
            energy_slow: 0.0,
            kick: 0.0,
            flow: 0.0,
            raw_bass_avg: 0.0,
            last_written: 0,
            silent_for: 1.0,
            samples: Vec::with_capacity(FFT_SIZE),
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            window,
        }
    }
}

impl Spectrum {
    /// Analyse les derniers échantillons joués. En pause, tout redescend doucement.
    pub fn update(&mut self, tap: &Tap, dt: f32, playing: bool) {
        let dt = dt.clamp(0.0, 0.1);
        let (rate, written) = tap.latest(FFT_SIZE, &mut self.samples);
        if written != self.last_written {
            self.last_written = written;
            self.silent_for = 0.0;
        } else {
            self.silent_for += dt;
        }
        let fresh = playing && self.silent_for < 0.25 && self.samples.len() == FFT_SIZE && rate > 0;
        let mut target = [0f32; BANDS];
        if fresh {
            for i in 0..FFT_SIZE {
                self.re[i] = self.samples[i] * self.window[i];
                self.im[i] = 0.0;
            }
            fft(&mut self.re, &mut self.im);
            let nyquist = rate as f32 / 2.0;
            let (lo, hi) = (35f32, 16000f32.min(nyquist * 0.95));
            let bin_hz = rate as f32 / FFT_SIZE as f32;
            for (b, t) in target.iter_mut().enumerate() {
                let f0 = lo * (hi / lo).powf(b as f32 / BANDS as f32);
                let f1 = lo * (hi / lo).powf((b + 1) as f32 / BANDS as f32);
                let i0 = (f0 / bin_hz) as usize;
                let i1 = ((f1 / bin_hz) as usize).max(i0 + 1);
                let mut peak = 0f32;
                for i in i0.max(1)..i1.min(FFT_SIZE / 2) {
                    peak = peak.max((self.re[i] * self.re[i] + self.im[i] * self.im[i]).sqrt());
                }
                // Amplitude en dB, ramenée entre 0 et 1 ; les aigus, plus faibles, sont un peu relevés.
                let tilt = 1.0 + 0.6 * b as f32 / BANDS as f32;
                let level = 20.0 * (peak * tilt / (FFT_SIZE as f32 / 4.0)).max(1e-6).log10();
                *t = ((level + 58.0) / 48.0).clamp(0.0, 1.0);
            }
        }
        // Coup de grosse caisse : les basses brutes dépassent nettement leur moyenne récente.
        let raw_bass = target[..6].iter().sum::<f32>() / 6.0;
        let flux = (raw_bass - self.raw_bass_avg).max(0.0);
        self.raw_bass_avg += (raw_bass - self.raw_bass_avg) * (dt * 4.0).min(1.0);
        self.kick = (self.kick * (-dt * 4.5).exp()).max((flux * 4.0).min(1.0));

        // Montée assez rapide, descente lente : un mouvement ample et fluide, pas nerveux.
        for (v, t) in self.bands.iter_mut().zip(target) {
            let k = if t > *v { 9.0 } else { 2.2 };
            *v += (t - *v) * (k * dt).min(1.0);
        }
        let avg = |r: std::ops::Range<usize>| self.bands[r.clone()].iter().sum::<f32>() / r.len() as f32;
        self.bass = avg(0..7);
        self.mid = avg(7..24);
        self.high = avg(24..BANDS);
        self.energy = avg(0..BANDS);
        self.bass_slow += (self.bass - self.bass_slow) * (dt * 0.8).min(1.0);
        self.energy_slow += (self.energy - self.energy_slow) * (dt * 0.8).min(1.0);
        self.flow += dt
            * if playing {
                0.25 + 0.7 * self.energy_slow
            } else {
                0.06
            };
    }

    /// Niveau du spectre à la position `u` (0 → 1) d'une largeur : graves au centre, aigus aux
    /// bords ; lissé avec les bandes voisines pour des formes douces.
    pub fn at(&self, u: f32) -> f32 {
        let d = (u * 2.0 - 1.0).abs().clamp(0.0, 1.0);
        let x = d * (BANDS - 1) as f32;
        let sample = |x: f32| {
            let x = x.clamp(0.0, (BANDS - 1) as f32);
            let (i, f) = (x.floor() as usize, x.fract());
            let a = self.bands[i.min(BANDS - 1)];
            let b = self.bands[(i + 1).min(BANDS - 1)];
            a + (b - a) * f
        };
        (sample(x - 2.0) + 2.0 * sample(x - 1.0) + 3.0 * sample(x) + 2.0 * sample(x + 1.0) + sample(x + 2.0))
            / 9.0
    }

    /// Vrai si quelque chose bouge encore (pour ne redessiner que si nécessaire).
    pub fn alive(&self) -> bool {
        self.energy > 0.002 || self.kick > 0.01
    }
}

/// Transformée de Fourier rapide (radix 2, en place).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -std::f32::consts::TAU / len as f32;
        let (wr, wi) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1f32, 0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
        }
        len <<= 1;
    }
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    crate::theme::alpha(c, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn rgb(c: Color32) -> [f32; 3] {
    [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0]
}

/// Nuances harmonieuses tirées de la couleur dominante : la couleur, une variante plus claire et
/// chaude, une variante plus profonde (plutôt que trois couleurs de la pochette qui se heurtent).
pub fn harmonize(palette: [Color32; 3]) -> [Color32; 3] {
    let main = palette[0];
    let light = crate::theme::mix(crate::theme::mix(main, palette[1], 0.25), Color32::WHITE, 0.35);
    let deep = crate::theme::mix(
        crate::theme::mix(main, palette[2], 0.2),
        Color32::from_rgb(40, 20, 80),
        0.25,
    );
    [main, light, deep]
}

/// Pseudo-hasard stable (particules).
fn hash(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x % 10_000) as f32 / 10_000.0
}

const FIELD_W: usize = 240;
const FIELD_H: usize = 70;

/// Nappes de lumière de l'Aurore, peintes dans une petite image floutée puis agrandie.
#[derive(Default)]
pub struct Aurora {
    glow: Vec<[f32; 3]>,
    core: Vec<[f32; 3]>,
    tmp: Vec<[f32; 3]>,
    texture: Option<egui::TextureHandle>,
}

impl Aurora {
    /// Dessine l'Aurore dans `rect` (bas de « En cours de lecture »).
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        s: &Spectrum,
        palette: [Color32; 3],
        intensity: f32,
    ) {
        let (w, h) = (FIELD_W, FIELD_H);
        self.glow.clear();
        self.glow.resize(w * h, [0.0; 3]);
        self.core.clear();
        self.core.resize(w * h, [0.0; 3]);
        let [c_main, c_light, c_deep] = palette.map(rgb);
        let t = s.flow;
        let kick = s.kick;
        let base = h as f32 * 0.6;

        // Lueur de fond : une grande nappe qui gonfle sur les coups de grosse caisse.
        let (gx, gy) = (w as f32 * 0.5, h as f32 * 0.72);
        let (sx, sy) = (w as f32 * (0.24 + 0.06 * kick), h as f32 * (0.30 + 0.12 * kick));
        let amp = (0.10 + 0.18 * s.bass_slow + 0.45 * kick) * intensity;
        for y in 0..h {
            for x in 0..w {
                let dx = (x as f32 - gx) / sx;
                let dy = (y as f32 - gy) / sy;
                let v = amp * (-(dx * dx + dy * dy)).exp();
                if v > 0.002 {
                    add(&mut self.glow[y * w + x], c_deep, v);
                }
            }
        }

        // Colonnes de lumière douces (le spectre), fondues par le flou.
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let envelope = (std::f32::consts::PI * u).sin();
            let level = s.at(u) * envelope;
            let top = base - level * h as f32 * 0.55;
            for y in (top.max(0.0) as usize)..(base as usize).min(h) {
                let k = 1.0 - (base - y as f32) / (base - top).max(1.0);
                add(&mut self.glow[y * w + x], c_main, 0.07 * k * intensity);
            }
        }

        // Trois rubans : une large lueur (floutée) et un cœur plus fin (net).
        let ribbons = [
            (c_main, 1.3_f32, 0.55_f32, 0.0_f32, 1.0_f32),
            (c_light, 1.9, 0.8, 2.1, 0.7),
            (c_deep, 0.9, 0.4, 4.0, 0.6),
        ];
        for (k, (color, freq, speed, phase, weight)) in ribbons.into_iter().enumerate() {
            for x in 0..w {
                let u = (x as f32 + 0.5) / w as f32;
                let envelope = (std::f32::consts::PI * u).sin().powf(1.2);
                let level = s.at(u);
                let wave = (u * freq * std::f32::consts::TAU + t * speed + phase).sin()
                    + 0.35 * (u * freq * 2.3 * std::f32::consts::TAU - t * speed * 1.4 + phase).sin();
                let amp = h as f32 * (0.10 + 0.16 * level + 0.06 * kick) * envelope;
                let cy = base - 3.0 + wave * amp - k as f32 * 2.0;
                let sigma = (1.6 + 5.5 * level + 2.5 * kick) * envelope.max(0.15);
                let a = (0.35 + 0.8 * level + 0.9 * kick) * envelope * weight * intensity;
                let y0 = (cy - 3.0 * sigma).floor().max(0.0) as usize;
                let y1 = ((cy + 3.0 * sigma).ceil() as usize).min(h);
                for y in y0..y1 {
                    let d = (y as f32 - cy) / sigma;
                    add(&mut self.glow[y * w + x], color, a * 0.55 * (-d * d).exp());
                    let dc = (y as f32 - cy) / 0.9;
                    let core = a * 0.5 * (-dc * dc).exp();
                    if core > 0.003 {
                        add(&mut self.core[y * w + x], mix3(color, [1.0; 3], 0.35), core);
                    }
                }
            }
        }

        // Flou de la lueur (boîte séparable, deux passes ≈ gaussien), puis le cœur par-dessus.
        self.tmp.resize(w * h, [0.0; 3]);
        for _ in 0..2 {
            box_blur(&mut self.glow, &mut self.tmp, w, h, 3);
        }
        let pixels = self
            .glow
            .iter()
            .zip(&self.core)
            .map(|(g, c)| {
                // Saturation douce (1 − e^−x) : les zones intenses « éclatent » sans brûler.
                let out = [0, 1, 2].map(|i| 1.0 - (-(g[i] + c[i]) * 1.6).exp());
                let a = out[0].max(out[1]).max(out[2]);
                let b = |v: f32| (v * 255.0) as u8;
                Color32::from_rgba_premultiplied(b(out[0]), b(out[1]), b(out[2]), b(a))
            })
            .collect();
        let image = egui::ColorImage { size: [w, h], pixels };
        match &mut self.texture {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.texture = Some(
                    painter
                        .ctx()
                        .load_texture("aurora", image, egui::TextureOptions::LINEAR),
                )
            }
        }
        if let Some(tex) = &self.texture {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(tex.id(), rect, uv, Color32::WHITE);
        }

        // Quelques poussières de lumière qui montent doucement (nettes, très discrètes).
        for i in 0..24u32 {
            let x = rect.left() + (0.08 + 0.84 * hash(i)) * rect.width();
            let speed = 0.025 + hash(i + 100) * 0.04;
            let y_norm = (hash(i + 200) + t * speed).fract();
            let y = rect.bottom() - y_norm * rect.height();
            let twinkle = 0.5 + 0.5 * (t * (1.2 + hash(i + 300) * 2.0) + i as f32).sin();
            let fade = (1.0 - y_norm).min(y_norm * 3.0).clamp(0.0, 1.0);
            let a = (0.10 + 0.30 * s.high) * twinkle * fade * intensity;
            painter.circle_filled(pos2(x, y), 0.8 + hash(i + 400), with_alpha(palette[1], a));
        }
    }
}

fn add(px: &mut [f32; 3], c: [f32; 3], v: f32) {
    for i in 0..3 {
        px[i] += c[i] * v;
    }
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// Flou en boîte, horizontal puis vertical, de rayon `r`.
fn box_blur(buf: &mut [[f32; 3]], tmp: &mut [[f32; 3]], w: usize, h: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f32;
    for y in 0..h {
        let row = &buf[y * w..(y + 1) * w];
        let mut acc = [0f32; 3];
        for px in &row[..=r.min(w - 1)] {
            add(&mut acc, *px, 1.0);
        }
        for x in 0..w {
            tmp[y * w + x] = acc.map(|v| v * norm);
            if x + r + 1 < w {
                add(&mut acc, row[x + r + 1], 1.0);
            }
            if x >= r {
                add(&mut acc, row[x - r], -1.0);
            }
        }
    }
    for x in 0..w {
        let mut acc = [0f32; 3];
        for y in 0..=r.min(h - 1) {
            add(&mut acc, tmp[y * w + x], 1.0);
        }
        for y in 0..h {
            buf[y * w + x] = acc.map(|v| v * norm);
            if y + r + 1 < h {
                add(&mut acc, tmp[(y + r + 1) * w + x], 1.0);
            }
            if y >= r {
                add(&mut acc, tmp[(y - r) * w + x], -1.0);
            }
        }
    }
}

/// Halo doux derrière la pochette : il suit l'ambiance (moyenne lente), pas chaque coup.
pub fn halo(painter: &egui::Painter, cover: Rect, s: &Spectrum, color: Color32, intensity: f32) {
    let bounds = cover.expand(cover.width() * 0.6);
    let radius = cover.width() * (0.75 + 0.08 * s.bass_slow);
    let a = (0.16 + 0.22 * s.energy_slow) * intensity;
    widgets::soft_glow(painter, bounds, cover.center(), radius, with_alpha(color, a));
}

/// Taches de lumière qui dérivent dans un bandeau (accueil), leur éclat suit la musique.
pub fn drifting_lights(painter: &egui::Painter, rect: Rect, s: &Spectrum, palette: [Color32; 3]) {
    let t = s.flow * 0.6;
    let levels = [s.bass_slow.max(s.kick * 0.6), s.energy_slow, s.high];
    let paths = [
        (0.27, 0.41, 0.0, 1.3),
        (0.19, 0.33, 2.4, 0.4),
        (0.23, 0.29, 4.1, 2.6),
    ];
    for (k, (a, b, p, q)) in paths.into_iter().enumerate() {
        let center = pos2(
            rect.left() + rect.width() * (0.5 + 0.38 * (t * a + p).sin()),
            rect.top() + rect.height() * (0.5 + 0.45 * (t * b + q).cos()),
        );
        let level = levels[k];
        let radius = rect.height() * (1.15 + 0.6 * level);
        let alpha = 0.22 + 0.40 * level;
        widgets::soft_glow(painter, rect, center, radius, with_alpha(palette[k], alpha));
    }
}

/// Réglages du shader de vagues pour l'état actuel de la musique (à ajuster par l'appelant :
/// position de la ligne de base, fondus, coins…).
pub fn waves(s: &Spectrum, palette: [Color32; 3], intensity: f32) -> crate::shader::Waves {
    let levels = std::array::from_fn(|i| s.at(0.5 + 0.5 * i as f32 / (crate::shader::LEVELS - 1) as f32));
    crate::shader::Waves {
        time: s.flow,
        levels,
        colors: palette.map(rgb),
        kick: s.kick,
        bass: s.bass_slow,
        intensity,
        base: 0.6,
        amp: 1.0,
        fade_top: 0.4,
        fade_bottom: 0.12,
        left: 0.0,
        radius: 0.0,
    }
}

/// Poussières de lumière qui montent doucement et scintillent (nettes, très discrètes).
pub fn particles(painter: &egui::Painter, rect: Rect, s: &Spectrum, color: Color32, intensity: f32) {
    let t = s.flow;
    for i in 0..24u32 {
        let x = rect.left() + (0.08 + 0.84 * hash(i)) * rect.width();
        let speed = 0.025 + hash(i + 100) * 0.04;
        let y_norm = (hash(i + 200) + t * speed).fract();
        let y = rect.bottom() - y_norm * rect.height();
        let twinkle = 0.5 + 0.5 * (t * (1.2 + hash(i + 300) * 2.0) + i as f32).sin();
        let fade = (1.0 - y_norm).min(y_norm * 3.0).clamp(0.0, 1.0);
        let a = (0.10 + 0.30 * s.high) * twinkle * fade * intensity;
        painter.circle_filled(pos2(x, y), 0.8 + hash(i + 400), with_alpha(color, a));
    }
}
