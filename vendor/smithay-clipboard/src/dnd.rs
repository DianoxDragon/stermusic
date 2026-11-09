//! Ajout de SterMusic : fichiers glissés sur la fenêtre (Wayland).
//!
//! Le fil du presse-papiers accepte les glisser qui proposent une liste de fichiers et range ici
//! ce qui est déposé ; l'application vient le chercher à chaque image.

use std::sync::Mutex;

pub(crate) const URI_LIST: &str = "text/uri-list";

struct Shared {
    hovering: bool,
    /// Listes « text/uri-list » reçues, telles quelles.
    dropped: Vec<String>,
    waker: Option<Box<dyn Fn() + Send>>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared { hovering: false, dropped: Vec::new(), waker: None });

fn wake(s: &Shared) {
    if let Some(w) = &s.waker {
        w();
    }
}

pub(crate) fn set_hovering(on: bool) {
    let mut s = SHARED.lock().unwrap();
    if s.hovering != on {
        s.hovering = on;
        wake(&s);
    }
}

pub(crate) fn push_dropped(uri_list: String) {
    let mut s = SHARED.lock().unwrap();
    s.dropped.push(uri_list);
    wake(&s);
}

/// Appelée quand des fichiers survolent la fenêtre, la quittent ou y sont déposés (pour redessiner).
pub fn set_waker(waker: impl Fn() + Send + 'static) {
    SHARED.lock().unwrap().waker = Some(Box::new(waker));
}

/// Des fichiers survolent la fenêtre.
pub fn hovering() -> bool {
    SHARED.lock().unwrap().hovering
}

/// Listes de fichiers déposées depuis le dernier appel (format « text/uri-list »).
pub fn take_dropped() -> Vec<String> {
    std::mem::take(&mut SHARED.lock().unwrap().dropped)
}
