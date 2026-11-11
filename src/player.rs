//! Moteur audio : une fine couche au-dessus de rodio.
//!
//! En pause, la sortie audio est libérée tout de suite (à l'arrêt, après un court délai, pour
//! ne pas couper entre deux morceaux) : sinon PipeWire voit un flux actif (du silence) et Plasma
//! affiche SterMusic « en lecture ». À la reprise, le morceau est rouvert au même endroit.
//!
//! **Lecture sans blanc** : le son d'une sortie est une [`Chain`]. Quelques secondes avant la
//! fin d'un morceau, l'application y met le suivant ([`Player::queue_next`]) : il démarre à
//! l'échantillon près quand le premier s'arrête, sans rouvrir ni la sortie ni le décodeur.
//!
//! **Sortie bit-perfect** ([`Player::set_exact`]) : la sortie est ouverte à la fréquence et au
//! nombre de canaux du fichier, en flottants 32 bits (exacts pour du 24 bits), et le son ne
//! passe par aucun traitement (égaliseur, nivellement, limiteur, volume). PipeWire, qui
//! rééchantillonne d'habitude tout vers 48 kHz, est réglé sur la fréquence du morceau pendant
//! la lecture (`clock.force-rate`, rendu à la normale dès que SterMusic ne joue plus).

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

/// Délai avant de libérer la sortie audio quand rien ne joue (entre deux morceaux, elle reste).
pub const RELEASE_AFTER: Duration = Duration::from_millis(300);

type BoxSource = Box<dyn Source + Send>;

// ---------- Chaîne de morceaux (lecture sans blanc) ----------

/// Ce que le lecteur et la chaîne qui joue dans la sortie audio partagent.
struct ChainShared {
    /// Le morceau mis à la suite : son numéro et son son.
    next: Mutex<Option<(u64, BoxSource)>>,
    /// Numéro du morceau qui joue (0 : le premier de la chaîne).
    playing: AtomicU64,
    /// Position dans ce morceau (µs).
    position: AtomicU64,
    rate: SampleRate,
    channels: ChannelCount,
}

impl ChainShared {
    fn position(&self) -> Duration {
        Duration::from_micros(self.position.load(Ordering::Relaxed))
    }
}

/// Le son d'une sortie : le morceau en cours, puis celui mis à la suite, sans un échantillon de
/// silence entre les deux. Les deux ont le même format (sinon rien n'est mis à la suite).
struct Chain {
    current: BoxSource,
    shared: Arc<ChainShared>,
    /// Échantillons joués depuis le début du morceau ou le dernier déplacement, et la position
    /// à ce moment-là.
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
            // Fin du morceau : le suivant prend sa place tout de suite, s'il y en a un.
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

// ---------- Lecteur ----------

pub struct Player {
    stream: Option<OutputStream>,
    /// Format de la sortie ouverte en bit-perfect (fréquence, canaux) ; `None` : sortie ordinaire.
    stream_exact: Option<(SampleRate, ChannelCount)>,
    sink: Option<Sink>,
    /// La chaîne qui joue dans `sink` (position, morceau mis à la suite).
    chain: Option<Arc<ChainShared>>,
    /// Morceau mis à la suite dans la chaîne, et son numéro.
    queued: Option<(u64, Media)>,
    next_id: u64,
    /// Morceau qui n'a pas pu être mis à la suite (format différent, illisible) : on ne
    /// réessaie pas à chaque image.
    refused: Option<Media>,
    /// Ce qui est chargé (fichier, morceau d'un fichier CUE, adresse).
    media: Option<Media>,
    /// Morceau chargé dont la sortie audio a été libérée : sa position.
    parked: Option<Duration>,
    /// Depuis quand rien ne joue (pause ou arrêt).
    idle_since: Option<Instant>,
    volume: f32,
    /// Égaliseur et nivellement (réglages partagés avec l'interface).
    audio: dsp::Shared,
    /// Copie du son pour le visualiseur.
    tap: dsp::SharedTap,
    /// Fondu enchaîné : le morceau précédent qui s'efface, depuis quand, et la durée du fondu.
    fading: Option<(Sink, Instant, Duration)>,
    /// Webradio en cours : un direct ne se met pas en pause, il s'arrête.
    live: bool,
    /// Diffusion vers un appareil du réseau : tout passe alors par lui.
    cast: Option<crate::cast::Session>,
    /// Ce que l'appareil affiche pour le prochain chargement.
    meta: crate::cast::Meta,
    /// Sortie bit-perfect demandée.
    exact: bool,
    /// Fréquence imposée à PipeWire (0 : aucune).
    forced_rate: u32,
    /// ReplayGain : gain de l'album plutôt que celui du morceau (écoute d'un album à la suite).
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
        // On vérifie tout de suite qu'une sortie audio existe ; elle sera libérée si rien ne joue.
        player.stream_for(None)?;
        player.idle_since = Some(Instant::now());
        Ok(player)
    }

