//! Moteur audio : une fine couche au-dessus de rodio.
//!
//! En pause, la sortie audio est libérée tout de suite (à l'arrêt, après un court délai, pour
//! ne pas couper entre deux morceaux) : sinon PipeWire voit un flux actif (du silence) et Plasma
//! affiche SterMusic « en lecture ». À la reprise, le morceau est rouvert au même endroit.

use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink};

use crate::dsp::{self, Processor};

/// Délai avant de libérer la sortie audio quand rien ne joue (entre deux morceaux, elle reste).
pub const RELEASE_AFTER: Duration = Duration::from_millis(300);

pub struct Player {
    stream: Option<OutputStream>,
    sink: Option<Sink>,
    path: Option<PathBuf>,
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
}

impl Player {
    pub fn new(volume: f32, audio: dsp::Shared, tap: dsp::SharedTap) -> Result<Self, String> {
        let mut player = Self {
            stream: None,
            sink: None,
            path: None,
            parked: None,
            idle_since: None,
            volume,
            audio,
            tap,
            fading: None,
            live: false,
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
        self.live && self.sink.is_some()
    }

    /// Rouvre la sortie audio (une autre sortie vient d'être choisie) ; le morceau en cours
    /// reprend au même endroit. Un direct, lui, est arrêté : c'est à l'appelant de le relancer.
    pub fn reopen(&mut self) -> Result<(), String> {
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
    pub fn load(&mut self, path: &Path, paused: bool) -> Result<(), String> {
        self.stop();
        self.path = Some(path.to_path_buf());
        if paused {
            // En pause : rien n'est ouvert tant qu'on ne relance pas la lecture.
            self.parked = Some(Duration::ZERO);
            return Ok(());
        }
        self.open(path, Duration::ZERO)
    }

    fn open(&mut self, path: &Path, position: Duration) -> Result<(), String> {
        self.live = false;
        let file = File::open(path).map_err(|e| crate::trf!("Ouverture impossible : {e}", e))?;
        // `try_from(File)` donne au décodeur la taille du fichier et son extension : sans elles,
        // Symphonia échoue (voire panique, avec rodio 0.20) sur les M4A/AAC.
        // Filet de sécurité : un décodeur qui panique sur un fichier exotique ne doit pas
        // emporter toute l'application.
        let source = catch_unwind(AssertUnwindSafe(|| Decoder::try_from(file)))
            .map_err(|_| crate::i18n::tr("Format non pris en charge (le décodeur a échoué)").to_owned())?
            .map_err(|e| crate::trf!("Format non lisible : {e}", e))?;
        let source = Processor::new(
            source,
            self.audio.clone(),
            dsp::replaygain(path),
            Some(self.tap.clone()),
        );
        let volume = self.volume;
        let sink = Sink::connect_new(self.stream()?.mixer());
        sink.set_volume(volume);
        sink.append(source);
        if !position.is_zero() {
            let _ = sink.try_seek(position);
        }
        self.sink = Some(sink);
        self.parked = None;
        self.idle_since = None;
        Ok(())
    }

    /// Fondu enchaîné : `path` démarre en silence et monte pendant que le morceau en cours s'efface.
    pub fn crossfade_to(&mut self, path: &Path, duration: Duration) -> Result<(), String> {
        let Some(old) = self.sink.take().filter(|s| !s.is_paused()) else {
            return self.load(path, false);
        };
        self.finish_fade();
        self.path = Some(path.to_path_buf());
        match self.open(path, Duration::ZERO) {
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
        self.stream.is_some() && !self.is_playing()
    }

    /// Le fichier chargé a été renommé : une reprise après une pause le rouvrira sous son
    /// nouveau nom.
    pub fn renamed(&mut self, old: &Path, new: &Path) {
        if self.path.as_deref() == Some(old) {
            self.path = Some(new.to_path_buf());
        }
    }

    pub fn stop(&mut self) {
        self.finish_fade();
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.parked = None;
        self.path = None;
        self.live = false;
        self.idle_since = Some(Instant::now());
    }

    pub fn play(&mut self) -> Result<(), String> {
        if let (Some(position), Some(path)) = (self.parked, self.path.clone()) {
            return self.open(&path, position);
        }
        if let Some(sink) = &self.sink {
            sink.play();
            self.idle_since = None;
        }
        Ok(())
    }

    /// Met en pause et libère aussitôt la sortie audio : l'indicateur de son de Plasma s'éteint.
    pub fn pause(&mut self) {
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
        match &self.sink {
            Some(sink) => sink.is_paused(),
            None => true,
        }
    }

    /// Vrai si un morceau est chargé (en lecture ou en pause).
    pub fn has_track(&self) -> bool {
        self.sink.is_some() || self.parked.is_some()
    }

    pub fn is_playing(&self) -> bool {
        self.sink.as_ref().is_some_and(|s| !s.is_paused())
    }

    /// Vrai si le morceau chargé est arrivé à la fin.
    pub fn finished(&self) -> bool {
        self.sink.as_ref().is_some_and(|s| s.empty())
    }

    pub fn position(&self) -> Duration {
        match (&self.sink, self.parked) {
            (Some(sink), _) => sink.get_pos(),
            (None, Some(pos)) => pos,
            _ => Duration::ZERO,
        }
    }

    pub fn seek(&mut self, pos: Duration) -> Result<(), String> {
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
        if self.fading.is_some() {
            return; // le fondu applique le nouveau volume à la prochaine étape
        }
        if let Some(sink) = &self.sink {
            sink.set_volume(volume);
        }
    }
}
