//! Audio engine: a thin layer on top of rodio.
//!
//! When paused, the audio output is released right away (when stopped, after a short delay, so
//! as not to cut between two tracks): otherwise PipeWire sees an active stream (silence) and
//! Plasma shows SterMusic as "playing". On resume, the track is reopened at the same place.
//!
//! **Gapless playback**: the sound of an output is a [`Chain`]. A few seconds before the end of
//! a track, the application appends the next one to it ([`Player::queue_next`]): it starts at
//! the exact sample where the first one stops, without reopening the output or the decoder.
//!
//! **Bit-perfect output** ([`Player::set_exact`]): the output is opened at the file's sample
//! rate and channel count, in 32-bit floats (exact for 24-bit), and the sound goes through no
//! processing (equalizer, leveling, limiter, volume). PipeWire, which usually resamples
//! everything to 48 kHz, is set to the track's rate during playback (`clock.force-rate`, back
//! to normal as soon as SterMusic stops playing).

use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rodio::source::SeekError;
use rodio::{ChannelCount, Decoder, OutputStream, OutputStreamBuilder, SampleRate, Sink, Source};

use crate::dsp::{self, Processor};
use crate::media::{Media, Span};

/// Delay before releasing the audio output when nothing is playing (between two tracks, it stays).
pub const RELEASE_AFTER: Duration = Duration::from_millis(300);

type BoxSource = Box<dyn Source + Send>;

// ---------- Track chain (gapless playback) ----------

/// What the player and the chain playing in the audio output share.
struct ChainShared {
    /// The track queued next: its number and its audio.
    next: Mutex<Option<(u64, BoxSource)>>,
    /// Number of the track playing (0: the first of the chain).
    playing: AtomicU64,
    /// Position in this track (µs).
    position: AtomicU64,
    rate: SampleRate,
    channels: ChannelCount,
}

impl ChainShared {
    fn position(&self) -> Duration {
        Duration::from_micros(self.position.load(Ordering::Relaxed))
    }
}

/// The sound of an output: the current track, then the one queued next, without a single
/// sample of silence in between. Both have the same format (otherwise nothing is queued).
struct Chain {
    current: BoxSource,
    shared: Arc<ChainShared>,
    /// Samples played since the start of the track or the last seek, and the position at that
    /// moment.
    samples: u64,
    base: Duration,
}

impl Chain {
    fn new(first: BoxSource) -> (Self, Arc<ChainShared>) {
        let shared = Arc::new(ChainShared {
            next: Mutex::new(None),
            playing: AtomicU64::new(0),
            position: AtomicU64::new(0),
            rate: first.sample_rate(),
            channels: first.channels(),
        });
        let chain = Self {
            current: first,
            shared: shared.clone(),
            samples: 0,
            base: Duration::ZERO,
        };
        (chain, shared)
    }

    fn store_position(&self) {
        let per_second = (self.shared.rate as u64 * self.shared.channels.max(1) as u64).max(1);
        let micros = self.base.as_micros() as u64 + self.samples * 1_000_000 / per_second;
        self.shared.position.store(micros, Ordering::Relaxed);
    }
}

impl Iterator for Chain {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        loop {
            if let Some(s) = self.current.next() {
                self.samples += 1;
                if self.samples.is_multiple_of(256) {
                    self.store_position();
                }
                return Some(s);
            }
            // End of the track: the next one takes its place right away, if there is one.
            let (id, source) = self.shared.next.lock().ok()?.take()?;
            self.current = source;
            self.samples = 0;
            self.base = Duration::ZERO;
            self.shared.position.store(0, Ordering::Relaxed);
            self.shared.playing.store(id, Ordering::Release);
        }
    }
}

impl Source for Chain {
    fn current_span_len(&self) -> Option<usize> {
        self.current.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.shared.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.shared.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.current.try_seek(pos)?;
        self.samples = 0;
        self.base = pos;
        self.store_position();
        Ok(())
    }
}

// ---------- Player ----------

