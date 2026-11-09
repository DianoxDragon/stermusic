//! MPRIS : SterMusic apparaît dans le widget lecteur de KDE / GNOME, et les touches multimédia
//! du clavier le pilotent.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use eframe::egui;
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};

use crate::library::Track;

pub enum Remote {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    SeekBy(f32),
    SetPosition(Duration),
    Raise,
}

#[derive(PartialEq, Clone, Copy)]
pub enum State {
    Playing,
    Paused,
    Stopped,
}

pub struct Mpris {
    controls: Option<MediaControls>,
    rx: Receiver<Remote>,
    meta_for: Option<(PathBuf, Option<PathBuf>)>,
    state: Option<State>,
}

impl Mpris {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let config = PlatformConfig {
            dbus_name: "stermusic",
            display_name: "SterMusic",
            hwnd: None,
        };
        let controls = MediaControls::new(config).ok().and_then(|mut c| {
            let ctx = ctx.clone();
            c.attach(move |event| {
                let remote = match event {
                    MediaControlEvent::Play => Remote::Play,
                    MediaControlEvent::Pause => Remote::Pause,
                    MediaControlEvent::Toggle => Remote::Toggle,
                    MediaControlEvent::Next => Remote::Next,
                    MediaControlEvent::Previous => Remote::Previous,
                    MediaControlEvent::Stop => Remote::Stop,
                    MediaControlEvent::Seek(dir) => Remote::SeekBy(sign(dir) * 10.0),
                    MediaControlEvent::SeekBy(dir, d) => Remote::SeekBy(sign(dir) * d.as_secs_f32()),
                    MediaControlEvent::SetPosition(MediaPosition(d)) => Remote::SetPosition(d),
                    MediaControlEvent::Raise => Remote::Raise,
                    _ => return,
                };
                let _ = tx.send(remote);
                ctx.request_repaint();
            })
            .ok()?;
            Some(c)
        });
        Self {
            controls,
            rx,
            meta_for: None,
            state: None,
        }
    }

    pub fn available(&self) -> bool {
        self.controls.is_some()
    }

    pub fn poll(&self) -> Vec<Remote> {
        self.rx.try_iter().collect()
    }

    /// Met à jour titre / artiste / pochette (seulement si quelque chose a changé).
    pub fn set_track(&mut self, track: Option<&Track>, cover: Option<&Path>) {
        let Some(controls) = &mut self.controls else {
            return;
        };
        let key = track.map(|t| (t.path.clone(), cover.map(Path::to_path_buf)));
        if key == self.meta_for {
            return;
        }
        self.meta_for = key;
        let cover_url = cover.map(|c| format!("file://{}", c.display()));
        let meta = match track {
            Some(t) => MediaMetadata {
                title: Some(&t.title),
                artist: Some(t.artist_or_unknown()),
                album: Some(t.album_or_unknown()),
                cover_url: cover_url.as_deref(),
                duration: Some(t.duration()),
            },
            None => MediaMetadata::default(),
        };
        let _ = controls.set_metadata(meta);
    }

    /// État de lecture ; `force` après un déplacement dans le morceau.
    pub fn set_state(&mut self, state: State, position: Duration, force: bool) {
        let Some(controls) = &mut self.controls else {
            return;
        };
        if self.state == Some(state) && !force {
            return;
        }
        self.state = Some(state);
        let progress = Some(MediaPosition(position));
        let playback = match state {
            State::Playing => MediaPlayback::Playing { progress },
            State::Paused => MediaPlayback::Paused { progress },
            State::Stopped => MediaPlayback::Stopped,
        };
        let _ = controls.set_playback(playback);
    }
}

fn sign(dir: SeekDirection) -> f32 {
    match dir {
        SeekDirection::Forward => 1.0,
        SeekDirection::Backward => -1.0,
    }
}