    /// La sortie audio, ouverte au besoin. En bit-perfect, `format` (celui du morceau) fixe sa
    /// fréquence et ses canaux : une sortie ouverte dans un autre format est rouverte.
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

    /// Ferme la sortie audio (PipeWire retrouve sa fréquence habituelle).
    fn release_stream(&mut self) {
        self.stream = None;
        self.stream_exact = None;
        self.force_rate(0);
    }

    /// Impose (ou rend, avec 0) la fréquence de PipeWire.
    fn force_rate(&mut self, rate: u32) {
        if rate != self.forced_rate {
            crate::outputs::force_rate(rate);
            self.forced_rate = rate;
        }
    }

    /// Arrête la sortie en cours et oublie sa chaîne ; rend la position atteinte.
    fn drop_sink(&mut self) -> Option<Duration> {
        let pos = self.sink.as_ref().map(|_| self.position());
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.chain = None;
        self.queued = None;
        pos
    }

    /// Sortie bit-perfect : activée ou coupée, elle s'applique tout de suite (le morceau en cours
    /// reprend au même endroit).
    pub fn set_exact(&mut self, on: bool) -> Result<(), String> {
        if on == self.exact {
            return Ok(());
        }
        self.exact = on;
        self.reopen()
    }

    /// ReplayGain : gain de l'album (vrai) ou du morceau, pour les prochains morceaux ouverts.
    pub fn set_album_gain(&mut self, album: bool) {
        self.album_gain = album;
    }

    // ---------- Diffusion ----------

    /// Titre, artiste et pochette du prochain chargement (affichés par l'appareil diffusé).
    pub fn set_meta(&mut self, meta: crate::cast::Meta) {
        self.meta = meta;
    }

    pub fn casting(&self) -> Option<&crate::cast::Device> {
        self.cast.as_ref().map(|c| &c.device)
    }

    pub fn cast_error(&self) -> Option<String> {
        self.cast.as_ref().and_then(|c| c.error())
    }

    /// Passe la lecture sur un appareil : le morceau en cours y reprend au même endroit.
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

    /// Revient sur les haut-parleurs de l'ordinateur, au même endroit (en pause si l'appareil l'était).
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

    /// Webradio diffusée : l'appareil lit lui-même l'adresse du flux.
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

    /// Joue un flux de webradio (remplace le morceau en cours).
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

    /// Vrai pendant l'écoute d'une webradio.
    pub fn is_live(&self) -> bool {
        if let Some(c) = &self.cast {
            return c.is_live();
        }
        self.live && self.sink.is_some()
    }

    /// Rouvre la sortie audio (une autre sortie vient d'être choisie) ; le morceau en cours
    /// reprend au même endroit. Un direct, lui, est arrêté : c'est à l'appelant de le relancer.
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

