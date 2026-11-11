//! Intégration à KWin (KDE Plasma).
//!
//! Sous Wayland, une application ne connaît pas la position de sa fenêtre à l'écran. KWin, lui,
//! la connaît : au lancement, SterMusic charge un petit script KWin qui lui envoie (par D-Bus) la
//! position de sa fenêtre et l'écran où elle se trouve, à chaque déplacement. Le fond Mica peut
//! ainsi montrer la portion du fond d'écran qui est vraiment derrière la fenêtre.
//!
//! Le même script garde le mini lecteur au premier plan (Wayland ne laisse pas les applications
//! le demander elles-mêmes), et cache la fenêtre dans la zone de notification (Wayland ne sait
//! pas non plus cacher une fenêtre) : réduite, hors de la barre des tâches et du sélecteur de
//! fenêtres, puis rendue et activée. Les demandes passent par une marque à la fin du titre de
//! la fenêtre. Le script est déchargé à la fermeture de SterMusic.

use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui::{self, pos2, Rect};

/// Vrai pendant que KDE ferme la session (déconnexion, redémarrage, extinction) : le service
/// `org.kde.Shutdown` n'existe que le temps de cette fermeture. SterMusic doit alors vraiment se
/// fermer quand on le lui demande, au lieu de se cacher dans la zone de notification (sinon KDE
/// attend la fenêtre, et le redémarrage reste bloqué).
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

/// Position de la fenêtre et de son écran, en coordonnées du bureau (pixels logiques).
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub window: Rect,
    pub output_rect: Rect,
    pub output: String,
}

/// Suffixe du titre de la fenêtre qui demande au script de la garder au premier plan.
pub const KEEP_ABOVE_MARK: &str = " · Mini lecteur";
/// Suffixe du titre qui demande au script de cacher la fenêtre (zone de notification).
pub const HIDDEN_MARK: &str = " · Zone de notification";

const SERVICE: &str = "org.stermusic.SterMusic";
const SCRIPT_NAME: &str = "stermusic";

const SCRIPT: &str = r#"// Script KWin de SterMusic (chargé et déchargé automatiquement par SterMusic).
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
    /// Appelé par le script : « x;y;largeur;hauteur;écran x;écran y;écran largeur;écran hauteur;nom ».
    fn update(&self, data: String) {
        if let Some(g) = parse(&data) {
            let _ = self.tx.send(Msg::Geometry(g));
            self.ctx.request_repaint();
        }
    }

    /// Appelé par le script quand la fenêtre est réduite ou rendue (Wayland ne le dit pas aux
    /// applications) : réduite, elle n'anime plus rien.
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
    /// Dernière position connue de la fenêtre.
    pub geometry: Option<Geometry>,
    /// La fenêtre est réduite (ou cachée dans la zone de notification).
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

    /// Vrai si le script tourne (la position de la fenêtre est suivie).
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

    /// Décharge le script de KWin.
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
    // Un script resté chargé (SterMusic fermé brutalement) est remplacé.
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
    /// Sur un bus privé : `dbus-run-session -- cargo test session_ending -- --ignored`.
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
