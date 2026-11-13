//! What the player opens: a file, a track within a file (CUE sheet), or an address (music
//! server, podcast), and the engine that decodes it.

use std::path::PathBuf;
use std::time::Duration;

use rodio::Source;

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    File(PathBuf),
    Url(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    pub input: Input,
    /// Start and end of the track within the file (track of a CUE sheet).
    pub start: Duration,
    pub end: Option<Duration>,
    /// Known duration (for ffmpeg decoding, which doesn't report it).
    pub duration: Option<Duration>,
}

impl Media {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            input: Input::File(path.into()),
            start: Duration::ZERO,
            end: None,
            duration: None,
        }
    }

    pub fn url(url: impl Into<String>, duration: Option<Duration>) -> Self {
        Self {
            input: Input::Url(url.into()),
            start: Duration::ZERO,
            end: None,
            duration,
        }
    }

    /// The file, if there is one.
    pub fn path(&self) -> Option<&std::path::Path> {
        match &self.input {
            Input::File(p) => Some(p),
            Input::Url(_) => None,
        }
    }

    /// True if ffmpeg must handle it (remote address, or format unknown to Symphonia).
    pub fn needs_ffmpeg(&self) -> bool {
        match &self.input {
            Input::Url(_) => true,
            Input::File(p) => crate::ffdecode::wants(p),
        }
    }

    /// What ffmpeg receives: the path or the address.
    pub fn ffmpeg_input(&self) -> String {
        match &self.input {
            Input::File(p) => p.to_string_lossy().into_owned(),
            Input::Url(u) => u.clone(),
        }
    }
}

/// A track cut out of a longer source: it starts at `start` and stops at `end`, including after
/// a seek (which rodio's `take_duration` does not recompute).
pub struct Span<S: Source> {
    inner: S,
    start: Duration,
    /// Duration of the track (its end minus its start).
    length: Option<Duration>,
    /// Samples read since the start of the track or the last seek, and the position at that
    /// moment.
    samples: u64,
    base: Duration,
}

impl<S: Source> Span<S> {
    pub fn new(mut inner: S, start: Duration, end: Option<Duration>) -> Self {
        if !start.is_zero() {
            let _ = inner.try_seek(start);
        }
        Self {
            inner,
            start,
            length: end.map(|e| e.saturating_sub(start)),
            samples: 0,
            base: Duration::ZERO,
        }
    }

    fn elapsed(&self) -> Duration {
        let per_second = self.inner.sample_rate() as u64 * self.inner.channels().max(1) as u64;
        self.base + Duration::from_secs_f64(self.samples as f64 / per_second.max(1) as f64)
    }
}

impl<S: Source> Iterator for Span<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if let Some(len) = self.length {
            // Checked once per block of 1024 samples: precise enough, cheap.
            if self.samples.is_multiple_of(1024) && self.elapsed() >= len {
                return None;
            }
        }
        let s = self.inner.next()?;
        self.samples += 1;
        Some(s)
    }
}

impl<S: Source> Source for Span<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.length.or_else(|| self.inner.total_duration())
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(self.start + pos)?;
        self.samples = 0;
        self.base = pos;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_cuts_and_seeks() {
        // 10 s of silence at 1000 Hz mono: the track from 2 s to 5 s.
        let src = rodio::source::Zero::new(1, 1000).take_duration(Duration::from_secs(10));
        let mut span = Span::new(src, Duration::from_secs(2), Some(Duration::from_secs(5)));
        let n = span.by_ref().take(10_000).count();
        assert!((2_990..=3_100).contains(&n), "{n}");
    }
}