    /// Charge un fichier (remplace le morceau en cours), en lecture ou en pause.
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
            // En pause : rien n'est ouvert tant qu'on ne relance pas la lecture.
            self.parked = Some(Duration::ZERO);
            return Ok(());
        }
        let media = media.clone();
        self.open(&media, Duration::ZERO)
    }

    /// Le son de `media`, décodé et passé par le traitement (sauf en bit-perfect).
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
        // `try_from(File)` donne au décodeur la taille du fichier : sans elle, Symphonia échoue
        // (voire panique, avec rodio 0.20) sur les M4A/AAC. Les décodeurs de rodio retirent
        // aussi le silence ajouté par l'encodeur au début et à la fin des MP3 et des AAC.
        // Filet de sécurité : un décodeur qui panique sur un fichier exotique ne doit pas
        // emporter toute l'application.
        let source = catch_unwind(AssertUnwindSafe(|| Decoder::try_from(file)))
            .map_err(|_| crate::i18n::tr("Format non pris en charge (le décodeur a échoué)").to_owned())?
            .map_err(|e| crate::trf!("Format non lisible : {e}", e))?;
        // Morceau d'un fichier CUE : découpé entre son début et sa fin.
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

    // ---------- Lecture sans blanc ----------

    /// Met `media` à la suite du morceau en cours : il démarrera sans le moindre blanc. Faux si
    /// ce n'est pas possible (diffusion, direct, pause, format différent du morceau en cours,
    /// fichier illisible) : le passage se fera alors comme d'habitude.
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

    /// Retire le morceau mis à la suite (la file d'attente a changé).
    pub fn clear_next(&mut self) {
        if let Some(chain) = &self.chain {
            if let Ok(mut slot) = chain.next.lock() {
                *slot = None;
            }
        }
        self.queued = None;
    }

    /// Le morceau mis à la suite.
    pub fn queued(&self) -> Option<&Media> {
        self.queued.as_ref().map(|(_, m)| m)
    }

    /// Le morceau mis à la suite vient de commencer : le voici (à appeler à chaque image).
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

    /// Fondu enchaîné : `media` démarre en silence et monte pendant que le morceau en cours s'efface.
    pub fn crossfade_to(&mut self, media: &Media, duration: Duration) -> Result<(), String> {
        if self.cast.is_some() || self.exact {
            return self.load(media, false);
        }
        // Le morceau qui s'efface ne doit pas enchaîner sur un autre.
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

    /// Vrai pendant un fondu enchaîné.
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

    /// À appeler régulièrement : fait avancer le fondu enchaîné, et libère la sortie audio quand
    /// rien ne joue depuis un moment.
    pub fn tick(&mut self) {
        if self.cast.is_some() {
            return;
        }
        if let Some((old, start, duration)) = &self.fading {
            let t = (start.elapsed().as_secs_f32() / duration.as_secs_f32().max(0.01)).clamp(0.0, 1.0);
            if t >= 1.0 {
                self.finish_fade();
            } else {
                // Courbes à puissance constante : pas de creux de volume au milieu du fondu.
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

    /// Vrai si la sortie audio est ouverte sans rien jouer (elle sera bientôt libérée).
    pub fn holds_idle_output(&self) -> bool {
        if self.cast.is_some() {
            return false;
        }
        self.stream.is_some() && !self.is_playing()
    }

    /// Le fichier chargé a été renommé : une reprise après une pause le rouvrira sous son
    /// nouveau nom.
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
                // Fini ou jamais chargé : on recharge.
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

    /// Met en pause et libère aussitôt la sortie audio : l'indicateur de son de Plasma s'éteint.
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

    /// Vrai si un morceau est chargé (en lecture ou en pause).
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

    /// Vrai si le morceau chargé est arrivé à la fin.
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
            return; // le fondu applique le nouveau volume à la prochaine étape
        }
        if let Some(sink) = &self.sink {
            sink.set_volume(volume);
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // PipeWire retrouve sa fréquence habituelle même si SterMusic se ferme en pleine lecture.
        self.force_rate(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;

    /// Le morceau mis à la suite démarre juste après le dernier échantillon du premier : rien
    /// n'est perdu ni ajouté, et la position repart de zéro.
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
        // 500 échantillons stéréo à 44,1 kHz : 250 trames, soit 5,67 ms (relevé tous les 256).
        assert!(shared.position() <= Duration::from_micros(5_700));
    }

    /// Sans morceau à la suite, la chaîne s'arrête avec le premier.
    #[test]
    fn chain_ends_alone() {
        let (chain, _) = Chain::new(Box::new(SamplesBuffer::new(1, 48_000, vec![0.5; 10])));
        assert_eq!(chain.count(), 10);
    }

    /// Au-delà de 100 % : un son faible est amplifié d'autant, un son fort est retenu par le
    /// limiteur (jamais au-dessus de 1), et à 100 % rien ne change.
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

    /// En bit-perfect, le traitement rend le son exactement tel qu'il est entré (même au-delà
    /// du seuil du limiteur), égaliseur et nivellement allumés ou non.
    #[test]
    fn exact_is_untouched() {
        let input: Vec<f32> = (0..4000).map(|i| ((i as f32) * 0.37).sin() * 0.999).collect();
        let settings = dsp::AudioSettings::new();
        settings.set(true, &[6.0; 10], 3.0, true);
        settings.set_exact(true);
        let p = Processor::new(SamplesBuffer::new(2, 44_100, input.clone()), settings.clone(), Some(-6.0), None);
        let out: Vec<f32> = p.collect();
        assert_eq!(out, input);
        // Et sans bit-perfect, ce même réglage change bien le son.
        settings.set_exact(false);
        let p = Processor::new(SamplesBuffer::new(2, 44_100, input.clone()), settings, Some(-6.0), None);
        let out: Vec<f32> = p.collect();
        assert_ne!(out, input);
    }
}