pub struct Player {
    stream: Option<OutputStream>,
    /// Format of the output opened in bit-perfect mode (rate, channels); `None`: regular output.
    stream_exact: Option<(SampleRate, ChannelCount)>,
    sink: Option<Sink>,
    /// The chain playing in `sink` (position, track queued next).
    chain: Option<Arc<ChainShared>>,
    /// Track queued next in the chain, and its number.
    queued: Option<(u64, Media)>,
    next_id: u64,
    /// Track that could not be queued (different format, unreadable): no retry on every frame.
    refused: Option<Media>,
    /// What is loaded (file, track of a CUE sheet, address).
    media: Option<Media>,
    /// Loaded track whose audio output was released: its position.
    parked: Option<Duration>,
    /// Since when nothing has been playing (pause or stop).
    idle_since: Option<Instant>,
    volume: f32,
    /// Equalizer and leveling (settings shared with the interface).
    audio: dsp::Shared,
    /// Audio copy for the visualizer.
    tap: dsp::SharedTap,
    /// Crossfade: the previous track fading out, since when, and the fade duration.
    fading: Option<(Sink, Instant, Duration)>,
    /// Web radio playing: a live stream doesn't pause, it stops.
    live: bool,
    /// Casting to a network device: everything then goes through it.
    cast: Option<crate::cast::Session>,
    /// What the device displays for the next load.
    meta: crate::cast::Meta,
    /// Bit-perfect output requested.
    exact: bool,
    /// Rate forced on PipeWire (0: none).
    forced_rate: u32,
    /// ReplayGain: album gain rather than track gain (listening to an album in sequence).
    album_gain: bool,
}

impl Player {
    pub fn new(volume: f32, audio: dsp::Shared, tap: dsp::SharedTap) -> Result<Self, String> {
        let mut player = Self {
            stream: None,
            stream_exact: None,
            sink: None,
            chain: None,
            queued: None,
            next_id: 0,
            refused: None,
            media: None,
            parked: None,
            idle_since: None,
            volume,
            audio,
            tap,
            fading: None,
            live: false,
            cast: None,
            meta: Default::default(),
            exact: false,
            forced_rate: 0,
            album_gain: false,
        };
        // Check right away that an audio output exists; it will be released if nothing plays.
        player.stream_for(None)?;
        player.idle_since = Some(Instant::now());
        Ok(player)
    }

    /// The audio output, opened as needed. In bit-perfect mode, `format` (the track's) sets its
    /// rate and channels: an output opened in another format is reopened.
    fn stream_for(&mut self, format: Option<(SampleRate, ChannelCount)>) -> Result<&OutputStream, String> {
        let want = format.filter(|_| self.exact);
        if self.stream.is_some() && self.stream_exact != want {
            self.release_stream();
        }
        if self.stream.is_none() {
            let exact = want.and_then(|(rate, channels)| {
                self.force_rate(rate);
                OutputStreamBuilder::from_default_device()
                    .and_then(|b| {
                        b.with_sample_rate(rate)
                            .with_channels(channels)
                            .with_sample_format(rodio::cpal::SampleFormat::F32)
                            .open_stream()
                    })
                    .ok()
            });
            self.stream_exact = exact.as_ref().and(want);
            let mut stream = match exact {
                Some(s) => s,
                None => {
                    self.force_rate(0);
                    OutputStreamBuilder::open_default_stream()
                        .map_err(|e| crate::trf!("Impossible d'ouvrir la sortie audio : {e}", e))?
                }
            };
            stream.log_on_drop(false);
            self.stream = Some(stream);
        }
        Ok(self.stream.as_ref().unwrap())
    }

    /// Closes the audio output (PipeWire gets its usual rate back).
    fn release_stream(&mut self) {
        self.stream = None;
        self.stream_exact = None;
        self.force_rate(0);
    }

