//! SterMusic addition: files dragged onto the window (Wayland).
//!
//! The clipboard thread accepts drags offering a list of files and stores here what is
//! dropped; the application fetches it every frame.

use std::sync::Mutex;

pub(crate) const URI_LIST: &str = "text/uri-list";

struct Shared {
    hovering: bool,
    /// "text/uri-list" lists received, as is.
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

/// Called when files hover the window, leave it or are dropped on it (to redraw).
pub fn set_waker(waker: impl Fn() + Send + 'static) {
    SHARED.lock().unwrap().waker = Some(Box::new(waker));
}

/// Files are hovering the window.
pub fn hovering() -> bool {
    SHARED.lock().unwrap().hovering
}

/// File lists dropped since the last call ("text/uri-list" format).
pub fn take_dropped() -> Vec<String> {
    std::mem::take(&mut SHARED.lock().unwrap().dropped)
}
