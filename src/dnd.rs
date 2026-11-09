//! Glisser-déposer de fichiers sous Wayland.
//!
//! winit (0.30) ne transmet les fichiers déposés que sous X11 : sous Wayland, rien n'arrive
//! jusqu'à egui. On écoute donc nous-mêmes le protocole (`wl_data_device`), sur la connexion de
//! la fenêtre mais avec notre propre file d'événements, dans un thread, comme le fait le
//! presse-papiers d'egui. Les fichiers reçus sont glissés dans les entrées d'egui
//! ([`Dnd::feed`]) : le reste de l'application ne voit pas la différence avec X11.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use wayland_client::backend::Backend;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::{wl_callback, wl_registry, wl_seat};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle, WEnum};

const URI_LIST: &str = "text/uri-list";

#[derive(Default)]
struct Shared {
    /// Des fichiers survolent la fenêtre.
    hovering: bool,
    dropped: Vec<PathBuf>,
}

/// Arrêt propre du thread d'écoute (avant que la fenêtre ne ferme la connexion).
struct Worker {
    stop: Arc<AtomicBool>,
    conn: Connection,
    wake: QueueHandle<State>,
    thread: std::thread::JoinHandle<()>,
}

pub struct Dnd {
    shared: Arc<Mutex<Shared>>,
    worker: Option<Worker>,
}

impl Dnd {
    /// Sous X11 (ou si Wayland ne propose pas le glisser-déposer), ne fait rien : winit s'en charge.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let worker = start(cc, shared.clone());
        Self { shared, worker }
    }

    /// Ajoute aux entrées d'egui les fichiers survolés ou déposés.
    pub fn feed(&self, raw: &mut egui::RawInput) {
        if self.worker.is_none() {
            return;
        }
        let mut s = self.shared.lock().unwrap();
        if s.hovering && raw.hovered_files.is_empty() {
            raw.hovered_files.push(egui::HoveredFile::default());
        }
        for path in s.dropped.drain(..) {
            raw.dropped_files.push(egui::DroppedFile {
                path: Some(path),
                ..Default::default()
            });
        }
    }

    pub fn stop(&mut self) {
        let Some(w) = self.worker.take() else { return };
        w.stop.store(true, Ordering::Relaxed);
        // Un aller-retour avec le serveur réveille le thread, qui attend un événement.
        w.conn.display().sync(&w.wake, ());
        let _ = w.conn.flush();
        let _ = w.thread.join();
    }
}

fn start(cc: &eframe::CreationContext<'_>, shared: Arc<Mutex<Shared>>) -> Option<Worker> {
    let RawDisplayHandle::Wayland(handle) = cc.display_handle().ok()?.as_raw() else {
        return None;
    };
    // SAFETY: le pointeur est le `wl_display` de la fenêtre, vivant jusqu'à la fin d'eframe ;
    // le thread qui s'en sert est arrêté avant (voir `Dnd::stop`, appelé à la fermeture).
    let backend = unsafe { Backend::from_foreign_display(handle.display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    let (globals, mut queue) = registry_queue_init::<State>(&conn).ok()?;
    let qh = queue.handle();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).ok()?;
    let manager: WlDataDeviceManager = globals.bind(&qh, 1..=3, ()).ok()?;
    let device = manager.get_data_device(&seat, &qh, ());

    let stop = Arc::new(AtomicBool::new(false));
    let mut state = State {
        shared,
        ctx: cc.egui_ctx.clone(),
        conn: conn.clone(),
        offers: Vec::new(),
        drag: None,
        _device: device,
    };
    let stopped = stop.clone();
    let thread = std::thread::Builder::new()
        .name("wayland-dnd".into())
        .spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                if queue.blocking_dispatch(&mut state).is_err() {
                    break;
                }
            }
        })
        .ok()?;
    Some(Worker {
        stop,
        conn,
        wake: qh,
        thread,
    })
}

/// Une offre de données annoncée par le serveur (un glisser en cours, ou le presse-papiers).
struct Offer {
    offer: WlDataOffer,
    /// Elle propose une liste de fichiers.
    files: bool,
    /// Le serveur a confirmé une action (copie) : `finish` est permis.
    action: bool,
}

struct State {
    shared: Arc<Mutex<Shared>>,
    ctx: egui::Context,
    conn: Connection,
    offers: Vec<Offer>,
    /// L'offre du glisser qui survole la fenêtre.
    drag: Option<WlDataOffer>,
    _device: WlDataDevice,
}