    /// Forces (or restores, with 0) PipeWire's rate.
    fn force_rate(&mut self, rate: u32) {
        if rate != self.forced_rate {
            crate::outputs::force_rate(rate);
            self.forced_rate = rate;
        }
    }

    /// Stops the current output and forgets its chain; returns the position reached.
    fn drop_sink(&mut self) -> Option<Duration> {
        let pos = self.sink.as_ref().map(|_| self.position());
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.chain = None;
        self.queued = None;
        pos
    }

    /// Bit-perfect output: turned on or off, it applies right away (the current track resumes at
    /// the same place).
    pub fn set_exact(&mut self, on: bool) -> Result<(), String> {
        if on == self.exact {
            return Ok(());
        }
        self.exact = on;
        self.reopen()
    }

    /// ReplayGain: album gain (true) or track gain, for the next tracks opened.
    pub fn set_album_gain(&mut self, album: bool) {
        self.album_gain = album;
    }

    // ---------- Casting ----------

    /// Title, artist and album art of the next load (shown by the cast device).
    pub fn set_meta(&mut self, meta: crate::cast::Meta) {
        self.meta = meta;
    }

    pub fn casting(&self) -> Option<&crate::cast::Device> {
        self.cast.as_ref().map(|c| &c.device)
    }

    pub fn cast_error(&self) -> Option<String> {
        self.cast.as_ref().and_then(|c| c.error())
    }

    /// Moves playback to a device: the current track resumes there at the same place.
    pub fn start_cast(&mut self, device: crate::cast::Device) -> Result<(), String> {
        let pos = self.position().as_secs_f32();
        let playing = self.is_playing();
        let media = self.media.clone().filter(|_| !self.live);
        self.stop_cast();
        self.finish_fade();
        self.drop_sink();
        self.release_stream();
        let mut session = crate::cast::Session::start(device)?;
        session.set_volume(self.volume);
        if let Some(m) = &media {
            session.load(m, self.meta.clone(), pos, !playing);
        }
        self.parked = None;
        self.live = false;
        self.media = media;
        self.cast = Some(session);
        Ok(())
    }

    /// Comes back to the computer's speakers, at the same place (paused if the device was).
    pub fn stop_cast(&mut self) {
        let Some(mut session) = self.cast.take() else {
            return;
        };
        let pos = Duration::from_secs_f32(session.position().max(0.0));
        let playing = session.state() == crate::cast::State::Playing;
        session.stop();
        let live = self.live;
        self.live = false;
        if live {
            self.media = None;
            return;
        }
        if let Some(m) = self.media.clone() {
            if playing {
                let _ = self.open(&m, pos);
            } else {
                self.parked = Some(pos);
            }
        }
    }

    /// Web radio being cast: the device plays the stream address itself.
    pub fn cast_live(&mut self, url: &str) {
        if let Some(c) = &mut self.cast {
            let media = Media::url(url, None);
            let mut meta = self.meta.clone();
            meta.live = true;
            c.load(&media, meta, 0.0, false);
            self.media = Some(media);
            self.live = true;
        }
    }

    /// Plays a web radio stream (replaces the current track).
    pub fn play_stream<S>(&mut self, source: S) -> Result<(), String>
    where
        S: rodio::Source + Send + 'static,
    {
        self.stop();
        let source = Processor::new(source, self.audio.clone(), None, Some(self.tap.clone()));
        let sink = Sink::connect_new(self.stream_for(None)?.mixer());
        sink.set_volume(self.volume);
        sink.append(source);
        self.sink = Some(sink);
        self.live = true;
        self.idle_since = None;
        Ok(())
    }

    /// True while listening to a web radio.
    pub fn is_live(&self) -> bool {
        if let Some(c) = &self.cast {
            return c.is_live();
        }
        self.live && self.sink.is_some()
    }

