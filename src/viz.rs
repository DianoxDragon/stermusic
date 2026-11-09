//! Visualiseur : analyse du son (spectre) et dessins qui suivent la musique, volontairement
//! vagues — lumière, flou, couleurs de la pochette, dans l'esprit du Mica.
//!
//! - « Aurore » (En cours de lecture) : rubans lumineux qui ondulent, barres de spectre fondues
//!   et particules qui montent ;
//! - halo qui « respire » sur les basses derrière la pochette ;
//! - taches de lumière qui dérivent dans le bandeau de l'accueil.

use eframe::egui::{self, pos2, vec2, Color32, Rect, Stroke};

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
    /// « Temps » des animations : avance plus vite quand la musique est intense.
    pub flow: f32,
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
            flow: 0.0,
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
        // Montée rapide, descente douce : un mouvement fluide plutôt que nerveux.
        for (v, t) in self.bands.iter_mut().zip(target) {
            let k = if t > *v { 18.0 } else { 3.5 };
            *v += (t - *v) * (k * dt).min(1.0);
        }
        let avg = |r: std::ops::Range<usize>| self.bands[r.clone()].iter().sum::<f32>() / r.len() as f32;
        self.bass = avg(0..7);
        self.mid = avg(7..24);
        self.high = avg(24..BANDS);
        self.energy = avg(0..BANDS);
        self.flow += dt * if playing { 0.35 + 1.1 * self.energy } else { 0.08 };
    }

    /// Niveau du spectre à la position `u` (0 → 1) d'une largeur : graves au centre, aigus aux bords.
    pub fn at(&self, u: f32) -> f32 {
        let d = (u * 2.0 - 1.0).abs().clamp(0.0, 1.0);
        let x = d * (BANDS - 1) as f32;
        let (i, f) = (x.floor() as usize, x.fract());
        let a = self.bands[i.min(BANDS - 1)];
        let b = self.bands[(i + 1).min(BANDS - 1)];
        a + (b - a) * f
    }

    /// Vrai si quelque chose bouge encore (pour ne redessiner que si nécessaire).
    pub fn alive(&self) -> bool {
        self.energy > 0.002
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

/// Pseudo-hasard stable (particules).
fn hash(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x % 10_000) as f32 / 10_000.0
}

