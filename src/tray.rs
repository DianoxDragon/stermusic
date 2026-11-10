//! Icône dans la zone de notification (StatusNotifierItem : KDE Plasma, et GNOME avec
//! l'extension AppIndicator).
//!
//! Clic : montrer ou cacher la fenêtre ; clic du milieu : lecture / pause ; molette : morceau
//! suivant ou précédent ; menu : lecture, morceaux, fenêtre, quitter. Le service tourne dans
//! son propre thread ; les commandes arrivent à l'interface par un canal.

use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui;
use ksni::blocking::TrayMethods;

use crate::i18n::tr;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    /// Montrer la fenêtre si elle est cachée, la cacher sinon.
    Toggle,
    Show,
    PlayPause,
    Next,
    Previous,
    Quit,
}

/// Ce que l'icône affiche.
#[derive(Clone, Default, PartialEq)]
pub struct State {
    pub playing: bool,
    /// « Titre — Artiste », vide sans morceau.
    pub track: String,
    /// La fenêtre est cachée dans la zone de notification.
    pub hidden: bool,
}

struct Item {
    tx: Sender<Command>,
    ctx: egui::Context,
    state: State,
    icon: Vec<ksni::Icon>,
}

impl Item {
    fn send(&self, c: Command) {
        let _ = self.tx.send(c);
        self.ctx.request_repaint();
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "stermusic".into()
    }

    fn title(&self) -> String {
        "SterMusic".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_name(&self) -> String {
        "stermusic".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let description = if self.state.track.is_empty() {
            String::new()
        } else if self.state.playing {
            self.state.track.clone()
        } else {
            crate::trf!("{0} (en pause)", self.state.track)
        };
        ksni::ToolTip {
            title: "SterMusic".into(),
            description,
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Command::Toggle);
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(Command::PlayPause);
    }

    fn scroll(&mut self, delta: i32, orientation: ksni::Orientation) {
        if orientation == ksni::Orientation::Vertical && delta != 0 {
            self.send(if delta < 0 { Command::Next } else { Command::Previous });
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{MenuItem, StandardItem};
        let item = |label: &str, icon: &str, c: Command| -> MenuItem<Self> {
            StandardItem {
                label: label.replace('_', "__"),
                icon_name: icon.into(),
                activate: Box::new(move |t: &mut Self| t.send(c)),
                ..Default::default()
            }
            .into()
        };
        let mut items = Vec::new();
        if !self.state.track.is_empty() {
            items.push(
                StandardItem {
                    label: self.state.track.replace('_', "__"),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
            items.push(MenuItem::Separator);
        }
        let (label, icon) = if self.state.playing {
            (tr("Pause"), "media-playback-pause")
        } else {
            (tr("Lecture"), "media-playback-start")
        };
        items.push(item(label, icon, Command::PlayPause));
        items.push(item(tr("Morceau suivant"), "media-skip-forward", Command::Next));
        items.push(item(tr("Morceau précédent"), "media-skip-backward", Command::Previous));
        items.push(MenuItem::Separator);
        let (label, icon) = if self.state.hidden {
            (tr("Afficher SterMusic"), "window-new")
        } else {
            (tr("Cacher SterMusic"), "window-minimize")
        };
        items.push(item(label, icon, if self.state.hidden { Command::Show } else { Command::Toggle }));
        items.push(item(tr("Quitter"), "application-exit", Command::Quit));
        items
    }
}

/// L'icône de l'application, en ARGB (ce qu'attend le protocole), en deux tailles.
fn app_icon() -> Vec<ksni::Icon> {
    let Ok(img) = image::load_from_memory(include_bytes!("../assets/icon.png")) else {
        return Vec::new();
    };
    [32u32, 64]
        .into_iter()
        .map(|size| {
            let rgba = img
                .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
                .to_rgba8();
            let data = rgba
                .pixels()
                .flat_map(|p| [p[3], p[0], p[1], p[2]])
                .collect();
            ksni::Icon {
                width: size as i32,
                height: size as i32,
                data,
            }
        })
        .collect()
}

pub struct Tray {
    handle: ksni::blocking::Handle<Item>,
    rx: Receiver<Command>,
    state: State,
}

impl Tray {
    /// Place l'icône ; `None` si le bureau n'a pas de zone de notification compatible.
    pub fn start(ctx: &egui::Context, state: State) -> Option<Self> {
        let (tx, rx) = channel();
        let item = Item {
            tx,
            ctx: ctx.clone(),
            state: state.clone(),
            icon: app_icon(),
        };
        let handle = item.spawn().ok()?;
        Some(Self { handle, rx, state })
    }

    /// Met à jour ce que montre l'icône (seulement si quelque chose a changé).
    pub fn set_state(&mut self, state: State) {
        if state != self.state {
            self.state = state.clone();
            self.handle.update(move |item| item.state = state);
        }
    }

    pub fn poll(&self) -> Vec<Command> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let _ = self.handle.shutdown();
    }
}
