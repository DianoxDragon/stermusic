//! Listening history: each track listened to for at least 30 seconds (or half of it if it is
//! shorter) is recorded with its date and the time really listened, in
//! `~/.local/share/stermusic/history.tsv` (one line per play: "timestamp<TAB>path<TAB>seconds";
//! the first versions lacked the last column). A track skipped before it counted is recorded
//! too, with a fourth column "s" (skipped). Used for the per-period statistics and the yearly
//! recap; each track's play counter has existed for longer (settings).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub struct Listen {
    /// Start of the play, in seconds since the Unix epoch.
    pub time: u64,
    pub path: PathBuf,
    /// Seconds really listened (0: unknown, old line).
    pub seconds: u32,
    /// Track skipped before it counted as a play.
    pub skipped: bool,
}

fn file() -> PathBuf {
    crate::library::data_dir().join("history.tsv")
}

/// Reads the history again (unreadable lines are ignored): the plays, then the skipped tracks.
pub fn load() -> (Vec<Listen>, Vec<Listen>) {
    let Ok(text) = std::fs::read_to_string(file()) else {
        return (Vec::new(), Vec::new());
    };
    parse(&text).into_iter().partition(|l| !l.skipped)
}

fn parse(text: &str) -> Vec<Listen> {
    text.lines()
        .filter_map(|l| {
            // A path never contains a tab (see `append`).
            let mut parts = l.split('\t');
            let time = parts.next()?;
            let path = parts.next()?;
            let seconds = parts.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
            let skipped = parts.next().is_some_and(|k| k.trim() == "s");
            Some(Listen {
                time: time.trim().parse().ok()?,
                path: PathBuf::from(path),
                seconds,
                skipped,
            })
        })
        .collect()
}

/// Appends a play at the end of the file.
pub fn append(listen: &Listen) {
    let _ = std::fs::create_dir_all(crate::library::data_dir());
    // A path containing a newline would break the format: it is not recorded.
    let path = listen.path.to_string_lossy();
    if path.contains('\n') || path.contains('\t') {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file()) {
        let _ = writeln!(f, "{}", line(listen));
    }
}

fn line(l: &Listen) -> String {
    let kind = if l.skipped { "\ts" } else { "" };
    format!("{}\t{}\t{}{kind}", l.time, l.path.to_string_lossy(), l.seconds)
}

/// Rewrites the whole history (after files were renamed): plays and skipped tracks, in
/// chronological order.
pub fn save(listens: &[Listen], skips: &[Listen]) {
    let mut all: Vec<&Listen> = listens.iter().chain(skips).collect();
    all.sort_by_key(|l| l.time);
    let mut text = String::new();
    for l in all {
        text.push_str(&line(l));
        text.push('\n');
    }
    let _ = std::fs::create_dir_all(crate::library::data_dir());
    let tmp = file().with_extension("tsv.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(tmp, file());
    }
}

/// Follows a renamed file.
pub fn rename(all: &mut [Listen], old: &Path, new: &Path) -> bool {
    let mut changed = false;
    for l in all.iter_mut().filter(|l| l.path == old) {
        l.path = new.to_path_buf();
        changed = true;
    }
    changed
}

/// Offset of local time from universal time, in seconds (`date` command, read once).
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

/// Local day (number of days since January 1st, 1970) of a timestamp.
pub fn local_day(time: u64) -> i64 {
    (time as i64 + local_offset()).div_euclid(86_400)
}

/// (year, month, day) from a number of days since the Unix epoch (Gregorian calendar, Howard
/// Hinnant's "civil_from_days" algorithm).
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

/// Local year of a timestamp.
pub fn year_of(time: u64) -> i32 {
    civil(local_day(time)).0
}

/// Day of the week (0 = Monday) from a number of days since the Unix epoch (a Thursday).
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
        assert_eq!(weekday(0), 3); // Thursday
        assert_eq!(weekday(20_402), 0); // Monday
    }

    #[test]
    fn lines() {
        let text = "1762790400\t/m/a.mp3\nabc\t/m/b.mp3\n1762790500\t/m/c d.flac\t187\n\n1762790600\t/m/d.mp3\t12\ts\n";
        let all = parse(text);
        assert_eq!(all.len(), 3);
        assert!(all[2].skipped && !all[1].skipped && all[2].seconds == 12);
        assert_eq!(line(&all[2]), "1762790600\t/m/d.mp3\t12\ts");
        assert_eq!(line(&all[1]), "1762790500\t/m/c d.flac\t187");
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