    /// Reopens the audio output (another output was just chosen); the current track resumes at the
    /// same place. A live stream is stopped instead: it is up to the caller to restart it.
    pub fn reopen(&mut self) -> Result<(), String> {
        if self.cast.is_some() {
            return Ok(());
        }
        if self.live {
            self.stop();
            self.release_stream();
            return Ok(());
        }
        let playing = self.is_playing();
        self.finish_fade();
        if let Some(pos) = self.drop_sink() {
            self.parked = Some(pos);
        }
        self.release_stream();
        if playing {
            return self.play();
        }
        Ok(())
    }

    /// Loads a file (replaces the current track), playing or paused.
    pub fn load(&mut self, media: &Media, paused: bool) -> Result<(), String> {
        if let Some(c) = &mut self.cast {
            c.load(media, self.meta.clone(), 0.0, paused);
            self.media = Some(media.clone());
            self.live = false;
            return Ok(());
        }
        self.stop();
        self.media = Some(media.clone());
        if paused {
            // Paused: nothing is opened until playback is started again.
            self.parked = Some(Duration::ZERO);
            return Ok(());
        }
        let media = media.clone();
        self.open(&media, Duration::ZERO)
    }

    /// The audio of `media`, decoded and passed through the processing (except in bit-perfect mode).
    fn source(&self, media: &Media) -> Result<BoxSource, String> {
        let gain = if self.exact {
            None
        } else {
            media.path().and_then(|p| dsp::replaygain(p, self.album_gain))
        };
        let tap = Some(self.tap.clone());
        if media.needs_ffmpeg() {
            let total = media
                .end
                .map(|e| e.saturating_sub(media.start))
                .or(media.duration);
            let input = media.ffmpeg_input();
            let source = if self.exact {
                crate::ffdecode::FfmpegSource::open_native(&input, media.start, total)?
            } else {
                crate::ffdecode::FfmpegSource::open(&input, media.start, total)?
            };
            let source = Span::new(source, Duration::ZERO, total);
            return Ok(Box::new(Processor::new(source, self.audio.clone(), gain, tap)));
        }
        let path = media.path().unwrap_or(Path::new(""));
        let file = File::open(path).map_err(|e| crate::trf!("Ouverture impossible : {e}", e))?;
        // `try_from(File)` gives the decoder the file size: without it, Symphonia fails (or even
        // panics, with rodio 0.20) on M4A/AAC. rodio's decoders also remove the silence added by the
        // encoder at the start and end of MP3 and AAC files.
        // Safety net: a decoder panicking on an exotic file must not take the whole application down
        // with it.
        let source = catch_unwind(AssertUnwindSafe(|| Decoder::try_from(file)))
            .map_err(|_| crate::i18n::tr("Format non pris en charge (le décodeur a échoué)").to_owned())?
            .map_err(|e| crate::trf!("Format non lisible : {e}", e))?;
        // Track of a CUE sheet: cut between its start and its end.
        let source = Span::new(source, media.start, media.end);
        Ok(Box::new(Processor::new(source, self.audio.clone(), gain, tap)))
    }

    fn open(&mut self, media: &Media, position: Duration) -> Result<(), String> {
        self.live = false;
        self.refused = None;
        let source = self.source(media)?;
        let format = (source.sample_rate(), source.channels());
        let volume = self.volume;
        let sink = Sink::connect_new(self.stream_for(Some(format))?.mixer());
        sink.set_volume(volume);
        let (chain, shared) = Chain::new(source);
        sink.append(chain);
        if !position.is_zero() {
            let _ = sink.try_seek(position);
            shared.position.store(position.as_micros() as u64, Ordering::Relaxed);
        }
        self.sink = Some(sink);
        self.chain = Some(shared);
        self.queued = None;
        self.parked = None;
        self.idle_since = None;
        Ok(())
    }

    // ---------- Gapless playback ----------

