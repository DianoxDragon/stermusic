//! A single SterMusic window: when launched a second time (double click on a track, Dolphin
//! menu, terminal), SterMusic hands its files to the instance already open, over D-Bus, instead
//! of opening a second window.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui;

const NAME: &str = "org.stermusic.App";
const PATH: &str = "/App";

/// What another instance asks for.
pub enum Request {
    /// Open the tag editor on these files or folders.
    EditTags(Vec<PathBuf>),
    /// Play these tracks (even outside the library).
    Play(Vec<PathBuf>),
    /// Show the window.
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
    /// Opens the tag editor on these files or folders.
    fn edit_tags(&self, paths_: Vec<String>) {
        self.send(Request::EditTags(paths(paths_)));
    }

    /// Plays these tracks.
    fn play(&self, paths_: Vec<String>) {
        self.send(Request::Play(paths(paths_)));
    }

    /// Shows the window.
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

/// Calls a method of the instance already open; false if there is none.
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

/// Hands files to edit to an instance already open; false if there is none.
pub fn send_tags(paths: &[PathBuf]) -> bool {
    call("EditTags", Some(paths))
}

/// Hands tracks to play to an instance already open; false if there is none.
pub fn send_play(paths: &[PathBuf]) -> bool {
    call("Play", Some(paths))
}

/// Shows the window of the instance already open; false if there is none.
pub fn send_show() -> bool {
    call("Show", None)
}
