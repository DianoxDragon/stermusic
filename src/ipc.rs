//! Une seule fenêtre SterMusic : lancé une deuxième fois (double-clic sur un morceau, menu de
//! Dolphin, terminal), SterMusic confie ses fichiers à l'instance déjà ouverte, par D-Bus, au lieu
//! d'ouvrir une deuxième fenêtre.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui;

const NAME: &str = "org.stermusic.App";
const PATH: &str = "/App";

/// Ce qu'une autre instance demande.
pub enum Request {
    /// Ouvrir l'éditeur de tags sur ces fichiers ou dossiers.
    EditTags(Vec<PathBuf>),
    /// Jouer ces morceaux (même hors bibliothèque).
    Play(Vec<PathBuf>),
    /// Montrer la fenêtre.
    Show,
}

struct Iface {
    tx: Sender<Request>,
    ctx: egui::Context,
}

impl Iface {
    fn send(&self, request: Request) {
        let _ = self.tx.send(request);
        self.ctx.request_repaint();
    }
}

fn paths(list: Vec<String>) -> Vec<PathBuf> {
    list.into_iter().map(PathBuf::from).collect()
}

#[zbus::interface(name = "org.stermusic.App")]
impl Iface {
    /// Ouvre l'éditeur de tags sur ces fichiers ou dossiers.
    fn edit_tags(&self, paths_: Vec<String>) {
        self.send(Request::EditTags(paths(paths_)));
    }

    /// Joue ces morceaux.
    fn play(&self, paths_: Vec<String>) {
        self.send(Request::Play(paths(paths_)));
    }

    /// Montre la fenêtre.
    fn show(&self) {
        self.send(Request::Show);
    }
}

pub struct Ipc {
    _conn: Option<zbus::blocking::Connection>,
    rx: Receiver<Request>,
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

    pub fn poll(&self) -> Vec<Request> {
        self.rx.try_iter().collect()
    }
}

/// Appelle une méthode de l'instance déjà ouverte ; faux s'il n'y en a pas.
fn call(method: &str, paths: Option<&[PathBuf]>) -> bool {
    let Ok(conn) = zbus::blocking::Connection::session() else {
        return false;
    };
    let result = match paths {
        Some(paths) => {
            let list: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            conn.call_method(Some(NAME), PATH, Some(NAME), method, &(list,))
        }
        None => conn.call_method(Some(NAME), PATH, Some(NAME), method, &()),
    };
    result.is_ok()
}

/// Confie des fichiers à éditer à une instance déjà ouverte ; faux s'il n'y en a pas.
pub fn send_tags(paths: &[PathBuf]) -> bool {
    call("EditTags", Some(paths))
}

/// Confie des morceaux à jouer à une instance déjà ouverte ; faux s'il n'y en a pas.
pub fn send_play(paths: &[PathBuf]) -> bool {
    call("Play", Some(paths))
}

/// Montre la fenêtre de l'instance déjà ouverte ; faux s'il n'y en a pas.
pub fn send_show() -> bool {
    call("Show", None)
}
