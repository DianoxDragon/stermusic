//! Mesure du volume perçu d'un morceau ou d'un album, pour ReplayGain 2.0 : sonie intégrée de la
//! norme EBU R 128 (ITU-R BS.1770 : filtre « K », blocs de 400 ms, double seuil), ramenée à la
//! référence de −18 LUFS.
//!
//! Chaque morceau est décodé en entier une fois ; on garde l'énergie de ses blocs, ce qui permet
//! de calculer le gain de l'album (tous ses blocs réunis) sans rien relire.

use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rodio::{Decoder, Source};

/// Niveau de référence de ReplayGain 2.0.
const REFERENCE_LUFS: f64 = -18.0;
/// Seuil absolu : en dessous, un bloc est du silence et ne compte pas.
const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// Seuil relatif : les blocs à plus de 10 LU sous la moyenne ne comptent pas non plus.
const RELATIVE_GATE_LU: f64 = -10.0;

#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Les deux étages du filtre K (rehaussement des aigus, puis passe-haut), calculés pour la
/// fréquence d'échantillonnage du fichier.
fn k_weighting(rate: f64) -> [Biquad; 2] {
    use std::f64::consts::PI;
    let shelf = {
        let (f0, gain_db, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
        let k = (PI * f0 / rate).tan();
        let vh = 10f64.powf(gain_db / 20.0);
        let vb = vh.powf(0.4996667741545416);
        let a0 = 1.0 + k / q + k * k;
        Biquad {
            b: [
                (vh + vb * k / q + k * k) / a0,
                2.0 * (k * k - vh) / a0,
                (vh - vb * k / q + k * k) / a0,
            ],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            z: [0.0; 2],
        }
    };
    let highpass = {
        let (f0, q) = (38.13547087602444, 0.5003270373238773);
        let k = (PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        Biquad {
            b: [1.0, -2.0, 1.0],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            z: [0.0; 2],
        }
    };
    [shelf, highpass]
}

/// Mesure en continu : on lui donne les échantillons entrelacés, il range l'énergie des blocs.
pub struct Meter {
    channels: usize,
    filters: Vec<[Biquad; 2]>,
    /// Longueur d'un pas de 100 ms, en trames ; un bloc de 400 ms en couvre quatre.
    step: usize,
    /// Énergie (somme des carrés, toutes voies) du pas en cours et des trois précédents.
    current: f64,
    frames: usize,
    recent: [f64; 3],
    steps: usize,
    channel: usize,
    /// Un fichier mono est joué sur les deux enceintes : il compte double.
    weight: f64,
    blocks: Vec<f64>,
    peak: f32,
}

impl Meter {
    pub fn new(rate: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            channels,
            filters: vec![k_weighting(rate.max(8000) as f64); channels],
            step: (rate.max(8000) as usize / 10).max(1),
            current: 0.0,
            frames: 0,
            recent: [0.0; 3],
            steps: 0,
            channel: 0,
            weight: if channels == 1 { 2.0 } else { 1.0 },
            blocks: Vec::new(),
            peak: 0.0,
        }
    }

    pub fn push(&mut self, sample: f32) {
        self.peak = self.peak.max(sample.abs());
        let [shelf, highpass] = &mut self.filters[self.channel];
        let y = highpass.run(shelf.run(sample as f64));
        self.current += y * y;
        self.channel += 1;
        if self.channel < self.channels {
            return;
        }
        self.channel = 0;
        self.frames += 1;
        if self.frames < self.step {
            return;
        }
        // Un pas de 100 ms se termine : avec les trois précédents, il forme un bloc de 400 ms
        // (les blocs se recouvrent donc aux trois quarts, comme le veut la norme).
        self.steps += 1;
        if self.steps >= 4 {
            let energy = self.current + self.recent.iter().sum::<f64>();
            self.blocks.push(self.weight * energy / (4 * self.step) as f64);
        }
        self.recent = [self.recent[1], self.recent[2], self.current];
        self.current = 0.0;
        self.frames = 0;
    }

    pub fn finish(self) -> Analysis {
        Analysis {
            blocks: self.blocks,
            peak: self.peak,
        }
    }
}

/// Ce qu'on retient d'un morceau : l'énergie de ses blocs de 400 ms et sa crête.
pub struct Analysis {
    pub blocks: Vec<f64>,
    pub peak: f32,
}

fn lufs(mean_square: f64) -> f64 {
    -0.691 + 10.0 * mean_square.log10()
}

fn mean_square(lufs: f64) -> f64 {
    10f64.powf((lufs + 0.691) / 10.0)
}

/// Sonie intégrée (LUFS) d'un ensemble de blocs ; `None` s'il n'y a que du silence.
pub fn integrated<'a>(blocks: impl Iterator<Item = &'a f64> + Clone) -> Option<f64> {
    let mean = |threshold: f64| -> Option<f64> {
        let (mut sum, mut n) = (0.0, 0usize);
        for &b in blocks.clone().filter(|&&b| b >= threshold) {
            sum += b;
            n += 1;
        }
        (n > 0).then(|| sum / n as f64)
    };
    let absolute = mean_square(ABSOLUTE_GATE_LUFS);
    let relative = mean_square(lufs(mean(absolute)?) + RELATIVE_GATE_LU);
    mean(absolute.max(relative)).map(lufs)
}

