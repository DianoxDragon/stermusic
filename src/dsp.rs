//! Traitement du son, appliqué à chaque morceau pendant la lecture :
//! - égaliseur graphique 10 bandes (31 Hz → 16 kHz), filtres en cloche de l'« Audio EQ Cookbook »
//!   (R. Bristow-Johnson), avec pré-amplification ;
//! - nivellement du volume : gain ReplayGain du fichier s'il existe, sinon nivellement automatique
//!   doux (baisse vite, remonte lentement) ;
//! - limiteur doux, pour que ni l'égaliseur ni le nivellement ne fassent saturer.
//!
//! En sortie bit-perfect, rien de tout cela : le son passe tel quel.
//!
//! Les réglages sont partagés avec l'interface (atomiques) : on les entend changer en direct.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};

/// Fréquences centrales des dix bandes (Hz).
pub const BANDS: [f32; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
/// Plage des curseurs (dB).
pub const MAX_DB: f32 = 12.0;

/// Préréglages, comme dans le lecteur de Windows.
pub const PRESETS: [(&str, [f32; 10]); 10] = [
    (crate::i18n::n_("Plat"), [0.0; 10]),
    (
        crate::i18n::n_("Rock"),
        [4.5, 3.5, 2.0, -0.5, -1.5, -1.0, 1.0, 2.5, 3.5, 4.0],
    ),
    (
        crate::i18n::n_("Pop"),
        [-1.0, 0.5, 2.5, 3.5, 3.0, 1.0, -0.5, -1.0, -1.0, -1.5],
    ),
    (
        crate::i18n::n_("Jazz"),
        [3.0, 2.0, 1.0, 1.5, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0],
    ),
    (
        crate::i18n::n_("Classique"),
        [3.5, 2.5, 1.5, 0.5, -0.5, -0.5, 0.0, 1.5, 2.5, 3.0],
    ),
    (
        crate::i18n::n_("Électro"),
        [5.0, 4.0, 1.0, 0.0, -2.0, 1.0, 0.5, 1.5, 4.0, 4.5],
    ),
    (
        crate::i18n::n_("Basses +"),
        [7.0, 5.5, 4.0, 2.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0],
    ),
    (
        crate::i18n::n_("Aigus +"),
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.5, 4.0, 5.5, 6.5],
    ),
    (
        crate::i18n::n_("Voix"),
        [-2.0, -1.5, -1.0, 1.0, 3.0, 3.5, 3.0, 1.5, 0.0, -1.0],
    ),
    (
        crate::i18n::n_("Acoustique"),
        [3.0, 3.0, 2.0, 1.0, 1.5, 1.5, 2.5, 2.5, 2.0, 1.5],
    ),
];

/// Réglages partagés entre l'interface et le son.
pub struct AudioSettings {
    eq_on: AtomicBool,
    gains: [AtomicU32; 10],
    preamp: AtomicU32,
    leveling: AtomicBool,
    /// Sortie bit-perfect : aucun traitement.
    exact: AtomicBool,
    /// Nivellement automatique : dernier état (gain, puissance moyenne), repris par le morceau
    /// suivant pour que le volume ne saute pas d'un morceau à l'autre.
    last_gain: AtomicU32,
    last_power: AtomicU32,
    version: AtomicU64,
}

pub type Shared = Arc<AudioSettings>;

impl AudioSettings {
    pub fn new() -> Shared {
        Arc::new(Self {
            eq_on: AtomicBool::new(false),
            gains: Default::default(),
            preamp: AtomicU32::new(0f32.to_bits()),
            leveling: AtomicBool::new(false),
            exact: AtomicBool::new(false),
            last_gain: AtomicU32::new(1f32.to_bits()),
            last_power: AtomicU32::new((TARGET_RMS * TARGET_RMS).to_bits()),
            version: AtomicU64::new(1),
        })
    }

