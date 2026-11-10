//! Bibliothèque mise à jour toute seule : les dossiers de musique sont surveillés (inotify), et
//! un fichier ajouté, modifié, renommé ou supprimé est pris en compte sans « Rescanner ».
//!
//! Les changements sont regroupés : on attend que le disque se calme (une copie d'album produit
//! des dizaines d'événements), puis seuls les fichiers touchés sont relus. Au-delà d'un certain
//! nombre, un rescan complet (qui reprend le cache) est plus simple.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

/// Silence à attendre avant de traiter les changements.
const QUIET: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct Watcher {
    _inner: Option<RecommendedWatcher>,
    rx: Option<Receiver<Vec<PathBuf>>>,
    pending: HashSet<PathBuf>,
    last: Option<Instant>,
    /// Ce qui n'a pas pu être surveillé.
    pub error: Option<String>,
}

/// Fichiers temporaires et cachés : rien à voir avec la bibliothèque (écriture des tags via une
/// copie « .nom.stermusic-tmp », fichiers partiels des navigateurs et de KIO…).
fn ignored(p: &Path) -> bool {
    p.file_name().map(|n| n.to_string_lossy()).is_some_and(|n| {
        n.starts_with('.') || n.ends_with(".part") || n.ends_with(".tmp") || n.ends_with('~')
    })
}

impl Watcher {
    /// Surveille ces dossiers (et remplace la surveillance précédente).
    pub fn start(dirs: &[PathBuf], ctx: &egui::Context) -> Self {
        let mut me = Self::default();
        if dirs.is_empty() {
            return me;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let handler = move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            // Lectures et ouvertures : rien n'a changé.
            if matches!(event.kind, EventKind::Access(_) | EventKind::Other) {
                return;
            }
            let paths: Vec<PathBuf> = event.paths.into_iter().filter(|p| !ignored(p)).collect();
            if !paths.is_empty() && tx.send(paths).is_ok() {
                ctx.request_repaint_after(QUIET + Duration::from_millis(100));
            }
        };
        let mut watcher = match notify::recommended_watcher(handler) {
            Ok(w) => w,
            Err(e) => {
                me.error = Some(e.to_string());
                return me;
            }
        };
        for d in dirs {
            if let Err(e) = watcher.watch(d, RecursiveMode::Recursive) {
                // Le plus souvent : trop de dossiers pour la limite d'inotify du système.
                me.error = Some(match e.kind {
                    notify::ErrorKind::MaxFilesWatch => crate::i18n::tr(
                        "trop de dossiers pour la limite du système (fs.inotify.max_user_watches)",
                    )
                    .to_owned(),
                    _ => e.to_string(),
                });
            }
        }
        me._inner = Some(watcher);
        me.rx = Some(rx);
        me
    }

    /// Les chemins changés, une fois le disque calmé depuis `QUIET`.
    pub fn poll(&mut self) -> Option<Vec<PathBuf>> {
        if let Some(rx) = &self.rx {
            let mut got = false;
            for paths in rx.try_iter() {
                self.pending.extend(paths);
                got = true;
            }
            if got {
                self.last = Some(Instant::now());
            }
        }
        let ready = self.last.is_some_and(|t| t.elapsed() >= QUIET) && !self.pending.is_empty();
        if !ready {
            return None;
        }
        self.last = None;
        Some(self.pending.drain().collect())
    }

    /// Des changements attendent que le disque se calme.
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// Ce qu'il faut faire des chemins changés.
pub enum Plan {
    /// Trop de changements : rescan complet.
    Rescan,
    /// Fichiers à (re)lire et chemins disparus (fichiers ou dossiers entiers).
    Update { read: Vec<PathBuf>, gone: Vec<PathBuf> },
}

/// Trie les chemins changés (à appeler hors du fil de l'interface : il parcourt les dossiers
/// nouveaux).
pub fn plan(paths: Vec<PathBuf>) -> Plan {
    const MAX: usize = 400;
    let mut read = Vec::new();
    let mut gone = Vec::new();
    for p in paths {
        if p.is_dir() {
            // Dossier copié ou déplacé dans la bibliothèque : ses morceaux.
            let files = crate::tags::expand_until([p], &std::sync::atomic::AtomicBool::new(false));
            read.extend(files);
        } else if p.exists() {
            // Une feuille CUE (ou un fichier qu'elle découpe) : tout le dossier est à relire.
            let near_cue = crate::library::is_cue_file(&p)
                || p.parent().and_then(|d| std::fs::read_dir(d).ok()).is_some_and(|mut d| {
                    d.any(|e| e.is_ok_and(|e| crate::library::is_cue_file(&e.path())))
                });
            if near_cue {
                return Plan::Rescan;
            }
            if crate::library::is_audio_file(&p) {
                read.push(p);
            }
        } else {
            gone.push(p);
        }
        if read.len() > MAX {
            return Plan::Rescan;
        }
    }
    read.sort();
    read.dedup();
    Plan::Update { read, gone }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters() {
        assert!(ignored(Path::new("/m/.a.mp3.stermusic-tmp")));
        assert!(ignored(Path::new("/m/a.mp3.part")));
        assert!(!ignored(Path::new("/m/a.mp3")));
    }

    #[test]
    fn plans() {
        let dir = std::env::temp_dir().join(format!("stermusic-watch-{}", std::process::id()));
        let album = dir.join("Album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::write(album.join("01.mp3"), b"x").unwrap();
        std::fs::write(album.join("cover.jpg"), b"x").unwrap();
        std::fs::write(dir.join("seul.flac"), b"x").unwrap();
        let Plan::Update { read, gone } = plan(vec![
            album.clone(),
            dir.join("seul.flac"),
            dir.join("cover.png"),
            dir.join("parti.mp3"),
        ]) else {
            panic!("pas de rescan attendu");
        };
        assert_eq!(read, vec![album.join("01.mp3"), dir.join("seul.flac")]);
        assert_eq!(gone, vec![dir.join("cover.png"), dir.join("parti.mp3")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Vraie surveillance (inotify) d'un dossier temporaire : un fichier copié est signalé une
    /// fois le disque calmé.
    #[test]
    fn watches() {
        let dir = std::env::temp_dir().join(format!("stermusic-inotify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = Watcher::start(std::slice::from_ref(&dir), &egui::Context::default());
        assert!(w.error.is_none(), "{:?}", w.error);
        std::fs::write(dir.join("nouveau.mp3"), b"x").unwrap();
        std::fs::write(dir.join(".cache.tmp"), b"x").unwrap();
        let mut got = None;
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(100));
            if let Some(p) = w.poll() {
                got = Some(p);
                break;
            }
        }
        let got = got.expect("aucun changement signalé");
        assert!(got.contains(&dir.join("nouveau.mp3")), "{got:?}");
        assert!(!got.iter().any(|p| p.ends_with(".cache.tmp")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