/// « Aurore » : rubans lumineux, barres de spectre fondues et particules, dans `rect`.
pub fn aurora(painter: &egui::Painter, rect: Rect, s: &Spectrum, palette: [Color32; 3], intensity: f32) {
    let t = s.flow;
    let base = rect.top() + rect.height() * 0.62;

    // Barres de spectre, très discrètes, qui montent depuis la ligne des rubans.
    let bars = 72;
    let inner = rect.shrink2(vec2(rect.width() * 0.12, 0.0));
    let step = inner.width() / bars as f32;
    for i in 0..bars {
        let u = (i as f32 + 0.5) / bars as f32;
        let level = s.at(u);
        let h = rect.height() * 0.42 * level * level.sqrt();
        if h < 1.0 {
            continue;
        }
        let x = inner.left() + (i as f32 + 0.5) * step;
        let bar = Rect::from_min_max(pos2(x - step * 0.22, base - h), pos2(x + step * 0.22, base));
        let color = if i % 2 == 0 { palette[0] } else { palette[1] };
        let mut mesh = egui::Mesh::default();
        let bottom = with_alpha(color, 0.16 * intensity);
        let top = with_alpha(color, 0.0);
        for (p, c) in [
            (bar.left_top(), top),
            (bar.right_top(), top),
            (bar.right_bottom(), bottom),
            (bar.left_bottom(), bottom),
        ] {
            mesh.colored_vertex(p, c);
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
    }

    // Rubans : des bandes de lumière, denses au centre et transparentes sur les bords (un mesh à
    // trois rangées), avec un fil plus vif au milieu.
    let ribbons = [
        (palette[0], 1.4_f32, 0.8_f32, 0.0_f32, 1.0_f32),
        (palette[1], 2.1, 1.2, 2.1, 0.75),
        (palette[2], 1.0, 0.6, 4.0, 0.55),
    ];
    let n = 140;
    for (k, (color, freq, speed, phase, weight)) in ribbons.into_iter().enumerate() {
        let amp_base = rect.height() * (0.16 + 0.05 * k as f32);
        let mut mesh = egui::Mesh::default();
        let mut core = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let u = i as f32 / n as f32;
            // Les bords s'effacent : le ruban naît et meurt dans le flou.
            let envelope = (std::f32::consts::PI * u).sin().powf(1.3);
            let level = s.at(u);
            let wave = (u * freq * std::f32::consts::TAU + t * speed + phase).sin()
                + 0.4 * (u * freq * 2.7 * std::f32::consts::TAU - t * speed * 1.3 + phase * 0.7).sin();
            let amp = amp_base * (0.3 + 1.1 * level) * envelope;
            let x = rect.left() + u * rect.width();
            let y = base + wave * amp * 0.5 - k as f32 * 6.0;
            let half = (10.0 + rect.height() * 0.24 * level) * envelope * (0.75 + 0.5 * s.energy) * weight;
            let a = (0.20 + 0.32 * level) * envelope * weight * intensity;
            mesh.colored_vertex(pos2(x, y - half), Color32::TRANSPARENT);
            mesh.colored_vertex(pos2(x, y), with_alpha(color, a));
            mesh.colored_vertex(pos2(x, y + half), Color32::TRANSPARENT);
            if i > 0 {
                let (p, q) = ((i as u32 - 1) * 3, i as u32 * 3);
                mesh.add_triangle(p, p + 1, q);
                mesh.add_triangle(p + 1, q + 1, q);
                mesh.add_triangle(p + 1, p + 2, q + 1);
                mesh.add_triangle(p + 2, q + 2, q + 1);
            }
            core.push(pos2(x, y));
        }
        painter.add(mesh);
        let glow = (0.10 + 0.25 * s.energy) * weight * intensity;
        painter.add(egui::Shape::line(
            core.clone(),
            Stroke::new(7.0_f32, with_alpha(color, glow * 0.7)),
        ));
        painter.add(egui::Shape::line(
            core,
            Stroke::new(
                1.6_f32,
                with_alpha(
                    crate::theme::mix(color, Color32::WHITE, 0.35),
                    (0.35 + 0.5 * s.energy) * weight * intensity,
                ),
            ),
        ));
    }

    // Particules qui montent doucement et scintillent.
    for i in 0..42u32 {
        let x = rect.left() + hash(i) * rect.width();
        let speed = 0.03 + hash(i + 100) * 0.06;
        let y_norm = (hash(i + 200) + t * speed).fract();
        let y = rect.bottom() - y_norm * rect.height() * 1.1;
        let twinkle = 0.5 + 0.5 * (t * (1.5 + hash(i + 300) * 3.0) + i as f32).sin();
        let fade = (1.0 - y_norm).min(y_norm * 4.0).clamp(0.0, 1.0);
        let a = (0.15 + 0.45 * s.high) * twinkle * fade * intensity;
        let r = 0.8 + hash(i + 400) * 1.4;
        painter.circle_filled(pos2(x, y), r, with_alpha(palette[(i % 3) as usize], a));
    }
}

/// Halo qui « respire » sur les basses, derrière la pochette.
pub fn halo(painter: &egui::Painter, cover: Rect, s: &Spectrum, color: Color32, intensity: f32) {
    let bounds = cover.expand(cover.width() * 0.6);
    let radius = cover.width() * (0.72 + 0.16 * s.bass);
    let a = (0.18 + 0.45 * s.bass) * intensity;
    widgets::soft_glow(painter, bounds, cover.center(), radius, with_alpha(color, a));
}

/// Taches de lumière qui dérivent dans un bandeau (accueil), leur éclat suit la musique.
pub fn drifting_lights(painter: &egui::Painter, rect: Rect, s: &Spectrum, palette: [Color32; 3]) {
    let t = s.flow * 0.6;
    let levels = [s.bass, s.mid, s.high];
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
        let radius = rect.height() * (1.15 + 0.7 * level);
        let alpha = 0.22 + 0.45 * level;
        widgets::soft_glow(painter, rect, center, radius, with_alpha(palette[k], alpha));
    }
}
