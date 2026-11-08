//! Moteur audio : une fine couche au-dessus de rodio.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};

pub struct Player {
    // Le flux de sortie doit rester vivant tant qu'on joue du son.
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Option<Sink>,
    volume: f32,
}

impl Player {
    pub fn new(volume: f32) -> Result<Self, String> {
        let (stream, handle) = OutputStream::try_default()
            .map_err(|e| format!("Impossible d'ouvrir la sortie audio : {e}"))?;
        Ok(Self {
            _stream: stream,
            handle,
            sink: None,
            volume,
        })
    }

    /// Charge un fichier (remplace le morceau en cours), en lecture ou en pause.
    pub fn load(&mut self, path: &Path, paused: bool) -> Result<(), String> {
        self.stop();
        let file = File::open(path).map_err(|e| format!("Ouverture impossible : {e}"))?;
        let source =
            Decoder::new(BufReader::new(file)).map_err(|e| format!("Format non lisible : {e}"))?;
        let sink = Sink::try_new(&self.handle).map_err(|e| format!("Erreur audio : {e}"))?;
        sink.set_volume(self.volume);
        if paused {
            sink.pause();
        }
        sink.append(source);
        self.sink = Some(sink);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
    }

    pub fn play(&self) {
        if let Some(sink) = &self.sink {
            sink.play();
        }
    }

    pub fn pause(&self) {
        if let Some(sink) = &self.sink {
            sink.pause();
        }
    }

    pub fn is_paused(&self) -> bool {
        self.sink.as_ref().map(|s| s.is_paused()).unwrap_or(true)
    }

    /// Vrai si un morceau est chargé (en lecture ou en pause).
    pub fn has_track(&self) -> bool {
        self.sink.is_some()
    }

    pub fn is_playing(&self) -> bool {
        self.has_track() && !self.is_paused()
    }

    /// Vrai si le morceau chargé est arrivé à la fin.
    pub fn finished(&self) -> bool {
        self.sink.as_ref().map(|s| s.empty()).unwrap_or(false)
    }

    pub fn position(&self) -> Duration {
        self.sink.as_ref().map(|s| s.get_pos()).unwrap_or_default()
    }

    pub fn seek(&self, pos: Duration) -> Result<(), String> {
        match &self.sink {
            Some(sink) => sink
                .try_seek(pos)
                .map_err(|e| format!("Déplacement impossible : {e}")),
            None => Ok(()),
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        if let Some(sink) = &self.sink {
            sink.set_volume(volume);
        }
    }
}
