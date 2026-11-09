//! Une seule fenêtre SterMusic : `stermusic --tags fichiers…` (menu de Dolphin, terminal) confie
//! les fichiers à l'instance déjà ouverte, par D-Bus, au lieu d'en lancer une deuxième.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui;

const NAME: &str = "org.stermusic.App";
const PATH: &str = "/App";

struct Iface {
    tx: Sender<Vec<PathBuf>>,
    ctx: egui::Context,
}

#[zbus::interface(name = "org.stermusic.App")]
impl Iface {
    /// Ouvre l'éditeur de tags sur ces fichiers ou dossiers.
    fn edit_tags(&self, paths: Vec<String>) {
        let _ = self.tx.send(paths.into_iter().map(PathBuf::from).collect());
        self.ctx.request_repaint();
    }
}

pub struct Ipc {
    _conn: Option<zbus::blocking::Connection>,
    rx: Receiver<Vec<PathBuf>>,
}

impl Ipc {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let conn = zbus::blocking::connection::Builder::session()
            .and_then(|b| b.name(NAME))
            .and_then(|b| b.serve_at(PATH, Iface { tx, ctx: ctx.clone() }))
            .and_then(|b| b.build())
            .ok();
        Self { _conn: conn, rx }
    }

    pub fn poll(&self) -> Option<Vec<PathBuf>> {
        let mut all: Vec<PathBuf> = Vec::new();
        while let Ok(v) = self.rx.try_recv() {
            all.extend(v);
        }
        (!all.is_empty()).then_some(all)
    }
}

/// Confie les fichiers à une instance déjà ouverte ; faux s'il n'y en a pas.
pub fn send_to_running(paths: &[PathBuf]) -> bool {
    let Ok(conn) = zbus::blocking::Connection::session() else {
        return false;
    };
    let paths: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    conn.call_method(Some(NAME), PATH, Some(NAME), "EditTags", &(paths,))
        .is_ok()
}