    pub fn set(&self, eq_on: bool, gains: &[f32; 10], preamp: f32, leveling: bool) {
        self.eq_on.store(eq_on, Ordering::Relaxed);
        for (slot, g) in self.gains.iter().zip(gains) {
            slot.store(g.clamp(-MAX_DB, MAX_DB).to_bits(), Ordering::Relaxed);
        }
        self.preamp
            .store(preamp.clamp(-MAX_DB, MAX_DB).to_bits(), Ordering::Relaxed);
        self.leveling.store(leveling, Ordering::Relaxed);
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Sortie bit-perfect : le son ne passe plus par aucun traitement.
    pub fn set_exact(&self, exact: bool) {
        self.exact.store(exact, Ordering::Relaxed);
        self.version.fetch_add(1, Ordering::Release);
    }

    fn snapshot(&self) -> (bool, [f32; 10], f32, bool) {
        let gains = std::array::from_fn(|i| f32::from_bits(self.gains[i].load(Ordering::Relaxed)));
        (
            self.eq_on.load(Ordering::Relaxed),
            gains,
            f32::from_bits(self.preamp.load(Ordering::Relaxed)),
            self.leveling.load(Ordering::Relaxed),
        )
    }
}

/// Gain ReplayGain (en dB), s'il est dans les tags : celui de l'album si `album` (et qu'il
/// existe), sinon celui du morceau. Le gain d'album garde les écarts voulus entre les morceaux
/// d'un album écouté à la suite.
pub fn replaygain(path: &Path, album: bool) -> Option<f32> {
    use lofty::prelude::*;
    let tagged = lofty::probe::Probe::open(path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let parse = |key: &ItemKey| -> Option<f32> {
        tag.get_string(key)?
            .trim()
            .trim_end_matches("dB")
            .trim()
            .replace(',', ".")
            .parse()
            .ok()
    };
    album
        .then(|| parse(&ItemKey::ReplayGainAlbumGain))
        .flatten()
        .or_else(|| parse(&ItemKey::ReplayGainTrackGain))
}

fn db(gain_db: f32) -> f32 {
    10f32.powf(gain_db / 20.0)
}

/// Filtre biquadratique (forme directe II transposée).
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    /// Filtre en cloche : `gain_db` autour de `freq`, largeur d'environ une octave.
    fn peaking(freq: f32, gain_db: f32, rate: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = std::f32::consts::TAU * freq / rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * 1.41);
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct State {
    z1: f32,
    z2: f32,
}

/// Source qui applique l'égaliseur, le nivellement et le limiteur à une autre source.
pub struct Processor<S: Source> {
    inner: S,
    settings: Shared,
    version: u64,
    filters: Vec<Biquad>,
    /// États des filtres : `canal * nombre de filtres + filtre`.
    states: Vec<State>,
    channels: usize,
    rate: SampleRate,
    channel: usize,
    countdown: usize,
    preamp: f32,
    eq_on: bool,
    leveling: bool,
    exact: bool,
    replaygain: Option<f32>,
    /// Nivellement automatique : puissance moyenne et gain courant.
    mean_square: f32,
    level_gain: f32,
    /// Copie mono du son pour le visualiseur.
    tap: Option<SharedTap>,
    mono: f32,
    chunk: Vec<f32>,
}

/// Puissance visée par le nivellement automatique (≈ −18 dBFS RMS).
const TARGET_RMS: f32 = 0.125;

impl<S: Source> Processor<S> {
    pub fn new(inner: S, settings: Shared, replaygain: Option<f32>, tap: Option<SharedTap>) -> Self {
        let mean_square = f32::from_bits(settings.last_power.load(Ordering::Relaxed));
        let level_gain = f32::from_bits(settings.last_gain.load(Ordering::Relaxed));
        let mut p = Self {
            channels: inner.channels().max(1) as usize,
            rate: inner.sample_rate(),
            inner,
            settings,
            version: 0,
            filters: Vec::new(),
            states: Vec::new(),
            channel: 0,
            countdown: 0,
            preamp: 1.0,
            eq_on: false,
            leveling: false,
            exact: false,
            replaygain,
            mean_square,
            level_gain,
            tap,
            mono: 0.0,
            chunk: Vec::with_capacity(TAP_CHUNK),
        };
        p.refresh(true);
        p
    }

    /// Relit les réglages (et le format du son) s'ils ont changé.
    fn refresh(&mut self, force: bool) {
        let version = self.settings.version.load(Ordering::Acquire);
        let channels = self.inner.channels().max(1) as usize;
        let rate = self.inner.sample_rate();
        if self.leveling && self.replaygain.is_none() {
            self.settings.last_gain.store(self.level_gain.to_bits(), Ordering::Relaxed);
            self.settings.last_power.store(self.mean_square.to_bits(), Ordering::Relaxed);
        }
        if !force && version == self.version && channels == self.channels && rate == self.rate {
            return;
        }
        self.exact = self.settings.exact.load(Ordering::Relaxed);
        self.version = version;
        let (eq_on, gains, preamp, leveling) = self.settings.snapshot();
        self.eq_on = eq_on;
        self.leveling = leveling;
        self.preamp = if eq_on { db(preamp) } else { 1.0 };
        let nyquist = rate as f32 / 2.0;
        let filters: Vec<Biquad> = if eq_on {
            BANDS
                .iter()
                .zip(gains)
                .filter(|(f, g)| g.abs() > 0.05 && **f < nyquist * 0.9)
                .map(|(f, g)| Biquad::peaking(*f, g, rate as f32))
                .collect()
        } else {
            Vec::new()
        };
        if filters.len() != self.filters.len() || channels != self.channels {
            self.states = vec![State::default(); filters.len() * channels];
        }
        self.filters = filters;
        self.channels = channels;
        self.rate = rate;
        self.channel %= channels;
    }

    fn level(&mut self, x: f32) -> f32 {
        if !self.leveling {
            return x;
        }
        if let Some(rg) = self.replaygain {
            return x * db(rg);
        }
        // Puissance moyenne sur ~3 s ; le gain baisse en ~0,3 s et remonte en ~4 s (pas de « pompage »).
        let per_second = (self.rate as f32 * self.channels as f32).max(1.0);
        self.mean_square += (x * x - self.mean_square) / (3.0 * per_second);
        let wanted = (TARGET_RMS / self.mean_square.sqrt().max(1e-4)).clamp(db(-12.0), db(9.0));
        let speed = if wanted < self.level_gain { 0.3 } else { 4.0 };
        self.level_gain += (wanted - self.level_gain) / (speed * per_second);
        x * self.level_gain
    }
}

impl<S: Source> Processor<S> {
    /// Copie pour le visualiseur : mélange mono, envoyé par paquets (jamais bloquant pour le
    /// son). À appeler après avoir avancé `channel`.
    fn feed_tap(&mut self, y: f32) {
        if let Some(tap) = &self.tap {
            self.mono += y;
            if self.channel == 0 {
                self.chunk.push(self.mono / self.channels as f32);
                self.mono = 0.0;
                if self.chunk.len() >= TAP_CHUNK {
                    tap.push(&self.chunk, self.rate);
                    self.chunk.clear();
                }
            }
        }
    }
}

/// Limiteur doux : rien ne change sous 0,9, au-delà la courbe s'arrondit jusqu'à 1.
fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.9 {
        x
    } else {
        x.signum() * (0.9 + 0.1 * ((a - 0.9) / 0.1).tanh())
    }
}

impl<S: Source> Iterator for Processor<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.countdown == 0 && self.channel == 0 {
            self.refresh(false);
            self.countdown = 1024;
        }
        self.countdown = self.countdown.saturating_sub(1);
        let mut x = self.inner.next()?;
        if self.exact {
            self.channel = (self.channel + 1) % self.channels;
            self.feed_tap(x);
            return Some(x);
        }
        let n = self.filters.len();
        if n > 0 {
            x *= self.preamp;
            let base = self.channel * n;
            for (f, st) in self.filters.iter().zip(&mut self.states[base..base + n]) {
                let y = f.b0 * x + st.z1;
                st.z1 = f.b1 * x - f.a1 * y + st.z2;
                st.z2 = f.b2 * x - f.a2 * y;
                x = y;
            }
        } else if self.eq_on {
            x *= self.preamp;
        }
        x = self.level(x);
        let y = soft_clip(x);
        self.channel = (self.channel + 1) % self.channels;
        self.feed_tap(y);
        Some(y)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source> Source for Processor<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)?;
        self.channel = 0;
        for s in &mut self.states {
            *s = State::default();
        }
        Ok(())
    }
}

