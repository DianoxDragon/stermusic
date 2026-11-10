//! Notes (étoiles) : les réglages en gardent une copie pour l'affichage et les tris, et elles
//! sont inscrites dans les fichiers (voir `tags::write_rating`), par un thread à part.
//!
//! Au chargement de la bibliothèque, les deux se recollent : une note trouvée dans un fichier
//! (posée par un autre lecteur, ou par SterMusic sur un autre ordinateur) l'emporte ; une note
//! des réglages absente du fichier (donnée avant que SterMusic les y écrive) y est inscrite.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use super::SterMusic;
use crate::tags::WriteOpts;

struct Job {
    path: PathBuf,
    stars: u8,
    opts: WriteOpts,
    /// Note donnée à l'instant (une erreur mérite alors un message).
    asked: bool,
}

struct Done {
    path: PathBuf,
    stars: u8,
    asked: bool,
    result: Result<(), String>,
}

pub struct RatingWriter {
    tx: Sender<Job>,
    rx: Receiver<Done>,
    /// Écritures en cours, par fichier (la dernière note demandée).
    pending: HashMap<PathBuf, u8>,
    /// Fichiers où l'écriture a échoué pendant cette session : on ne réessaie pas à chaque
    /// rechargement de la bibliothèque.
    failed: HashSet<PathBuf>,
    /// Le cache de la bibliothèque est à réenregistrer quand tout sera écrit.
    dirty: bool,
}

impl RatingWriter {
    pub fn new(ctx: &eframe::egui::Context) -> Self {
        let (tx, jobs) = channel::<Job>();
        let (done, rx) = channel::<Done>();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for job in jobs {
                let result = crate::tags::write_rating(&job.path, job.stars, job.opts);
                let _ = done.send(Done {
                    path: job.path,
                    stars: job.stars,
                    asked: job.asked,
                    result,
                });
                ctx.request_repaint();
            }
        });
        Self {
            tx,
            rx,
            pending: HashMap::new(),
            failed: HashSet::new(),
            dirty: false,
        }
    }
}

impl SterMusic {
    /// Note `stars` (0 : retirée) donnée au morceau `i` de la bibliothèque.
    pub(super) fn set_rating(&mut self, i: usize, stars: u8) {
        let Some(t) = self.lib.tracks.get(i) else { return };
        let path = t.path.clone();
        let stars = stars.min(5);
        if stars == 0 {
            self.settings.ratings.remove(&path);
        } else {
            self.settings.ratings.insert(path.clone(), stars);
        }
        if t.is_plain_file() {
            self.ratings.failed.remove(&path);
            self.write_rating(path, stars, true);
        }
    }

    fn write_rating(&mut self, path: PathBuf, stars: u8, asked: bool) {
        // Le morceau en cours (ou celui déjà ouvert pour lui succéder) est lu par le décodeur :
        // il est réécrit dans une copie, sans toucher au fichier ouvert.
        let open = self.now_loaded.lock().ok().and_then(|p| p.clone()).as_ref() == Some(&path)
            || self
                .player
                .as_ref()
                .and_then(|p| p.queued())
                .and_then(|m| m.path())
                == Some(path.as_path());
        let opts = WriteOpts {
            id3v23: self.settings.tag_id3v23,
            // Une note ne change pas la musique : l'album ne remonte pas dans « Ajoutés récemment ».
            keep_mtime: true,
            via_copy: open,
        };
        self.ratings.pending.insert(path.clone(), stars);
        let _ = self.ratings.tx.send(Job { path, stars, opts, asked });
    }

    /// Recolle les notes des réglages et celles des fichiers (au chargement de la bibliothèque).
    pub(super) fn sync_ratings(&mut self) {
        let mut to_write = Vec::new();
        // Un morceau du cache lu par une version plus ancienne ne connaît pas encore la note de
        // son fichier : il attend d'être relu.
        let fresh = |t: &&crate::library::Track| t.is_plain_file() && t.schema >= crate::library::TRACK_SCHEMA;
        for t in self.lib.tracks.iter().filter(fresh) {
            if self.ratings.pending.contains_key(&t.path) {
                continue;
            }
            match (t.rating, self.settings.ratings.get(&t.path).copied()) {
                (Some(file), Some(mine)) if file != mine => {
                    self.settings.ratings.insert(t.path.clone(), file);
                }
                (Some(file), None) => {
                    self.settings.ratings.insert(t.path.clone(), file);
                }
                (None, Some(mine)) if !self.ratings.failed.contains(&t.path) => {
                    to_write.push((t.path.clone(), mine));
                }
                _ => {}
            }
        }
        for (path, stars) in to_write {
            self.write_rating(path, stars, false);
        }
        self.smart_cache.borrow_mut().clear();
    }

    /// Résultats des écritures de notes.
    pub(super) fn poll_ratings(&mut self) {
        let mut error = None;
        while let Ok(done) = self.ratings.rx.try_recv() {
            if self.ratings.pending.get(&done.path) == Some(&done.stars) {
                self.ratings.pending.remove(&done.path);
            }
            match done.result {
                Ok(()) => {
                    if let Some(i) = self.lib.find(&done.path) {
                        self.lib.tracks[i].rating = (done.stars > 0).then_some(done.stars);
                        self.ratings.dirty = true;
                    }
                }
                Err(e) => {
                    self.ratings.failed.insert(done.path.clone());
                    if done.asked {
                        error = Some(e);
                    }
                }
            }
        }
        if let Some(e) = error {
            self.toast(crate::trf!("Note gardée par SterMusic, mais pas inscrite dans le fichier : {e}", e));
        }
        if self.ratings.dirty && self.ratings.pending.is_empty() {
            self.ratings.dirty = false;
            crate::library::save_cache(&self.lib.tracks);
        }
    }
}