/// Gain ReplayGain (dB) qui ramène cette sonie à la référence.
pub fn gain(loudness: f64) -> f32 {
    (REFERENCE_LUFS - loudness) as f32
}

/// Décode le fichier en entier et mesure son volume. `stop` interrompt le décodage.
pub fn analyze(path: &Path, stop: &AtomicBool) -> Result<Analysis, String> {
    // Formats que Symphonia ne lit pas (Opus, WMA, APE…) : décodés par ffmpeg.
    if crate::ffdecode::wants(path) {
        let source = crate::ffdecode::FfmpegSource::open(&path.to_string_lossy(), Duration::ZERO, None)?;
        return measure(source, stop);
    }
    let file = File::open(path).map_err(|e| crate::trf!("ouverture impossible : {e}", e))?;
    // Comme pour la lecture : un décodeur qui panique sur un fichier exotique ne doit pas
    // emporter l'application.
    let source = catch_unwind(AssertUnwindSafe(|| Decoder::try_from(file)))
        .map_err(|_| crate::i18n::tr("format non pris en charge").to_owned())?
        .map_err(|e| crate::trf!("format non lisible : {e}", e))?;
    measure(source, stop)
}

fn measure(source: impl Source + Send, stop: &AtomicBool) -> Result<Analysis, String> {
    let mut meter = Meter::new(source.sample_rate(), source.channels() as usize);
    let measured = catch_unwind(AssertUnwindSafe(move || {
        for (i, sample) in source.enumerate() {
            if i % 65536 == 0 && stop.load(Ordering::Relaxed) {
                return None;
            }
            meter.push(sample);
        }
        Some(meter.finish())
    }));
    match measured {
        Ok(Some(a)) => Ok(a),
        Ok(None) => Err(crate::i18n::tr("interrompu").to_owned()),
        Err(_) => Err(crate::i18n::tr("le décodeur a échoué").to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, channels: usize, amplitude: f32, seconds: f32) -> Analysis {
        let mut meter = Meter::new(rate, channels);
        for n in 0..(rate as f32 * seconds) as usize {
            let x = amplitude * (std::f32::consts::TAU * 997.0 * n as f32 / rate as f32).sin();
            for _ in 0..channels {
                meter.push(x);
            }
        }
        meter.finish()
    }

    /// Repère de la norme : une sinusoïde de 997 Hz à pleine échelle sur une voie vaut
    /// −3,01 LKFS ; sur deux voies, 0. À −20 dBFS en stéréo, on doit donc lire −20 LUFS.
    #[test]
    fn reference_sine() {
        for rate in [44100, 48000, 96000] {
            let a = sine(rate, 2, 0.1, 5.0);
            let l = integrated(a.blocks.iter()).unwrap();
            assert!((l + 20.0).abs() < 0.1, "{rate} Hz : {l}");
            assert!((gain(l) - 2.0).abs() < 0.1);
            assert!((a.peak - 0.1).abs() < 0.001);
        }
        // Mono : joué sur les deux enceintes, donc aussi fort que le même son en stéréo.
        let mono = integrated(sine(44100, 1, 0.1, 5.0).blocks.iter()).unwrap();
        assert!((mono + 20.0).abs() < 0.1, "{mono}");
    }

    #[test]
    fn gating() {
        // Le silence ne compte pas : la moitié du morceau muette ne change pas la mesure.
        let mut loud = sine(44100, 2, 0.1, 4.0).blocks;
        let reference = integrated(loud.iter()).unwrap();
        loud.extend(std::iter::repeat_n(0.0, 40));
        assert!((integrated(loud.iter()).unwrap() - reference).abs() < 0.01);
        assert!(integrated([0.0, 0.0].iter()).is_none());
        // Album : un morceau à −20 LUFS et un autre à −25, de même durée, donnent −21,8.
        let quiet = sine(44100, 2, 0.0562, 4.0).blocks;
        let album = integrated(loud.iter().chain(quiet.iter())).unwrap();
        assert!((album - (reference - 1.82)).abs() < 0.05, "{album}");
        // Un passage très faible (plus de 10 LU sous la moyenne) est écarté.
        let faint = sine(44100, 2, 0.001, 4.0).blocks;
        let gated = integrated(loud.iter().chain(faint.iter())).unwrap();
        assert!((gated - reference).abs() < 0.01, "{gated}");
    }
}
