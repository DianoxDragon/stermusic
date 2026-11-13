//! Taste: what plays say about each track, and the shuffle that takes it into account.
//!
//! A track's **engagement** goes from 0 to 1: the share of it really listened to, over every
//! time it started. A track listened to the end every time is worth 1; half listened, 0.5;
//! skipped after a few seconds one time out of two, half as much.

use std::path::PathBuf;

use eframe::egui;

use super::{Action, PlayStat, SterMusic};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::widgets;

/// Engagement of a track (see the module); `None` if it was never started.
pub fn engagement(stat: &PlayStat, duration_secs: f64) -> Option<f32> {
    let attempts = stat.count + stat.skips;
    if attempts == 0 {
        return None;
    }
    // Average share of the track heard on each counted play. Without a time measurement (plays
    // from before it was recorded), three quarters are assumed.
    let heard = if stat.count > 0 && stat.seconds > 0.0 && duration_secs > 0.0 {
        (stat.seconds / (stat.count as f64 * duration_secs)).clamp(0.0, 1.0) as f32
    } else {
        0.75
    };
    Some(stat.count as f32 * heard / attempts as f32)
}

/// Weight of a track in the "by my taste" shuffle: 1 for a track never started (it keeps its
/// chance to be discovered), more for those listened to often and to the end, rated well or
/// marked as favorite, less for those skipped or rated badly.
pub fn weight(stat: Option<&PlayStat>, duration_secs: f64, rating: u8, favorite: bool) -> f32 {
    let mut w = 1.0;
    if let Some((stat, e)) = stat.and_then(|s| engagement(s, duration_secs).map(|e| (s, e))) {
        // From 0.3 (always skipped) to 3 (always listened to in full), strengthened by habit.
        let habit = 1.0 + (stat.count.min(20) as f32) / 20.0;
        w = (0.3 + 2.7 * e) * habit;
    }
    w *= match rating {
        1 => 0.35,
        2 => 0.65,
        4 => 1.5,
        5 => 2.2,
        _ => 1.0,
    };
    if favorite {
        w *= 1.6;
    }
    w.clamp(0.05, 20.0)
}

/// Orders `items` by my taste: from heaviest to lightest. A slight randomness (± 12 %) only
/// breaks ties between tracks of similar weight (all never-started tracks are worth 1), so that
/// two queues in a row are not identical; a clear difference in taste is always respected.
pub fn taste_order<T>(items: &mut Vec<T>, weight: impl Fn(&T) -> f32) {
    let mut keyed: Vec<(f32, T)> = items
        .drain(..)
        .map(|item| (weight(&item) * (0.88 + 0.24 * fastrand::f32()), item))
        .collect();
    keyed.sort_by(|a, b| b.0.total_cmp(&a.0));
    items.extend(keyed.into_iter().map(|(_, item)| item));
}

impl SterMusic {
    /// Weight of library track `i` in the "by my taste" shuffle.
    pub(super) fn taste_weight(&self, i: usize) -> f32 {
        let t = &self.lib.tracks[i];
        weight(
            self.settings.plays.get(&t.path),
            t.duration_ms as f64 / 1000.0,
            self.settings.ratings.get(&t.path).copied().unwrap_or(0),
            self.settings.favorites.contains(&t.path),
        )
    }

