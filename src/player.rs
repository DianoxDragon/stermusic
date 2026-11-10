//! Moteur audio : une fine couche au-dessus de rodio.
//!
//! En pause, la sortie audio est libérée tout de suite (à l'arrêt, après un court délai, pour
//! ne pas couper entre deux morceaux) : sinon PipeWire voit un flux actif (du silence) et Plasma
//! affiche SterMusic « en lecture ». À la reprise, le morceau est rouvert au même endroit.

use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::time::{Duration, Instant};

use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink};

use crate::dsp::{self, Processor};
use crate::media::{Media, Span};

/// Délai avant de libérer la sortie audio quand rien ne joue (entre deux morceaux, elle reste).
pub const RELEASE_AFTER: Duration = Duration::from_millis(300);

pub struct Player {
    stream: Option<OutputStream>,
    sink: Option<Sink>,
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
}

impl Player {
    pub fn new(volume: f32, audio: dsp::Shared, tap: dsp::SharedTap) -> Result<Self, String> {
        let mut player = Self {
            stream: None,
            sink: None,
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
        };
        // On vérifie tout de suite qu'une sortie audio existe ; elle sera libérée si rien ne joue.
        player.stream()?;
        player.idle_since = Some(Instant::now());
        Ok(player)
    }

    fn stream(&mut self) -> Result<&OutputStream, String> {
        if self.stream.is_none() {
            let mut stream = OutputStreamBuilder::open_default_stream()
                .map_err(|e| crate::trf!("Impossible d'ouvrir la sortie audio : {e}", e))?;
            stream.log_on_drop(false);
            self.stream = Some(stream);
        }
        Ok(self.stream.as_ref().unwrap())
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
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.stream = None;
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
        let sink = Sink::connect_new(self.stream()?.mixer());
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
            self.stream = None;
            return Ok(());
        }
        let playing = self.is_playing();
        self.finish_fade();
        if let Some(sink) = self.sink.take() {
            self.parked = Some(sink.get_pos());
            sink.stop();
        }
        self.stream = None;
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

    fn open(&mut self, media: &Media, position: Duration) -> Result<(), String> {
        self.live = false;
        let gain = media.path().and_then(dsp::replaygain);
        let volume = self.volume;
        let sink = Sink::connect_new(self.stream()?.mixer());
        sink.set_volume(volume);
        if media.needs_ffmpeg() {
            let total = media
                .end
                .map(|e| e.saturating_sub(media.start))
                .or(media.duration);
            let source = crate::ffdecode::FfmpegSource::open(&media.ffmpeg_input(), media.start, total)?;
            let source = Span::new(source, Duration::ZERO, total);
            sink.append(Processor::new(
                source,
                self.audio.clone(),
                gain,
                Some(self.tap.clone()),
            ));
        } else {
            let path = media.path().unwrap_or(Path::new(""));
            let file = File::open(path).map_err(|e| crate::trf!("Ouverture impossible : {e}", e))?;
            // `try_from(File)` donne au décodeur la taille du fichier et son extension : sans elles,
            // Symphonia échoue (voire panique, avec rodio 0.20) sur les M4A/AAC.
            // Filet de sécurité : un décodeur qui panique sur un fichier exotique ne doit pas
            // emporter toute l'application.
            let source = catch_unwind(AssertUnwindSafe(|| Decoder::try_from(file)))
                .map_err(|_| crate::i18n::tr("Format non pris en charge (le décodeur a échoué)").to_owned())?
                .map_err(|e| crate::trf!("Format non lisible : {e}", e))?;
            // Morceau d'un fichier CUE : découpé entre son début et sa fin.
            let source = Span::new(source, media.start, media.end);
            sink.append(Processor::new(
                source,
                self.audio.clone(),
                gain,
                Some(self.tap.clone()),
            ));
        }
        if !position.is_zero() {
            let _ = sink.try_seek(position);
        }
        self.sink = Some(sink);
        self.parked = None;
        self.idle_since = None;
        Ok(())
    }

    /// Fondu enchaîné : `media` démarre en silence et monte pendant que le morceau en cours s'efface.
    pub fn crossfade_to(&mut self, media: &Media, duration: Duration) -> Result<(), String> {
        if self.cast.is_some() {
            return self.load(media, false);
        }
        let Some(old) = self.sink.take().filter(|s| !s.is_paused()) else {
            return self.load(media, false);
        };
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
        if let Some(sink) = self.sink.take() {
            self.parked = Some(sink.get_pos());
            sink.stop();
        }
        self.stream = None;
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
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
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
            self.stream = None;
            return;
        }
        self.finish_fade();
        if let Some(sink) = self.sink.take() {
            self.parked = Some(sink.get_pos());
            sink.stop();
        }
        self.stream = None;
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