impl State {
    fn take(&mut self, offer: &WlDataOffer) -> Option<Offer> {
        let i = self.offers.iter().position(|o| &o.offer == offer)?;
        Some(self.offers.swap_remove(i))
    }

    fn set_hovering(&self, on: bool) {
        self.shared.lock().unwrap().hovering = on;
        self.ctx.request_repaint();
    }

    /// Lit la liste des fichiers déposés (le programme d'origine l'écrit dans un tube).
    fn receive(&self, offer: &WlDataOffer) -> Vec<PathBuf> {
        let Ok((mut reader, writer)) = std::io::pipe() else {
            return Vec::new();
        };
        offer.receive(URI_LIST.to_owned(), std::os::fd::AsFd::as_fd(&writer));
        let _ = self.conn.flush();
        // Notre copie du côté écriture doit être fermée, sinon la lecture ne finirait jamais.
        drop(writer);
        let mut text = String::new();
        let _ = reader.read_to_string(&mut text);
        parse_uri_list(&text)
    }
}

/// « file:///home/moi/Ma%20musique/a.mp3 » → chemin ; les autres adresses sont ignorées.
fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        // « file://hôte/chemin » : seul le chemin compte (l'hôte est presque toujours vide).
        .filter_map(|rest| rest.find('/').map(|i| &rest[i..]))
        .map(|path| {
            let bytes = path.as_bytes();
            let mut out = Vec::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                let hex = (bytes[i] == b'%')
                    .then(|| bytes.get(i + 1..i + 3))
                    .flatten()
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(
                out,
            ))
        })
        .collect()
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wl_data_device::Event;
        match event {
            Event::DataOffer { id } => state.offers.push(Offer {
                offer: id,
                files: false,
                action: false,
            }),
            Event::Enter { serial, id, .. } => {
                let Some(offer) = id else { return };
                let files = state.offers.iter().any(|o| o.offer == offer && o.files);
                if files {
                    offer.accept(serial, Some(URI_LIST.to_owned()));
                    if offer.version() >= 3 {
                        offer.set_actions(DndAction::Copy, DndAction::Copy);
                    }
                } else {
                    offer.accept(serial, None);
                }
                state.drag = Some(offer);
                state.set_hovering(files);
            }
            Event::Leave => {
                if let Some(o) = state.drag.take().and_then(|d| state.take(&d)) {
                    o.offer.destroy();
                }
                state.set_hovering(false);
            }
            Event::Drop => {
                let Some(o) = state.drag.take().and_then(|d| state.take(&d)) else {
                    return;
                };
                if o.files {
                    let paths = state.receive(&o.offer);
                    // `finish` sans action confirmée serait une erreur de protocole.
                    if o.offer.version() >= 3 && o.action {
                        o.offer.finish();
                    }
                    state.shared.lock().unwrap().dropped.extend(paths);
                }
                o.offer.destroy();
                let _ = state.conn.flush();
                state.set_hovering(false);
            }
            // Le presse-papiers ne nous concerne pas : son offre est libérée aussitôt.
            Event::Selection { id: Some(offer) } => {
                state.take(&offer);
                offer.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(o) = state.offers.iter_mut().find(|o| &o.offer == offer) else {
            return;
        };
        match event {
            wl_data_offer::Event::Offer { mime_type } if mime_type == URI_LIST => o.files = true,
            wl_data_offer::Event::Action { dnd_action } => {
                o.action = matches!(dnd_action, WEnum::Value(a) if !a.is_empty());
            }
            _ => {}
        }
    }
}

/// Objets dont les événements ne nous servent pas.
macro_rules! ignore_events {
    ($($proxy:ty => $data:ty),* $(,)?) => {$(
        impl Dispatch<$proxy, $data> for State {
            fn event(
                _: &mut Self,
                _: &$proxy,
                _: <$proxy as Proxy>::Event,
                _: &$data,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events! {
    wl_registry::WlRegistry => GlobalListContents,
    wl_seat::WlSeat => (),
    WlDataDeviceManager => (),
    wl_callback::WlCallback => (),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_lists() {
        let text = "# commentaire\r\nfile:///home/moi/Ma%20musique/%C3%A9t%C3%A9.mp3\r\n\
                    file://localhost/tmp/a%25b.flac\r\nhttps://exemple.fr/x.mp3\r\n";
        assert_eq!(
            parse_uri_list(text),
            vec![
                PathBuf::from("/home/moi/Ma musique/été.mp3"),
                PathBuf::from("/tmp/a%b.flac"),
            ]
        );
    }
}
