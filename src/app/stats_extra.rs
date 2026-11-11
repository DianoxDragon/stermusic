//! Statistiques, suite : la manière d'écouter (écoutes complètes, partielles, morceaux passés),
//! l'activité au fil du temps, les heures et les jours, les morceaux préférés (meilleur
//! engagement) et ceux qu'on passe le plus.
//!
//! Tout cela vient de l'historique daté ; les morceaux passés n'y sont notés que depuis la
//! version qui a introduit ces chiffres.

use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense};

use super::stats::Period;
use super::{now_secs, Action, SterMusic};
use crate::history::{self, Listen};
use crate::i18n::tr;
use crate::library::plural;
use crate::{anim, theme, trf, widgets};

/// Une écoute est complète si l'on est allé (presque) au bout : neuf dixièmes du morceau, ou
/// ses quinze dernières secondes (un fondu enchaîné coupe la fin).
pub fn is_complete(heard_secs: f64, duration_secs: f64) -> bool {
    duration_secs > 0.0 && heard_secs >= (0.9 * duration_secs).min(duration_secs - 15.0)
}

fn in_period(time: u64, period: Period) -> bool {
    match period {
        Period::All => true,
        Period::Year => history::year_of(time) == history::year_of(now_secs()),
        Period::Month => time >= now_secs().saturating_sub(30 * 86_400),
    }
}

/// Manière d'écouter sur une période.
#[derive(Default, Debug, PartialEq)]
pub struct Habits {
    pub complete: u64,
    pub partial: u64,
    pub skipped: u64,
    /// Écoutes par heure de la journée (heure locale) et par jour de la semaine (lundi d'abord).
    pub hours: [u64; 24],
    pub weekdays: [u64; 7],
}

impl Habits {
    /// `duration` donne la durée d'un morceau (secondes), s'il est encore dans la bibliothèque.
    pub fn compute<'a>(
        listens: impl Iterator<Item = &'a Listen>,
        skips: impl Iterator<Item = &'a Listen>,
        duration: impl Fn(&Listen) -> Option<f64>,
    ) -> Self {
        let mut h = Self::default();
        for l in listens {
            let local = l.time as i64 + history::local_offset();
            h.hours[(local.rem_euclid(86_400) / 3600) as usize] += 1;
            h.weekdays[history::weekday(history::local_day(l.time))] += 1;
            // Sans durée mesurée (anciennes lignes) ou sans le morceau, on ne peut pas trancher.
            if l.seconds > 0 {
                if let Some(d) = duration(l) {
                    if is_complete(l.seconds as f64, d) {
                        h.complete += 1;
                    } else {
                        h.partial += 1;
                    }
                }
            }
        }
        h.skipped = skips.count() as u64;
        h
    }
}

/// Écoutes par tranche de temps : les valeurs, le nom de chaque tranche (bulle), et l'étiquette
/// affichée sous certaines.
struct Activity {
    values: Vec<u64>,
    names: Vec<String>,
    labels: Vec<String>,
}

fn month_short(m: u32) -> &'static str {
    [
        tr("janv."),
        tr("févr."),
        tr("mars"),
        tr("avr."),
        tr("mai"),
        tr("juin"),
        tr("juil."),
        tr("août"),
        tr("sept."),
        tr("oct."),
        tr("nov."),
        tr("déc."),
    ][(m.clamp(1, 12) - 1) as usize]
}

