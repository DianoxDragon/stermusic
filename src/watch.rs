//! Library updated by itself: the music folders are watched (inotify), and a file that is
//! added, modified, renamed or deleted is taken into account without "Rescan".
//!
//! Changes are batched: we wait for the disk to settle (copying an album produces dozens of
//! events), then only the touched files are read again. Beyond a certain number, a full rescan
//! (which reuses the cache) is simpler.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

/// Quiet time to wait before handling the changes.
const QUIET: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct Watcher {
    _inner: Option<RecommendedWatcher>,
    rx: Option<Receiver<Vec<PathBuf>>>,
    pending: HashSet<PathBuf>,
    last: Option<Instant>,
    /// What could not be watched.
    pub error: Option<String>,
}

/// Temporary and hidden files: nothing to do with the library (tags written through a
/// ".name.stermusic-tmp" copy, partial files of browsers and KIO…).
fn ignored(p: &Path) -> bool {
    p.file_name().map(|n| n.to_string_lossy()).is_some_and(|n| {
        n.starts_with('.') || n.ends_with(".part") || n.ends_with(".tmp") || n.ends_with('~')
    })
}

impl Watcher {
    /// Watches these folders (and replaces the previous watch).
    pub fn start(dirs: &[PathBuf], ctx: &egui::Context) -> Self {
        let mut me = Self::default();
        if dirs.is_empty() {
            return me;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let handler = move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            // Reads and opens: nothing changed.
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
                // Most often: too many folders for the system's inotify limit.
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

    /// The changed paths, once the disk has been quiet for `QUIET`.
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

    /// Changes are waiting for the disk to settle.
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// What to do with the changed paths.
pub enum Plan {
    /// Too many changes: full rescan.
    Rescan,
    /// Files to (re)read and paths that disappeared (files or whole folders).
    Update { read: Vec<PathBuf>, gone: Vec<PathBuf> },
}

/// Sorts the changed paths (to be called outside the interface thread: it walks the new
/// folders).
pub fn plan(paths: Vec<PathBuf>) -> Plan {
    const MAX: usize = 400;
    let mut read = Vec::new();
    let mut gone = Vec::new();
    for p in paths {
        if p.is_dir() {
            // Folder copied or moved into the library: its tracks.
            let files = crate::tags::expand_until([p], &std::sync::atomic::AtomicBool::new(false));
            read.extend(files);
        } else if p.exists() {
            // A CUE sheet (or a file it splits): the whole folder must be read again.
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

    /// Real watch (inotify) of a temporary folder: a copied file is reported once the disk has
    /// settled.
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
