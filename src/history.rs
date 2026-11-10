//! Historique d'écoute : chaque morceau écouté au moins 30 secondes (ou la moitié s'il est plus
//! court) est noté avec sa date et le temps vraiment écouté, dans
//! `~/.local/share/stermusic/history.tsv` (une ligne par écoute : « horodatage<TAB>chemin<TAB>
//! secondes » ; les premières versions n'avaient pas la dernière colonne). Sert aux statistiques par période et au récap de l'année ;
//! le compteur d'écoutes de chaque morceau, lui, existe depuis plus longtemps (réglages).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub struct Listen {
    /// Début de l'écoute, en secondes depuis l'époque Unix.
    pub time: u64,
    pub path: PathBuf,
    /// Secondes réellement écoutées (0 : inconnu, ancienne ligne).
    pub seconds: u32,
}

fn file() -> PathBuf {
    crate::library::data_dir().join("history.tsv")
}

/// Relit l'historique (les lignes illisibles sont ignorées).
pub fn load() -> Vec<Listen> {
    let Ok(text) = std::fs::read_to_string(file()) else {
        return Vec::new();
    };
    parse(&text)
}

fn parse(text: &str) -> Vec<Listen> {
    text.lines()
        .filter_map(|l| {
            let (time, rest) = l.split_once('\t')?;
            // La durée est à la fin ; un chemin ne contient jamais de tabulation (voir `append`).
            let (path, seconds) = match rest.rsplit_once('\t') {
                Some((p, s)) => (p, s.trim().parse().unwrap_or(0)),
                None => (rest, 0),
            };
            Some(Listen {
                time: time.trim().parse().ok()?,
                path: PathBuf::from(path),
                seconds,
            })
        })
        .collect()
}

/// Ajoute une écoute à la fin du fichier.
pub fn append(listen: &Listen) {
    let _ = std::fs::create_dir_all(crate::library::data_dir());
    // Un chemin contenant un retour à la ligne casserait le format : il n'est pas noté.
    let path = listen.path.to_string_lossy();
    if path.contains('\n') || path.contains('\t') {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file()) {
        let _ = writeln!(f, "{}\t{}\t{}", listen.time, path, listen.seconds);
    }
}

/// Réécrit tout l'historique (après le renommage de fichiers).
pub fn save(all: &[Listen]) {
    let mut text = String::new();
    for l in all {
        text.push_str(&format!(
            "{}\t{}\t{}\n",
            l.time,
            l.path.to_string_lossy(),
            l.seconds
        ));
    }
    let _ = std::fs::create_dir_all(crate::library::data_dir());
    let tmp = file().with_extension("tsv.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(tmp, file());
    }
}

/// Fait suivre un fichier renommé.
pub fn rename(all: &mut [Listen], old: &Path, new: &Path) -> bool {
    let mut changed = false;
    for l in all.iter_mut().filter(|l| l.path == old) {
        l.path = new.to_path_buf();
        changed = true;
    }
    changed
}

/// Décalage de l'heure locale sur le temps universel, en secondes (commande `date`, lue une fois).
pub fn local_offset() -> i64 {
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        let out = std::process::Command::new("date").arg("+%z").output().ok();
        let text = out
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        let text = text.trim();
        if text.len() != 5 {
            return 0;
        }
        let sign = if text.starts_with('-') { -1 } else { 1 };
        let hours: i64 = text[1..3].parse().unwrap_or(0);
        let minutes: i64 = text[3..5].parse().unwrap_or(0);
        sign * (hours * 3600 + minutes * 60)
    })
}

/// Jour local (nombre de jours depuis le 1er janvier 1970) d'un horodatage.
pub fn local_day(time: u64) -> i64 {
    (time as i64 + local_offset()).div_euclid(86_400)
}

/// (année, mois, jour) d'un nombre de jours depuis l'époque Unix (calendrier grégorien, algorithme
/// « civil_from_days » de Howard Hinnant).
pub fn civil(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year as i32, month, day)
}

/// Année locale d'un horodatage.
pub fn year_of(time: u64) -> i32 {
    civil(local_day(time)).0
}

/// Jour de la semaine (0 = lundi) d'un nombre de jours depuis l'époque Unix (un jeudi).
pub fn weekday(days: i64) -> usize {
    (days + 3).rem_euclid(7) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_402), (2025, 11, 10));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(weekday(0), 3); // jeudi
        assert_eq!(weekday(20_402), 0); // lundi
    }

    #[test]
    fn lines() {
        let text = "1762790400\t/m/a.mp3\nabc\t/m/b.mp3\n1762790500\t/m/c d.flac\t187\n\n";
        let all = parse(text);
        assert_eq!(all.len(), 2);
        assert_eq!(
            (all[0].path.clone(), all[0].seconds),
            (PathBuf::from("/m/a.mp3"), 0)
        );
        assert_eq!(
            (all[1].path.clone(), all[1].seconds),
            (PathBuf::from("/m/c d.flac"), 187)
        );
        let mut all = all;
        assert!(rename(&mut all, Path::new("/m/a.mp3"), Path::new("/m/z.mp3")));
        assert_eq!(all[0].path, PathBuf::from("/m/z.mp3"));
    }
}