    /// Queues `media` after the current track: it will start without the slightest gap. False if
    /// that's not possible (casting, live stream, paused, format different from the current track,
    /// unreadable file): the transition then happens as usual.
    pub fn queue_next(&mut self, media: &Media) -> bool {
        if self.cast.is_some() || self.live || self.fading.is_some() {
            return false;
        }
        if self.queued.as_ref().is_some_and(|(_, m)| m == media) {
            return true;
        }
        if self.refused.as_ref() == Some(media) {
            return false;
        }
        let (Some(sink), Some(chain)) = (&self.sink, self.chain.clone()) else {
            return false;
        };
        if sink.is_paused() || sink.empty() {
            return false;
        }
        let source = match self.source(media) {
            Ok(s) if s.sample_rate() == chain.rate && s.channels() == chain.channels => s,
            _ => {
                self.clear_next();
                self.refused = Some(media.clone());
                return false;
            }
        };
        self.next_id += 1;
        if let Ok(mut slot) = chain.next.lock() {
            *slot = Some((self.next_id, source));
        }
        self.queued = Some((self.next_id, media.clone()));
        true
    }

    /// Removes the track queued next (the queue changed).
    pub fn clear_next(&mut self) {
        if let Some(chain) = &self.chain {
            if let Ok(mut slot) = chain.next.lock() {
                *slot = None;
            }
        }
        self.queued = None;
    }

    /// The track queued next.
    pub fn queued(&self) -> Option<&Media> {
        self.queued.as_ref().map(|(_, m)| m)
    }

    /// The track queued next just started: here it is (to be called every frame).
    pub fn take_started(&mut self) -> Option<Media> {
        let chain = self.chain.as_ref()?;
        let (id, _) = self.queued.as_ref()?;
        if chain.playing.load(Ordering::Acquire) != *id {
            return None;
        }
        let (_, media) = self.queued.take()?;
        self.media = Some(media.clone());
        Some(media)
    }

    /// Crossfade: `media` starts silent and rises while the current track fades out.
    pub fn crossfade_to(&mut self, media: &Media, duration: Duration) -> Result<(), String> {
        if self.cast.is_some() || self.exact {
            return self.load(media, false);
        }
        // The fading track must not chain into another one.
        self.clear_next();
        let Some(old) = self.sink.take().filter(|s| !s.is_paused()) else {
            return self.load(media, false);
        };
        self.chain = None;
        self.finish_fade();
        self.media = Some(media.clone());
        match self.open(media, Duration::ZERO) {
            Ok(()) => {
                if let Some(sink) = &self.sink {
                    sink.set_volume(0.0);
                }
                self.fading = Some((old, Instant::now(), duration));
                Ok(())
            }
            Err(e) => {
                old.stop();
                Err(e)
            }
        }
    }

    /// True during a crossfade.
    pub fn is_fading(&self) -> bool {
        self.fading.is_some()
    }

    fn finish_fade(&mut self) {
        if let Some((old, ..)) = self.fading.take() {
            old.stop();
        }
        if let Some(sink) = &self.sink {
            sink.set_volume(self.volume);
        }
    }

    /// To be called regularly: advances the crossfade, and releases the audio output when nothing
    /// has been playing for a while.
    pub fn tick(&mut self) {
        if self.cast.is_some() {
            return;
        }
        if let Some((old, start, duration)) = &self.fading {
            let t = (start.elapsed().as_secs_f32() / duration.as_secs_f32().max(0.01)).clamp(0.0, 1.0);
            if t >= 1.0 {
                self.finish_fade();
            } else {
                // Constant power curves: no volume dip in the middle of the fade.
                let angle = t * std::f32::consts::FRAC_PI_2;
                old.set_volume(self.volume * angle.cos());
                if let Some(sink) = &self.sink {
                    sink.set_volume(self.volume * angle.sin());
                }
            }
        }
        let idle = match &self.sink {
            Some(sink) => sink.is_paused(),
            None => true,
        };
        if !idle {
            self.idle_since = None;
            return;
        }
        let since = *self.idle_since.get_or_insert_with(Instant::now);
        if since.elapsed() < RELEASE_AFTER {
            return;
        }
        if let Some(pos) = self.drop_sink() {
            self.parked = Some(pos);
        }
        self.release_stream();
    }