fn activity(listens: &[Listen], period: Period) -> Activity {
    let today = history::local_day(now_secs());
    match period {
        // Trente jours, un par colonne.
        Period::Month => {
            let mut values = vec![0u64; 30];
            for l in listens {
                let ago = today - history::local_day(l.time);
                if (0..30).contains(&ago) {
                    values[29 - ago as usize] += 1;
                }
            }
            let days: Vec<(i32, u32, u32)> = (0..30).map(|k| history::civil(today - 29 + k)).collect();
            Activity {
                values,
                names: days.iter().map(|&(_, m, d)| format!("{d} {}", month_short(m))).collect(),
                labels: days
                    .iter()
                    .enumerate()
                    .map(|(k, &(_, m, d))| if k % 5 == 4 { format!("{d}/{m}") } else { String::new() })
                    .collect(),
            }
        }
        // Les mois : ceux de l'année, ou les vingt-quatre derniers.
        _ => {
            let (year, month, _) = history::civil(today);
            let index = |y: i32, m: u32| y as i64 * 12 + m as i64 - 1;
            let last = index(year, month);
            let (first, n) = if period == Period::Year {
                (index(year, 1), 12)
            } else {
                let oldest = listens
                    .iter()
                    .map(|l| {
                        let (y, m, _) = history::civil(history::local_day(l.time));
                        index(y, m)
                    })
                    .min()
                    .unwrap_or(last);
                let first = oldest.max(last - 23).min(last - 5);
                (first, (last - first + 1) as usize)
            };
            let mut values = vec![0u64; n];
            for l in listens {
                let (y, m, _) = history::civil(history::local_day(l.time));
                let k = index(y, m) - first;
                if (0..n as i64).contains(&k) {
                    values[k as usize] += 1;
                }
            }
            let months: Vec<(i64, u32)> = (0..n as i64)
                .map(|k| ((first + k).div_euclid(12), (first + k).rem_euclid(12) as u32 + 1))
                .collect();
            Activity {
                values,
                names: months.iter().map(|&(y, m)| format!("{} {y}", month_short(m))).collect(),
                labels: months
                    .iter()
                    .map(|&(y, m)| {
                        if n <= 12 || m == 1 || m == 7 {
                            if m == 1 && n > 12 {
                                y.to_string()
                            } else {
                                month_short(m).to_owned()
                            }
                        } else {
                            String::new()
                        }
                    })
                    .collect(),
            }
        }
    }
}

/// Une note en petit, qui passe à la ligne dans sa colonne.
fn note(ui: &mut egui::Ui, text: &str, small: bool) {
    let mut rich = RichText::new(text).color(theme::TEXT_3);
    if small {
        rich = rich.small();
    }
    ui.add(egui::Label::new(rich).wrap());
}

/// Diagramme en colonnes : une colonne par valeur, son nom et sa valeur en bulle au survol, une
/// étiquette sous celles qui en ont une.
fn columns(ui: &mut egui::Ui, id: &str, values: &[u64], names: &[String], labels: &[String], w: f32, h: f32) {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h + 20.0), Sense::hover());
    let n = values.len().max(1);
    let max = values.iter().copied().max().unwrap_or(0).max(1) as f32;
    let t = anim::appear(ui.ctx(), egui::Id::new((id, "columns")), anim::SLOW);
    let step = rect.width() / n as f32;
    let bar_w = (step * 0.72).clamp(2.0, 34.0);
    let base = rect.top() + h;
    let hover = resp.hover_pos().map(|p| ((p.x - rect.left()) / step).floor() as usize);
    let painter = ui.painter();
    painter.hline(rect.x_range(), base, egui::Stroke::new(1.0_f32, theme::white(0x16)));
    for (k, &v) in values.iter().enumerate() {
        let cx = rect.left() + (k as f32 + 0.5) * step;
        let bh = (h - 6.0) * v as f32 / max * t;
        let hot = hover == Some(k);
        if v > 0 {
            let bar = Rect::from_min_max(pos2(cx - bar_w / 2.0, base - bh.max(2.0)), pos2(cx + bar_w / 2.0, base));
            let radius = (bar_w / 2.0).min(4.0) as u8;
            let corners = CornerRadius { nw: radius, ne: radius, sw: 0, se: 0 };
            painter.rect_filled(bar, corners, theme::alpha(theme::accent(), if hot { 0xFF } else { 0x90 }));
        } else {
            painter.circle_filled(pos2(cx, base - 2.0), 1.2, theme::white(0x30));
        }
        if let Some(label) = labels.get(k).filter(|l| !l.is_empty()) {
            painter.text(pos2(cx, base + 11.0), Align2::CENTER_CENTER, label, FontId::proportional(11.0), theme::TEXT_3);
        }
    }
    if let Some(k) = hover.filter(|&k| k < values.len()) {
        let name = names.get(k).cloned().unwrap_or_default();
        resp.on_hover_text(format!("{name} : {}", plural(values[k] as usize, "écoute", "écoutes")));
    }
}

