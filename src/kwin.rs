//! KWin (KDE Plasma) integration.
//!
//! Under Wayland, an application doesn't know the position of its window on screen. KWin does:
//! at startup, SterMusic loads a small KWin script that sends it (over D-Bus) the position of
//! its window and the screen it is on, on every move. The Mica background can thus show the
//! part of the wallpaper that is really behind the window.
//!
//! The same script keeps the mini player on top (Wayland doesn't let applications ask for it
//! themselves), and hides the window in the system tray (Wayland can't hide a window either):
//! minimized, out of the task bar and the window switcher, then restored and activated.
//! Requests go through a marker at the end of the window title. The script is unloaded when
//! SterMusic closes.

use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui::{self, pos2, Rect};

/// True while KDE is ending the session (logout, reboot, shutdown): the `org.kde.Shutdown`
/// service only exists during that time. SterMusic must then really quit when asked to,
/// instead of hiding in the system tray (otherwise KDE waits for the window, and the reboot
/// stays blocked).
pub fn session_ending() -> bool {
    let Ok(conn) = zbus::blocking::Connection::session() else {
        return false;
    };
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &("org.kde.Shutdown",),
    )
    .ok()
    .and_then(|reply| reply.body().deserialize::<bool>().ok())
    .unwrap_or(false)
}

/// Position of the window and of its screen, in desktop coordinates (logical pixels).
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub window: Rect,
    pub output_rect: Rect,
    pub output: String,
}

/// Window title suffix asking the script to keep the window on top.
pub const KEEP_ABOVE_MARK: &str = " · Mini lecteur";
/// Title suffix asking the script to hide the window (system tray).
pub const HIDDEN_MARK: &str = " · Zone de notification";

const SERVICE: &str = "org.stermusic.SterMusic";
const SCRIPT_NAME: &str = "stermusic";

const SCRIPT: &str = r#"// SterMusic's KWin script (loaded and unloaded automatically by SterMusic).
const miniState = {};

function send(w) {
    if (!w.output) {
        return;
    }
    const g = w.frameGeometry;
    const o = w.output.geometry;
    callDBus("org.stermusic.SterMusic", "/Geometry", "org.stermusic.Geometry", "Update",
        [g.x, g.y, g.width, g.height, o.x, o.y, o.width, o.height, w.output.name].join(";"));
}

function syncAbove(w) {
    const mini = w.caption.endsWith("KEEP_ABOVE_MARK");
    const id = String(w.internalId);
    if (miniState[id] !== mini) {
        miniState[id] = mini;
        w.keepAbove = mini;
    }
}

const hiddenState = {};

function syncHidden(w) {
    const hidden = w.caption.endsWith("HIDDEN_MARK");
    const id = String(w.internalId);
    const was = hiddenState[id];
    if (was === hidden) {
        return;
    }
    hiddenState[id] = hidden;
    if (hidden) {
        w.skipTaskbar = true;
        w.skipSwitcher = true;
        w.skipPager = true;
        w.minimized = true;
    } else if (was) {
        w.skipTaskbar = false;
        w.skipSwitcher = false;
        w.skipPager = false;
        w.minimized = false;
        workspace.activeWindow = w;
    }
}

function sendMinimized(w) {
    callDBus("org.stermusic.SterMusic", "/Geometry", "org.stermusic.Geometry", "Minimized", w.minimized);
}

function hook(w) {
    if (!w || w.resourceClass !== "stermusic") {
        return;
    }
    w.minimizedChanged.connect(function () { sendMinimized(w); });
    w.frameGeometryChanged.connect(function () { send(w); });
    w.outputChanged.connect(function () { send(w); });
    w.captionChanged.connect(function () { syncAbove(w); syncHidden(w); });
    send(w);
    syncAbove(w);
    syncHidden(w);
}

workspace.windowList().forEach(hook);
workspace.windowAdded.connect(hook);
"#;

enum Msg {
    Geometry(Geometry),
    Minimized(bool),
}