    /// True if the audio output is open without playing anything (it will soon be released).
    pub fn holds_idle_output(&self) -> bool {
        if self.cast.is_some() {
            return false;
        }
        self.stream.is_some() && !self.is_playing()
    }

    /// The loaded file was renamed: resuming after a pause will reopen it under its new name.
    pub fn renamed(&mut self, old: &Path, new: &Path) {
        if let Some(m) = &mut self.media {
            if m.path() == Some(old) {
                m.input = crate::media::Input::File(new.to_path_buf());
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(c) = &mut self.cast {
            c.stop();
            self.media = None;
            self.live = false;
            return;
        }
        self.finish_fade();
        self.drop_sink();
        self.parked = None;
        self.media = None;
        self.live = false;
        self.idle_since = Some(Instant::now());
    }

    pub fn play(&mut self) -> Result<(), String> {
        if let Some(c) = &mut self.cast {
            match c.state() {
                crate::cast::State::Paused | crate::cast::State::Loading | crate::cast::State::Playing => {
                    c.play()
                }
                // Finished or never loaded: reload.
                _ => {
                    if let Some(m) = self.media.clone() {
                        c.load(&m, self.meta.clone(), 0.0, false);
                    }
                }
            }
            return Ok(());
        }
        if let (Some(position), Some(media)) = (self.parked, self.media.clone()) {
            return self.open(&media, position);
        }
        if let Some(sink) = &self.sink {
            sink.play();
            self.idle_since = None;
        }
        Ok(())
    }

    /// Pauses and immediately releases the audio output: Plasma's audio indicator goes off.
    pub fn pause(&mut self) {
        if let Some(c) = &mut self.cast {
            if self.live {
                c.stop();
                self.media = None;
                self.live = false;
            } else {
                c.pause();
            }
            return;
        }
        if self.live {
            self.stop();
            self.release_stream();
            return;
        }
        self.finish_fade();
        if let Some(pos) = self.drop_sink() {
            self.parked = Some(pos);
        }
        self.release_stream();
        self.idle_since = Some(Instant::now());
    }

    pub fn is_paused(&self) -> bool {
        if let Some(c) = &self.cast {
            return !matches!(
                c.state(),
                crate::cast::State::Playing | crate::cast::State::Loading
            );
        }
        match &self.sink {
            Some(sink) => sink.is_paused(),
            None => true,
        }
    }

    /// True if a track is loaded (playing or paused).
    pub fn has_track(&self) -> bool {
        if let Some(c) = &self.cast {
            return c.has_media();
        }
        self.sink.is_some() || self.parked.is_some()
    }

    pub fn is_playing(&self) -> bool {
        if let Some(c) = &self.cast {
            return c.has_media()
                && matches!(
                    c.state(),
                    crate::cast::State::Playing | crate::cast::State::Loading
                );
        }
        self.sink.as_ref().is_some_and(|s| !s.is_paused())
    }

    /// True if the loaded track has reached the end.
    pub fn finished(&self) -> bool {
        if let Some(c) = &self.cast {
            return c.has_media() && c.state() == crate::cast::State::Finished;
        }
        self.sink.as_ref().is_some_and(|s| s.empty())
    }

    pub fn position(&self) -> Duration {
        if let Some(c) = &self.cast {
            return Duration::from_secs_f32(c.position().max(0.0));
        }
        match (&self.sink, self.parked) {
            (Some(_), _) if self.chain.is_some() => self.chain.as_ref().unwrap().position(),
            (Some(sink), _) => sink.get_pos(),
            (None, Some(pos)) => pos,
            _ => Duration::ZERO,
        }
    }

    pub fn seek(&mut self, pos: Duration) -> Result<(), String> {
        if let Some(c) = &mut self.cast {
            c.seek(pos.as_secs_f32());
            return Ok(());
        }
        if let Some(sink) = &self.sink {
            return sink
                .try_seek(pos)
                .map_err(|e| crate::trf!("Déplacement impossible : {e}", e));
        }
        if self.parked.is_some() {
            self.parked = Some(pos);
        }
        Ok(())
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        if let Some(c) = &self.cast {
            c.set_volume(volume);
            return;
        }
        if self.fading.is_some() {
            return; // the fade applies the new volume at the next step
        }
        if let Some(sink) = &self.sink {
            sink.set_volume(volume);
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // PipeWire gets its usual rate back even if SterMusic closes in the middle of playback.
        self.force_rate(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;

    /// The track queued next starts right after the last sample of the first one: nothing is lost
    /// or added, and the position starts again from zero.
    #[test]
    fn chain_is_gapless() {
        let a: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0).collect();
        let b: Vec<f32> = (0..500).map(|i| -(i as f32) / 500.0).collect();
        let (mut chain, shared) = Chain::new(Box::new(SamplesBuffer::new(2, 44_100, a.clone())));
        *shared.next.lock().unwrap() = Some((7, Box::new(SamplesBuffer::new(2, 44_100, b.clone()))));
        let first: Vec<f32> = chain.by_ref().take(a.len()).collect();
        assert_eq!(first, a);
        assert_eq!(shared.playing.load(Ordering::Acquire), 0);
        let second: Vec<f32> = chain.by_ref().collect();
        assert_eq!(second, b);
        assert_eq!(shared.playing.load(Ordering::Acquire), 7);
        // 500 stereo samples at 44.1 kHz: 250 frames, i.e. 5.67 ms (checked every 256).
        assert!(shared.position() <= Duration::from_micros(5_700));
    }

    /// Without a track queued next, the chain stops with the first one.
    #[test]
    fn chain_ends_alone() {
        let (chain, _) = Chain::new(Box::new(SamplesBuffer::new(1, 48_000, vec![0.5; 10])));
        assert_eq!(chain.count(), 10);
    }

    /// Beyond 100 %: a quiet sound is amplified accordingly, a loud sound is held back by the
    /// limiter (never above 1), and at 100 % nothing changes.
    #[test]
    fn boost_amplifies_without_clipping() {
        let quiet: Vec<f32> = (0..2000).map(|i| ((i as f32) * 0.21).sin() * 0.2).collect();
        let loud: Vec<f32> = (0..2000).map(|i| ((i as f32) * 0.21).sin() * 0.95).collect();
        let run = |input: &Vec<f32>, boost: f32| -> Vec<f32> {
            let settings = dsp::AudioSettings::new();
            settings.set_boost(boost);
            Processor::new(SamplesBuffer::new(2, 44_100, input.clone()), settings, None, None).collect()
        };
        assert_eq!(run(&quiet, 1.0), quiet);
        let boosted = run(&quiet, 2.25);
        for (o, i) in boosted.iter().zip(&quiet) {
            assert!((o - i * 2.25).abs() < 1e-5);
        }
        let limited = run(&loud, 2.25);
        let peak = limited.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(peak <= 1.0 && peak > 0.97, "{peak}");
    }

    /// In bit-perfect mode, the processing returns the sound exactly as it came in (even beyond
    /// the limiter threshold), equalizer and leveling on or off.
    #[test]
    fn exact_is_untouched() {
        let input: Vec<f32> = (0..4000).map(|i| ((i as f32) * 0.37).sin() * 0.999).collect();
        let settings = dsp::AudioSettings::new();
        settings.set(true, &[6.0; 10], 3.0, true);
        settings.set_exact(true);
        let p = Processor::new(SamplesBuffer::new(2, 44_100, input.clone()), settings.clone(), Some(-6.0), None);
        let out: Vec<f32> = p.collect();
        assert_eq!(out, input);
        // And without bit-perfect, the same setting does change the sound.
        settings.set_exact(false);
        let p = Processor::new(SamplesBuffer::new(2, 44_100, input.clone()), settings, Some(-6.0), None);
        let out: Vec<f32> = p.collect();
        assert_ne!(out, input);
    }
}
