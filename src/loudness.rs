//! Measurement of the perceived loudness of a track or an album, for ReplayGain 2.0: integrated
//! loudness of the EBU R 128 standard (ITU-R BS.1770: "K" filter, 400 ms blocks, double gate),
//! relative to the −18 LUFS reference.
//!
//! Each track is fully decoded once; the energy of its blocks is kept, which allows computing
//! the album gain (all its blocks together) without reading anything again.

use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rodio::{Decoder, Source};

/// ReplayGain 2.0 reference level.
const REFERENCE_LUFS: f64 = -18.0;
/// Absolute gate: below it, a block is silence and doesn't count.
const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// Relative gate: blocks more than 10 LU below the average don't count either.
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

/// The two stages of the K filter (high shelf, then high pass), computed for the file's sample
/// rate.
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

/// Streaming measurement: it is given interleaved samples, and stores the energy of the blocks.
pub struct Meter {
    channels: usize,
    filters: Vec<[Biquad; 2]>,
    /// Length of a 100 ms step, in frames; a 400 ms block covers four of them.
    step: usize,
    /// Energy (sum of squares, all channels) of the current step and the three previous ones.
    current: f64,
    frames: usize,
    recent: [f64; 3],
    steps: usize,
    channel: usize,
    /// A mono file is played on both speakers: it counts double.
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
        // A 100 ms step ends: with the three previous ones, it forms a 400 ms block (blocks thus
        // overlap by three quarters, as the standard requires).
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

/// What is kept from a track: the energy of its 400 ms blocks and its peak.
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

/// Integrated loudness (LUFS) of a set of blocks; `None` if there is only silence.
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

/// ReplayGain gain (dB) that brings this loudness to the reference.
pub fn gain(loudness: f64) -> f32 {
    (REFERENCE_LUFS - loudness) as f32
}

/// Decodes the whole file and measures its loudness. `stop` interrupts decoding.
pub fn analyze(path: &Path, stop: &AtomicBool) -> Result<Analysis, String> {
    // Formats Symphonia doesn't read (Opus, WMA, APE…): decoded by ffmpeg.
    if crate::ffdecode::wants(path) {
        let source = crate::ffdecode::FfmpegSource::open(&path.to_string_lossy(), Duration::ZERO, None)?;
        return measure(source, stop);
    }
    let file = File::open(path).map_err(|e| crate::trf!("ouverture impossible : {e}", e))?;
    // As for playback: a decoder panicking on an exotic file must not take the application down
    // with it.
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

    /// Reference from the standard: a full-scale 997 Hz sine on one channel is −3.01 LKFS; on two
    /// channels, 0. At −20 dBFS in stereo, we must therefore read −20 LUFS.
    #[test]
    fn reference_sine() {
        for rate in [44100, 48000, 96000] {
            let a = sine(rate, 2, 0.1, 5.0);
            let l = integrated(a.blocks.iter()).unwrap();
            assert!((l + 20.0).abs() < 0.1, "{rate} Hz : {l}");
            assert!((gain(l) - 2.0).abs() < 0.1);
            assert!((a.peak - 0.1).abs() < 0.001);
        }
        // Mono: played on both speakers, so as loud as the same sound in stereo.
        let mono = integrated(sine(44100, 1, 0.1, 5.0).blocks.iter()).unwrap();
        assert!((mono + 20.0).abs() < 0.1, "{mono}");
    }

    #[test]
    fn gating() {
        // Silence doesn't count: half of the track being silent doesn't change the measurement.
        let mut loud = sine(44100, 2, 0.1, 4.0).blocks;
        let reference = integrated(loud.iter()).unwrap();
        loud.extend(std::iter::repeat_n(0.0, 40));
        assert!((integrated(loud.iter()).unwrap() - reference).abs() < 0.01);
        assert!(integrated([0.0, 0.0].iter()).is_none());
        // Album: a track at −20 LUFS and another at −25, of the same duration, give −21.8.
        let quiet = sine(44100, 2, 0.0562, 4.0).blocks;
        let album = integrated(loud.iter().chain(quiet.iter())).unwrap();
        assert!((album - (reference - 1.82)).abs() < 0.05, "{album}");
        // A very quiet passage (more than 10 LU below the average) is discarded.
        let faint = sine(44100, 2, 0.001, 4.0).blocks;
        let gated = integrated(loud.iter().chain(faint.iter())).unwrap();
        assert!((gated - reference).abs() < 0.01, "{gated}");
    }
}