impl SterMusic {
    fn habits(&self, period: Period) -> Habits {
        Habits::compute(
            self.listens.iter().filter(|l| in_period(l.time, period)),
            self.skips.iter().filter(|l| in_period(l.time, period)),
            |l| self.lib.find(&l.path).map(|i| self.lib.tracks[i].duration_ms as f64 / 1000.0),
        )
    }

    /// « Comment tu écoutes », l'activité, les heures et les jours.
    pub(super) fn stats_habits(&mut self, ui: &mut egui::Ui, period: Period, w: f32) {
        let habits = self.habits(period);
        let dated: Vec<Listen> = self.listens.iter().filter(|l| in_period(l.time, period)).cloned().collect();
        if dated.is_empty() && habits.skipped == 0 {
            return;
        }
        let col_w = (w - 24.0) / 2.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            ui.vertical(|ui| {
                ui.set_width(col_w);
                super::lists::section(ui, tr("Comment tu écoutes"), None);
                self.completion_bar(ui, &habits, col_w);
            });
            ui.vertical(|ui| {
                ui.set_width(col_w);
                let title = match period {
                    Period::Month => tr("Jour après jour"),
                    _ => tr("Mois après mois"),
                };
                super::lists::section(ui, title, None);
                let a = activity(&dated, period);
                columns(ui, "stat_activity", &a.values, &a.names, &a.labels, col_w, 96.0);
            });
        });
        ui.add_space(18.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            ui.vertical(|ui| {
                ui.set_width(col_w);
                super::lists::section(ui, tr("Les heures où tu écoutes"), None);
                let names: Vec<String> = (0..24).map(|h| format!("{h} h – {} h", h + 1)).collect();
                let labels: Vec<String> = (0..24)
                    .map(|h| if h % 3 == 0 { format!("{h} h") } else { String::new() })
                    .collect();
                columns(ui, "stat_hours", &habits.hours, &names, &labels, col_w, 84.0);
            });
            ui.vertical(|ui| {
                ui.set_width(col_w);
                super::lists::section(ui, tr("Les jours où tu écoutes"), None);
                let days = [
                    tr("lundi"),
                    tr("mardi"),
                    tr("mercredi"),
                    tr("jeudi"),
                    tr("vendredi"),
                    tr("samedi"),
                    tr("dimanche"),
                ];
                let names: Vec<String> = days.iter().map(|d| (*d).to_owned()).collect();
                let labels: Vec<String> = days.iter().map(|d| d.chars().take(3).collect()).collect();
                columns(ui, "stat_days", &habits.weekdays, &names, &labels, col_w, 84.0);
            });
        });
    }

    /// Barre en trois parts : écoutes complètes, partielles, morceaux passés.
    fn completion_bar(&self, ui: &mut egui::Ui, h: &Habits, w: f32) {
        let total = h.complete + h.partial + h.skipped;
        if total == 0 {
            note(
                ui,
                tr("Les écoutes complètes, partielles et les morceaux passés sont comptés à partir de cette version : reviens après quelques écoutes."),
                false,
            );
            return;
        }
        let parts = [
            (h.complete, theme::accent(), tr("complètes"), tr("Écoutées jusqu'au bout (ou presque).")),
            (
                h.partial,
                theme::mix(theme::accent(), theme::accent_2(), 0.6).gamma_multiply(0.75),
                tr("partielles"),
                tr("Écoutées au moins 30 secondes (ou la moitié), mais pas jusqu'au bout."),
            ),
            (
                h.skipped,
                theme::white(0x38),
                tr("passés"),
                tr("Morceaux quittés pour un autre avant d'avoir compté comme une écoute."),
            ),
        ];
        let (bar, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
        let t = anim::appear(ui.ctx(), egui::Id::new("stat_completion"), anim::SLOW);
        ui.painter().rect_filled(bar, CornerRadius::same(6), theme::white(0x0C));
        let mut x = bar.left();
        for (k, (n, color, _, _)) in parts.iter().enumerate() {
            let pw = bar.width() * *n as f32 / total as f32 * t;
            if pw < 0.5 {
                continue;
            }
            let r = Rect::from_min_size(pos2(x, bar.top()), vec2(pw, bar.height()));
            let first = x <= bar.left() + 0.5;
            let last = k == parts.len() - 1 || parts[k + 1..].iter().all(|p| p.0 == 0);
            let corners = CornerRadius {
                nw: if first { 6 } else { 0 },
                sw: if first { 6 } else { 0 },
                ne: if last { 6 } else { 0 },
                se: if last { 6 } else { 0 },
            };
            ui.painter().rect_filled(r, corners, *color);
            x += pw;
        }
        ui.add_space(8.0);
        for (n, color, label, tip) in parts {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
            ui.painter().circle_filled(pos2(r.left() + 8.0, r.center().y), 5.0, color);
            ui.painter().text(
                pos2(r.left() + 22.0, r.center().y),
                Align2::LEFT_CENTER,
                format!("{n} {label}"),
                FontId::proportional(13.5),
                theme::TEXT,
            );
            ui.painter().text(
                pos2(r.right() - 8.0, r.center().y),
                Align2::RIGHT_CENTER,
                crate::i18n::percent(100.0 * n as f32 / total as f32),
                FontId::proportional(13.0),
                theme::TEXT_2,
            );
            resp.on_hover_text(tip);
        }
    }

    /// « Tes préférés » (meilleur engagement) et « Tu les passes souvent ».
    pub(super) fn stats_taste(&mut self, ui: &mut egui::Ui, w: f32) {
        let favorites: Vec<(usize, String, f32)> = self
            .engagements(3)
            .into_iter()
            .take(8)
            .map(|(i, e, n)| {
                let label = trf!("{0} sur {1}", crate::i18n::percent(100.0 * e), plural(n as usize, "lancement", "lancements"));
                (i, label, e)
            })
            .collect();
        let mut skipped: Vec<(usize, u32)> = self
            .settings
            .plays
            .iter()
            .filter(|(_, s)| s.skips >= 2)
            .filter_map(|(p, s)| Some((self.lib.find(p)?, s.skips)))
            .collect();
        skipped.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let most = skipped.first().map(|s| s.1).unwrap_or(1).max(1) as f32;
        let skipped: Vec<(usize, String, f32)> = skipped
            .into_iter()
            .take(8)
            .map(|(i, n)| (i, trf!("passé {0} fois", n), n as f32 / most))
            .collect();
        if favorites.is_empty() && skipped.is_empty() {
            return;
        }
        let col_w = (w - 24.0) / 2.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            ui.vertical(|ui| {
                ui.set_width(col_w);
                super::lists::section(ui, tr("Tes préférés"), None);
                if favorites.is_empty() {
                    note(ui, tr("Il faut au moins trois lancements d'un morceau pour le classer ici."), false);
                } else {
                    note(
                        ui,
                        tr("Ceux que tu écoutes le plus jusqu'au bout, à chaque fois qu'ils passent. C'est ce classement qui guide l'aléatoire « selon mes goûts »."),
                        true,
                    );
                    ui.add_space(4.0);
                    self.taste_rows(ui, &favorites, col_w, true);
                }
            });
            ui.vertical(|ui| {
                ui.set_width(col_w);
                super::lists::section(ui, tr("Tu les passes souvent"), None);
                if skipped.is_empty() {
                    note(ui, tr("Aucun morceau passé au moins deux fois pour l'instant."), false);
                } else {
                    note(ui, tr("Quittés pour un autre morceau avant d'avoir compté comme une écoute."), true);
                    ui.add_space(4.0);
                    self.taste_rows(ui, &skipped, col_w, false);
                }
            });
        });
    }

    /// Lignes de morceaux avec une valeur et une petite barre (0 → 1).
    fn taste_rows(&mut self, ui: &mut egui::Ui, rows: &[(usize, String, f32)], w: f32, accent: bool) {
        for (k, (i, label, ratio)) in rows.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 44.0), Sense::click());
            widgets::paint_interaction(ui, r, 6, &resp);
            let t = self.lib.tracks[*i].clone();
            let cr = Rect::from_min_size(pos2(r.left() + 6.0, r.top() + 5.0), vec2(34.0, 34.0));
            let tex = self.covers.thumb(&t.path).cloned();
            widgets::cover(ui.painter(), cr, tex.as_ref(), 4, &t.album);
            let tw = r.width() - 34.0 - 30.0 - 170.0;
            let g = widgets::galley(ui, &t.title, FontId::proportional(13.5), theme::TEXT, tw);
            ui.painter().galley(pos2(cr.right() + 12.0, r.top() + 5.0), g, theme::TEXT);
            let g = widgets::galley(ui, t.artist_or_unknown(), FontId::proportional(12.0), theme::TEXT_2, tw);
            ui.painter().galley(pos2(cr.right() + 12.0, r.top() + 23.0), g, theme::TEXT_2);
            let bar = Rect::from_min_size(pos2(r.right() - 82.0, r.center().y + 9.0), vec2(70.0, 3.0));
            ui.painter().rect_filled(bar, CornerRadius::same(2), theme::white(0x14));
            let fill = Rect::from_min_size(bar.min, vec2(70.0 * ratio.clamp(0.0, 1.0), 3.0));
            if accent {
                theme::brand_fill(ui.painter(), fill, 2, 1.0);
            } else {
                ui.painter().rect_filled(fill, CornerRadius::same(2), Color32::from_gray(150));
            }
            ui.painter().text(
                pos2(r.right() - 12.0, r.center().y - 6.0),
                Align2::RIGHT_CENTER,
                label,
                FontId::proportional(12.0),
                theme::TEXT_2,
            );
            if resp.double_clicked() {
                let list: Vec<usize> = rows.iter().map(|r| r.0).collect();
                self.actions.push(Action::Play { tracks: list, start: k });
            }
            resp.on_hover_text(tr("Double-clic pour écouter"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_or_partial() {
        // Morceau de 4 minutes : complet à partir de 3 min 36 (neuf dixièmes).
        assert!(is_complete(216.0, 240.0) && !is_complete(200.0, 240.0));
        // Morceau d'une minute : ses quinze dernières secondes peuvent manquer (fondu).
        assert!(is_complete(45.0, 60.0) && !is_complete(40.0, 60.0));
        assert!(!is_complete(10.0, 0.0));
    }

    #[test]
    fn habits_split_listens() {
        let listen = |time: u64, seconds: u32, skipped: bool| Listen {
            time,
            path: "/m/a.flac".into(),
            seconds,
            skipped,
        };
        let listens = [listen(1_762_790_400, 235, false), listen(1_762_794_000, 90, false), listen(1_762_797_600, 0, false)];
        let skips = [listen(1_762_790_000, 6, true)];
        let h = Habits::compute(listens.iter(), skips.iter(), |_| Some(240.0));
        // La troisième écoute n'a pas de durée mesurée : ni complète ni partielle.
        assert_eq!((h.complete, h.partial, h.skipped), (1, 1, 1));
        assert_eq!(h.hours.iter().sum::<u64>(), 3);
        assert_eq!(h.weekdays.iter().sum::<u64>(), 3);
    }
}