struct Iface {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

#[zbus::interface(name = "org.stermusic.Geometry")]
impl Iface {
    /// Called by the script: "x;y;width;height;screen x;screen y;screen width;screen height;name".
    fn update(&self, data: String) {
        if let Some(g) = parse(&data) {
            let _ = self.tx.send(Msg::Geometry(g));
            self.ctx.request_repaint();
        }
    }

    /// Called by the script when the window is minimized or restored (Wayland doesn't tell
    /// applications): while minimized, it animates nothing.
    fn minimized(&self, on: bool) {
        let _ = self.tx.send(Msg::Minimized(on));
        self.ctx.request_repaint();
    }
}

fn parse(data: &str) -> Option<Geometry> {
    let parts: Vec<&str> = data.split(';').collect();
    if parts.len() < 9 {
        return None;
    }
    let n: Vec<f32> = parts[..8]
        .iter()
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    Some(Geometry {
        window: Rect::from_min_size(pos2(n[0], n[1]), egui::vec2(n[2], n[3])),
        output_rect: Rect::from_min_size(pos2(n[4], n[5]), egui::vec2(n[6], n[7])),
        output: parts[8].to_owned(),
    })
}

pub struct KWin {
    conn: Option<zbus::blocking::Connection>,
    rx: Receiver<Msg>,
    /// Last known position of the window.
    pub geometry: Option<Geometry>,
    /// The window is minimized (or hidden in the system tray).
    pub minimized: bool,
}

impl KWin {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let conn = if crate::theme::is_kde_desktop() {
            start(tx, ctx.clone()).ok()
        } else {
            None
        };
        Self {
            conn,
            rx,
            geometry: None,
            minimized: false,
        }
    }

    /// True if the script is running (the window position is tracked).
    pub fn active(&self) -> bool {
        self.conn.is_some()
    }

    pub fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Geometry(g) => self.geometry = Some(g),
                Msg::Minimized(on) => self.minimized = on,
            }
        }
    }

    /// Unloads the KWin script.
    pub fn stop(&mut self) {
        if let Some(conn) = self.conn.take() {
            let _ = call(&conn, "unloadScript", &(SCRIPT_NAME,));
        }
    }
}

impl Drop for KWin {
    fn drop(&mut self) {
        self.stop();
    }
}

fn call<B>(conn: &zbus::blocking::Connection, method: &str, body: &B) -> zbus::Result<zbus::Message>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    conn.call_method(
        Some("org.kde.KWin"),
        "/Scripting",
        Some("org.kde.kwin.Scripting"),
        method,
        body,
    )
}

fn start(tx: Sender<Msg>, ctx: egui::Context) -> zbus::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::connection::Builder::session()?
        .name(SERVICE)?
        .serve_at("/Geometry", Iface { tx, ctx })?
        .build()?;
    let dir = crate::library::cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| zbus::Error::Failure(e.to_string()))?;
    let path = dir.join("kwin-script.js");
    let script = SCRIPT
        .replace("KEEP_ABOVE_MARK", KEEP_ABOVE_MARK)
        .replace("HIDDEN_MARK", HIDDEN_MARK);
    std::fs::write(&path, script)
        .map_err(|e| zbus::Error::Failure(e.to_string()))?;
    // A script left loaded (SterMusic closed abruptly) is replaced.
    let _ = call(&conn, "unloadScript", &(SCRIPT_NAME,));
    let path = path.to_string_lossy().into_owned();
    let id: i32 = call(&conn, "loadScript", &(path.as_str(), SCRIPT_NAME))?
        .body()
        .deserialize()?;
    if id < 0 {
        return Err(zbus::Error::Failure("KWin a refusé le script".into()));
    }
    call(&conn, "start", &())?;
    Ok(conn)
}

#[cfg(test)]
mod session_tests {
    /// On a private bus: `dbus-run-session -- cargo test session_ending -- --ignored`.
    #[test]
    #[ignore]
    fn session_ending() {
        assert!(!super::session_ending());
        let _shutdown = zbus::blocking::connection::Builder::session()
            .and_then(|b| b.name("org.kde.Shutdown"))
            .and_then(|b| b.build())
            .expect("nom pris sur le bus");
        assert!(super::session_ending());
    }
}