// ---------- Copie du son pour le visualiseur ----------

const TAP_CHUNK: usize = 256;
/// Échantillons gardés (un peu plus que la fenêtre d'analyse).
const TAP_KEEP: usize = 4096;

#[derive(Default)]
struct TapBuf {
    samples: VecDeque<f32>,
    rate: u32,
    /// Nombre total d'échantillons reçus (pour savoir si le son avance encore).
    written: u64,
}

/// Les derniers échantillons joués (mono), lus par le visualiseur.
#[derive(Default)]
pub struct Tap {
    buf: Mutex<TapBuf>,
}

pub type SharedTap = Arc<Tap>;

impl Tap {
    pub fn new() -> SharedTap {
        Arc::new(Self::default())
    }

    /// Côté son : n'attend jamais (si l'interface lit au même moment, ce paquet est sauté).
    fn push(&self, chunk: &[f32], rate: u32) {
        if let Ok(mut b) = self.buf.try_lock() {
            b.samples.extend(chunk.iter().copied());
            let excess = b.samples.len().saturating_sub(TAP_KEEP);
            b.samples.drain(..excess);
            b.rate = rate;
            b.written += chunk.len() as u64;
        }
    }

    /// Les `n` derniers échantillons, la fréquence d'échantillonnage et le compteur d'écriture.
    pub fn latest(&self, n: usize, out: &mut Vec<f32>) -> (u32, u64) {
        let b = self.buf.lock().unwrap();
        out.clear();
        let start = b.samples.len().saturating_sub(n);
        out.extend(b.samples.iter().skip(start).copied());
        (b.rate, b.written)
    }
}
