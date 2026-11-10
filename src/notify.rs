//! Notifications du bureau (Plasma, GNOME…) au changement de morceau, par D-Bus
//! (`org.freedesktop.Notifications`).
//!
//! Un thread garde la connexion et remplace la notification précédente au lieu d'en empiler
//! une par morceau ; elles sont marquées « transitoires » pour ne pas encombrer l'historique.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};

use zbus::zvariant::Value;

pub struct Note {
    pub title: String,
    pub body: String,
    /// Pochette (fichier image), sinon l'icône de SterMusic.
    pub image: Option<PathBuf>,
}

#[derive(Default)]
pub struct Notifier {
    tx: Option<Sender<Note>>,
}

impl Notifier {
    pub fn send(&mut self, note: Note) {
        if self.tx.is_none() {
            let (tx, rx) = channel::<Note>();
            std::thread::spawn(move || {
                let Ok(conn) = zbus::blocking::Connection::session() else {
                    return;
                };
                let mut last_id = 0u32;
                while let Ok(mut note) = rx.recv() {
                    // Plusieurs changements d'affilée (morceaux sautés) : seul le dernier compte.
                    while let Ok(newer) = rx.try_recv() {
                        note = newer;
                    }
                    if let Some(id) = show(&conn, &note, last_id) {
                        last_id = id;
                    }
                }
            });
            self.tx = Some(tx);
        }
        if let Some(tx) = &self.tx {
            if tx.send(note).is_err() {
                self.tx = None;
            }
        }
    }
}

fn show(conn: &zbus::blocking::Connection, note: &Note, replaces: u32) -> Option<u32> {
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert("desktop-entry", Value::from("stermusic"));
    hints.insert("transient", Value::from(true));
    hints.insert("category", Value::from("x-stermusic.track"));
    let image = note
        .image
        .as_ref()
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned());
    if let Some(img) = &image {
        hints.insert("image-path", Value::from(img.as_str()));
    }
    let actions: Vec<&str> = Vec::new();
    let reply = conn
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "SterMusic",
                replaces,
                "stermusic",
                note.title.as_str(),
                escape(&note.body),
                actions,
                hints,
                5000i32,
            ),
        )
        .ok()?;
    reply.body().deserialize::<u32>().ok()
}

/// Le corps d'une notification accepte un peu de HTML : on neutralise `&`, `<` et `>`.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
