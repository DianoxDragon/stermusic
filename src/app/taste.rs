//! Goûts : ce que les écoutes disent de chaque morceau, et l'aléatoire qui en tient compte.
//!
//! L'**engagement** d'un morceau va de 0 à 1 : la part de ce qu'on en a vraiment écouté, sur
//! toutes les fois où il a commencé. Un morceau écouté jusqu'au bout à chaque fois vaut 1 ; écouté
//! à moitié, 0,5 ; passé au bout de quelques secondes une fois sur deux, deux fois moins.

use std::path::PathBuf;

use eframe::egui;

use super::{Action, PlayStat, SterMusic};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::widgets;

/// Engagement d'un morceau (voir le module) ; `None` s'il n'a jamais été lancé.
pub fn engagement(stat: &PlayStat, duration_secs: f64) -> Option<f32> {
    let attempts = stat.count + stat.skips;
    if attempts == 0 {
        return None;
    }
    // Part moyenne du morceau entendue à chaque écoute comptée. Sans mesure du temps (écoutes
    // d'avant qu'il soit noté), on suppose trois quarts.
    let heard = if stat.count > 0 && stat.seconds > 0.0 && duration_secs > 0.0 {
        (stat.seconds / (stat.count as f64 * duration_secs)).clamp(0.0, 1.0) as f32
    } else {
        0.75
    };
    Some(stat.count as f32 * heard / attempts as f32)
}

/// Poids d'un morceau dans l'aléatoire « selon mes goûts » : 1 pour un morceau jamais lancé (il
/// garde sa chance d'être découvert), plus pour ceux qu'on écoute souvent et jusqu'au bout, qu'on
/// a bien notés ou mis en favori, moins pour ceux qu'on passe ou qu'on a mal notés.
pub fn weight(stat: Option<&PlayStat>, duration_secs: f64, rating: u8, favorite: bool) -> f32 {
    let mut w = 1.0;
    if let Some((stat, e)) = stat.and_then(|s| engagement(s, duration_secs).map(|e| (s, e))) {
        // De 0,3 (toujours passé) à 3 (toujours écouté en entier), renforcé par l'habitude.
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

/// Range `items` selon mes goûts : du plus lourd au plus léger. Un léger aléa (± 12 %) départage
/// seulement les morceaux de poids voisin (tous les jamais lancés valent 1), pour que deux files
/// de suite ne soient pas identiques ; un écart net de goût est toujours respecté.
pub fn taste_order<T>(items: &mut Vec<T>, weight: impl Fn(&T) -> f32) {
    let mut keyed: Vec<(f32, T)> = items
        .drain(..)
        .map(|item| (weight(&item) * (0.88 + 0.24 * fastrand::f32()), item))
        .collect();
    keyed.sort_by(|a, b| b.0.total_cmp(&a.0));
    items.extend(keyed.into_iter().map(|(_, item)| item));
}

impl SterMusic {
    /// Poids du morceau `i` de la bibliothèque dans l'aléatoire « selon mes goûts ».
    pub(super) fn taste_weight(&self, i: usize) -> f32 {
        let t = &self.lib.tracks[i];
        weight(
            self.settings.plays.get(&t.path),
            t.duration_ms as f64 / 1000.0,
            self.settings.ratings.get(&t.path).copied().unwrap_or(0),
            self.settings.favorites.contains(&t.path),
        )
    }

    /// Bouton « Aléatoire » avec sa petite flèche : le bouton lance la lecture au hasard ; la
    /// flèche (ou un clic droit) propose aussi « Selon mes goûts » : une file rangée de mes
    /// morceaux préférés aux moins aimés, sans rien changer au bouton aléatoire de la barre.
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

    /// Engagement de chaque morceau lancé au moins `min_attempts` fois : (morceau, engagement,
    /// nombre de lancements), du plus fort au plus faible.
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
        // À égalité : le plus lancé d'abord, puis le chemin (ordre stable d'une image à l'autre).
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
        // Quatre écoutes entières.
        assert!((engagement(&stat(4, 0, 800.0), 200.0).unwrap() - 1.0).abs() < 1e-6);
        // Deux écoutes à moitié.
        assert!((engagement(&stat(2, 0, 200.0), 200.0).unwrap() - 0.5).abs() < 1e-6);
        // Une écoute entière, trois sauts.
        assert!((engagement(&stat(1, 3, 200.0), 200.0).unwrap() - 0.25).abs() < 1e-6);
        // Toujours passé.
        assert_eq!(engagement(&stat(0, 5, 0.0), 200.0), Some(0.0));
        // Écoutes anciennes sans durée mesurée : trois quarts supposés.
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

    /// La file va du préféré au moins aimé ; seuls des goûts très proches peuvent s'échanger.
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
        // 1,0 et 1,02 : presque à égalité, l'ordre varie d'une file à l'autre.
        assert!(swapped > 100 && same > 100, "{swapped} {same}");
    }
}