    /// "Shuffle" button with its small arrow: the button starts random playback; the arrow (or a
    /// right click) also offers "By my taste": a queue ordered from my favorite tracks to the least
    /// liked, without changing the bar's shuffle button.
    pub(super) fn shuffle_split(&mut self, ui: &mut egui::Ui, tracks: &[usize], label: &str) {
        let (main, arrow) = widgets::split_button(ui, Some(Icon::Shuffle), label);
        let main = main.on_hover_text(tr("Lecture aléatoire (flèche : selon mes goûts)"));
        let arrow = arrow.on_hover_text(tr("Autres façons de lire"));
        if main.clicked() {
            self.actions.push(Action::PlayShuffled(tracks.to_vec()));
        }
        let popup_id = ui.make_persistent_id(("shuffle_split", main.id));
        if arrow.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup_id));
        }
        let mut chosen = None;
        let menu = |ui: &mut egui::Ui, chosen: &mut Option<bool>| {
            ui.set_min_width(240.0);
            if widgets::menu_item(ui, Some(Icon::Shuffle), tr("Au hasard")).clicked() {
                *chosen = Some(false);
            }
            if widgets::menu_item(ui, Some(Icon::Heart), tr("Selon mes goûts"))
                .on_hover_text(tr("La file est rangée par goût : d'abord les morceaux que tu écoutes jusqu'au bout, notes bien ou mets en favori ; à la fin, ceux que tu passes."))
                .clicked()
            {
                *chosen = Some(true);
            }
        };
        egui::popup::popup_above_or_below_widget(
            ui,
            popup_id,
            &arrow,
            egui::AboveOrBelow::Below,
            egui::PopupCloseBehavior::CloseOnClick,
            |ui| menu(ui, &mut chosen),
        );
        main.context_menu(|ui| menu(ui, &mut chosen));
        match chosen {
            Some(true) => self.actions.push(Action::PlayByTaste(tracks.to_vec())),
            Some(false) => self.actions.push(Action::PlayShuffled(tracks.to_vec())),
            None => {}
        }
    }

    /// Engagement of each track started at least `min_attempts` times: (track, engagement, number
    /// of starts), from strongest to weakest.
    pub(super) fn engagements(&self, min_attempts: u32) -> Vec<(usize, f32, u32)> {
        let mut v: Vec<(usize, f32, u32, PathBuf)> = self
            .settings
            .plays
            .iter()
            .filter(|(_, s)| s.count + s.skips >= min_attempts)
            .filter_map(|(p, s)| {
                let i = self.lib.find(p)?;
                let e = engagement(s, self.lib.tracks[i].duration_ms as f64 / 1000.0)?;
                Some((i, e, s.count + s.skips, p.clone()))
            })
            .collect();
        // On a tie: the most started first, then the path (stable order from one frame to the next).
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.cmp(&a.2)).then_with(|| a.3.cmp(&b.3)));
        v.into_iter().map(|(i, e, n, _)| (i, e, n)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(count: u32, skips: u32, seconds: f64) -> PlayStat {
        PlayStat {
            count,
            last: 0,
            seconds,
            skips,
        }
    }

    #[test]
    fn engagement_reads_listens_and_skips() {
        assert_eq!(engagement(&stat(0, 0, 0.0), 200.0), None);
        // Four full plays.
        assert!((engagement(&stat(4, 0, 800.0), 200.0).unwrap() - 1.0).abs() < 1e-6);
        // Two half plays.
        assert!((engagement(&stat(2, 0, 200.0), 200.0).unwrap() - 0.5).abs() < 1e-6);
        // One full play, three skips.
        assert!((engagement(&stat(1, 3, 200.0), 200.0).unwrap() - 0.25).abs() < 1e-6);
        // Always skipped.
        assert_eq!(engagement(&stat(0, 5, 0.0), 200.0), Some(0.0));
        // Old plays without measured duration: three quarters assumed.
        assert!((engagement(&stat(2, 0, 0.0), 200.0).unwrap() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn weights_follow_taste() {
        let never = weight(None, 200.0, 0, false);
        let loved = weight(Some(&stat(10, 0, 2000.0)), 200.0, 5, true);
        let skipped = weight(Some(&stat(0, 6, 0.0)), 200.0, 0, false);
        assert_eq!(never, 1.0);
        assert!(loved > 8.0 && skipped < 0.5, "{loved} {skipped}");
        assert!(weight(None, 200.0, 1, false) < never && weight(None, 200.0, 4, false) > never);
    }

    /// The queue goes from the favorite to the least liked; only very close tastes can swap.
    #[test]
    fn ordered_by_taste() {
        fastrand::seed(7);
        let weights = [1.0, 6.0, 0.3, 2.5, 1.02];
        let (mut swapped, mut same) = (0, 0);
        for _ in 0..500 {
            let mut v: Vec<usize> = (0..5).collect();
            taste_order(&mut v, |&i| weights[i]);
            assert_eq!(&v[..2], &[1, 3]);
            assert_eq!(v[4], 2);
            if v[2] == 4 {
                swapped += 1
            } else {
                same += 1
            }
        }
        // 1.0 and 1.02: almost a tie, the order varies from one queue to the next.
        assert!(swapped > 100 && same > 100, "{swapped} {same}");
    }
}
