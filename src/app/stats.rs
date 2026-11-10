//! Statistiques d'écoute et récap de l'année, à la Spotify Wrapped.
//!
//! « Depuis toujours » repose sur le compteur d'écoutes de chaque morceau ; les périodes (cette
//! année, 30 derniers jours) et le récap reposent sur l'historique daté (`history.rs`), qui
//! commence avec cette version de SterMusic.

use std::collections::HashMap;

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense,
};
use serde::{Deserialize, Serialize};

use super::views::{heading, segment};
use super::{now_secs, Action, SterMusic, View};
use crate::history::{self, Listen};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::library::{fmt_long, plural, Library};
use crate::{anim, theme, trf, widgets};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Period {
    #[default]
    All,
    Year,
    Month,
}

/// Chiffres d'une période.
pub struct Stats {
    pub listens: u64,
    pub seconds: f64,
    /// Morceaux, artistes, albums et genres, du plus écouté au moins écouté.
    pub tracks: Vec<(usize, u64)>,
    pub artists: Vec<(String, u64)>,
    pub albums: Vec<(usize, u64)>,
    pub genres: Vec<(String, u64)>,
}

/// Du plus écouté au moins écouté. À égalité, l'ordre alphabétique départage : sans lui,
/// l'ordre changeait à chaque image (celui d'une table de hachage) et les ex æquo clignotaient.
fn ranked<K: std::hash::Hash + Ord>(map: HashMap<K, u64>) -> Vec<(K, u64)> {
    let mut v: Vec<(K, u64)> = map.into_iter().filter(|e| e.1 > 0).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

/// Écoutes d'un morceau : leur nombre et le temps vraiment écouté (secondes).
pub type Heard = HashMap<usize, (u64, f64)>;

/// Chiffres d'après les écoutes de chaque morceau (indices de la bibliothèque).
pub fn compute(lib: &Library, counts: &Heard) -> Stats {
    let mut artists: HashMap<String, u64> = HashMap::new();
    let mut albums: HashMap<usize, u64> = HashMap::new();
    let mut genres: HashMap<String, u64> = HashMap::new();
    let (mut listens, mut seconds) = (0, 0.0);
    for (&i, &(n, heard)) in counts {
        let t = &lib.tracks[i];
        listens += n;
        seconds += heard;
        *artists.entry(t.artist_or_unknown().to_owned()).or_default() += n;
        *albums.entry(lib.track_album[i]).or_default() += n;
        if !t.genre.trim().is_empty() {
            *genres.entry(t.genre.trim().to_owned()).or_default() += n;
        }
    }
    let mut tracks: Vec<(usize, u64)> = counts
        .iter()
        .map(|(&i, &(n, _))| (i, n))
        .filter(|e| e.1 > 0)
        .collect();
    tracks.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Stats {
        listens,
        seconds,
        tracks,
        artists: ranked(artists),
        albums: ranked(albums),
        genres: ranked(genres),
    }
}

/// Écoutes de l'historique, par morceau de la bibliothèque. Une écoute notée sans durée (premières
/// versions de l'historique) compte pour le morceau entier.
fn count_listens<'a>(lib: &Library, listens: impl Iterator<Item = &'a Listen>) -> Heard {
    let mut counts: Heard = HashMap::new();
    for l in listens {
        if let Some(i) = lib.find(&l.path) {
            let heard = if l.seconds > 0 {
                l.seconds as f64
            } else {
                lib.tracks[i].duration_ms as f64 / 1000.0
            };
            let e = counts.entry(i).or_default();
            e.0 += 1;
            e.1 += heard;
        }
    }
    counts
}

/// Le récap d'une année.
pub struct Recap {
    pub stats: Stats,
    /// Jour de la semaine (0 = lundi) et mois (1 → 12) où l'on a le plus écouté.
    pub weekday: Option<usize>,
    pub month: Option<u32>,
    /// Morceaux écoutés pour la première fois cette année-là.
    pub discoveries: usize,
    /// Date de la première écoute de l'historique (jours depuis l'époque Unix).
    pub first_day: Option<i64>,
}

pub fn recap(lib: &Library, all: &[Listen], year: i32) -> Recap {
    let mine: Vec<&Listen> = all.iter().filter(|l| history::year_of(l.time) == year).collect();
    let stats = compute(lib, &count_listens(lib, mine.iter().copied()));
    let mut days = [0u64; 7];
    let mut months = [0u64; 12];
    for l in &mine {
        let d = history::local_day(l.time);
        days[history::weekday(d)] += 1;
        months[(history::civil(d).1 as usize).saturating_sub(1).min(11)] += 1;
    }
    let best = |v: &[u64]| (0..v.len()).filter(|&i| v[i] > 0).max_by_key(|&i| v[i]);
    let before: std::collections::HashSet<&std::path::Path> = all
        .iter()
        .filter(|l| history::year_of(l.time) < year)
        .map(|l| l.path.as_path())
        .collect();
    let mut seen = std::collections::HashSet::new();
    let discoveries = mine
        .iter()
        .filter(|l| !before.contains(l.path.as_path()) && seen.insert(l.path.as_path()))
        .count();
    Recap {
        stats,
        weekday: best(&days),
        month: best(&months).map(|m| m as u32 + 1),
        discoveries,
        first_day: all.iter().map(|l| history::local_day(l.time)).min(),
    }
}

fn weekday_name(d: usize) -> &'static str {
    const FR: [&str; 7] = [
        "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
    ];
    const EN: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    match crate::i18n::lang() {
        crate::i18n::Lang::Fr => FR[d.min(6)],
        crate::i18n::Lang::En => EN[d.min(6)],
    }
}

fn month_name(m: u32) -> &'static str {
    const FR: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];
    const EN: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let i = (m as usize).saturating_sub(1).min(11);
    match crate::i18n::lang() {
        crate::i18n::Lang::Fr => FR[i],
        crate::i18n::Lang::En => EN[i],
    }
}

/// « 10 novembre 2026 » / « November 10, 2026 ».
pub fn date_text(day: i64) -> String {
    let (y, m, d) = history::civil(day);
    match crate::i18n::lang() {
        crate::i18n::Lang::Fr => format!("{d} {} {y}", month_name(m)),
        crate::i18n::Lang::En => format!("{} {d}, {y}", month_name(m)),
    }
}

/// Nombre lisible : « 12 345 » (espace fine) ou « 12,345 ».
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let sep = match crate::i18n::lang() {
        crate::i18n::Lang::Fr => "\u{202f}",
        crate::i18n::Lang::En => ",",
    };
    let mut out = String::new();
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push_str(sep);
        }
        out.push(c);
    }
    out
}

impl SterMusic {
    fn stats_for(&self, period: Period) -> Stats {
        let mut counts: Heard = match period {
            // Le temps écouté n'est mesuré que depuis peu : sans mesure, le morceau entier par écoute.
            Period::All => self
                .settings
                .plays
                .iter()
                .filter_map(|(p, s)| {
                    let i = self.lib.find(p)?;
                    let heard = if s.seconds > 0.0 {
                        s.seconds
                    } else {
                        s.count as f64 * self.lib.tracks[i].duration_ms as f64 / 1000.0
                    };
                    Some((i, (s.count as u64, heard)))
                })
                .collect(),
            Period::Year => {
                let year = history::year_of(now_secs());
                count_listens(
                    &self.lib,
                    self.listens.iter().filter(|l| history::year_of(l.time) == year),
                )
            }
            Period::Month => {
                let since = now_secs().saturating_sub(30 * 86_400);
                count_listens(&self.lib, self.listens.iter().filter(|l| l.time >= since))
            }
        };
        // L'écoute en cours n'entre dans l'historique qu'à la fin du morceau : on l'ajoute ici
        // pour que les chiffres bougent en direct (pour « Depuis toujours », le nombre d'écoutes
        // y est déjà, seul le temps manque).
        if let Some((path, heard)) = self.listening_now() {
            if let Some(i) = self.lib.find(&path) {
                let e = counts.entry(i).or_default();
                if period != Period::All {
                    e.0 += 1;
                }
                e.1 += heard as f64;
            }
        }
        compute(&self.lib, &counts)
    }

    /// Années présentes dans l'historique, la plus récente d'abord.
    fn history_years(&self) -> Vec<i32> {
        let mut years: Vec<i32> = self.listens.iter().map(|l| history::year_of(l.time)).collect();
        years.sort_unstable();
        years.dedup();
        years.reverse();
        years
    }

    pub(super) fn stats_page(&mut self, ui: &mut egui::Ui) {
        let period = self.settings.stats_period;
        ui.horizontal(|ui| {
            heading(ui, tr("Statistiques"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                ui.spacing_mut().item_spacing.x = 6.0;
                for (p, label) in [
                    (Period::Month, tr("30 derniers jours")),
                    (Period::Year, tr("Cette année")),
                    (Period::All, tr("Depuis toujours")),
                ] {
                    if segment(ui, label, period == p).clicked() {
                        self.settings.stats_period = p;
                    }
                }
            });
        });
        ui.add_space(16.0);
        let stats = self.stats_for(period);
        let years = self.history_years();
        egui::ScrollArea::vertical()
            .id_salt("stats")
            .auto_shrink(false)
            .show(ui, |ui| {
                let w = ui.available_width() - 16.0;
                ui.set_width(w);
                // Le récap de l'année, en tête.
                if let Some(&year) = years.first() {
                    self.recap_banner(ui, year, &years[1..], w);
                    ui.add_space(18.0);
                }
                if stats.listens == 0 {
                    let sub = if period == Period::All {
                        tr("Les morceaux écoutés au moins 30 secondes sont comptés ici.")
                    } else {
                        tr("Rien d'écouté sur cette période. L'historique daté commence avec cette version de SterMusic.")
                    };
                    widgets::empty_state(ui, Icon::Clock, tr("Pas encore d'écoutes"), sub);
                    return;
                }
                let artists = stats.artists.len();
                let tiles = [
                    (grouped(stats.listens), tr("écoutes")),
                    (fmt_long(std::time::Duration::from_secs_f64(stats.seconds)), tr("d'écoute")),
                    (grouped(stats.tracks.len() as u64), tr("morceaux différents")),
                    (grouped(artists as u64), tr("artistes différents")),
                ];
                let tile_w = (w - 3.0 * 12.0) / 4.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    for (k, (value, label)) in tiles.iter().enumerate() {
                        let (r, _) = ui.allocate_exact_size(vec2(tile_w, 86.0), Sense::hover());
                        let t = anim::appear(ui.ctx(), egui::Id::new(("stat_tile", k, period as u8)), anim::SLOW);
                        let r = r.translate(vec2(0.0, 10.0 * (1.0 - t)));
                        ui.painter()
                            .rect_filled(r, CornerRadius::same(10), theme::white((0x0A as f32 * t) as u8));
                        ui.painter().text(
                            pos2(r.left() + 16.0, r.top() + 16.0),
                            Align2::LEFT_TOP,
                            value,
                            theme::semibold(26.0),
                            theme::accent_light().gamma_multiply(t),
                        );
                        ui.painter().text(
                            pos2(r.left() + 16.0, r.top() + 54.0),
                            Align2::LEFT_TOP,
                            *label,
                            FontId::proportional(13.0),
                            theme::TEXT_2.gamma_multiply(t),
                        );
                    }
                });
                ui.add_space(22.0);
                let col_w = (w - 24.0) / 2.0;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 24.0;
                    ui.vertical(|ui| {
                        ui.set_width(col_w);
                        super::lists::section(ui, tr("Morceaux les plus écoutés"), None);
                        self.top_tracks(ui, &stats.tracks, 10, col_w);
                    });
                    ui.vertical(|ui| {
                        ui.set_width(col_w);
                        super::lists::section(ui, tr("Artistes les plus écoutés"), None);
                        bars(ui, &stats.artists, 10, col_w, "stat_artists");
                        if !stats.genres.is_empty() {
                            ui.add_space(18.0);
                            super::lists::section(ui, tr("Genres"), None);
                            bars(ui, &stats.genres, 6, col_w, "stat_genres");
                        }
                    });
                });
                if !stats.albums.is_empty() {
                    ui.add_space(18.0);
                    super::lists::section(ui, tr("Albums les plus écoutés"), None);
                    let albums: Vec<usize> = stats.albums.iter().take(6).map(|a| a.0).collect();
                    let mut c = self.view_ctx();
                    super::lists::grid(ui, "stat_albums", albums.len(), Some(1), |ui, k, w| {
                        super::lists::album_card(ui, &mut c, albums[k], w)
                    });
                }
                if period != Period::All {
                    if let Some(first) = self.listens.iter().map(|l| history::local_day(l.time)).min() {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(trf!("Historique daté depuis le {0}.", date_text(first)))
                                .small()
                                .color(theme::TEXT_3),
                        );
                    }
                }
                ui.add_space(24.0);
            });
    }

    /// Bandeau « Ton année en musique » qui ouvre le récap.
    fn recap_banner(&mut self, ui: &mut egui::Ui, year: i32, others: &[i32], w: f32) {
        let (r, resp) = ui.allocate_exact_size(vec2(w, 96.0), Sense::click());
        let h = widgets::hover_t(ui, resp.id, resp.hovered());
        theme::brand_fill(ui.painter(), r, CornerRadius::same(14), 0.85 + 0.15 * h);
        widgets::soft_glow(
            ui.painter(),
            r,
            pos2(r.right() - r.width() * 0.2, r.top()),
            r.width() * 0.5,
            theme::white(0x30),
        );
        ui.painter().text(
            pos2(r.left() + 24.0, r.top() + 22.0),
            Align2::LEFT_TOP,
            trf!("Ton année {0} en musique", year),
            theme::semibold(24.0),
            Color32::WHITE,
        );
        ui.painter().text(
            pos2(r.left() + 24.0, r.top() + 58.0),
            Align2::LEFT_TOP,
            tr("Tes morceaux, tes artistes, tes habitudes : le récap"),
            FontId::proportional(14.0),
            theme::white(0xD0),
        );
        icons_chevron(ui, r, h);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.actions.push(Action::Go(View::Recap(year)));
        }
        if !others.is_empty() {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Autres années :")).small().color(theme::TEXT_3));
                for &y in others {
                    if widgets::link(ui, &y.to_string()).clicked() {
                        self.actions.push(Action::Go(View::Recap(y)));
                    }
                }
            });
        }
    }

    /// Liste des morceaux les plus écoutés : rang, pochette, titre, nombre d'écoutes.
    fn top_tracks(&mut self, ui: &mut egui::Ui, tracks: &[(usize, u64)], n: usize, w: f32) {
        let max = tracks.first().map(|t| t.1).unwrap_or(1).max(1);
        for (k, &(i, count)) in tracks.iter().take(n).enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 48.0), Sense::click());
            widgets::paint_interaction(ui, r, 6, &resp);
            let t = self.lib.tracks[i].clone();
            ui.painter().text(
                pos2(r.left() + 18.0, r.center().y),
                Align2::CENTER_CENTER,
                (k + 1).to_string(),
                theme::semibold(14.0),
                if k < 3 {
                    theme::accent_light()
                } else {
                    theme::TEXT_3
                },
            );
            let cr = Rect::from_min_size(pos2(r.left() + 36.0, r.top() + 6.0), vec2(36.0, 36.0));
            let tex = self.covers.thumb(&t.path).cloned();
            widgets::cover(ui.painter(), cr, tex.as_ref(), 4, &t.album);
            let tw = r.width() - 36.0 - 48.0 - 90.0;
            let g = widgets::galley(ui, &t.title, FontId::proportional(14.0), theme::TEXT, tw);
            ui.painter()
                .galley(pos2(cr.right() + 12.0, r.top() + 6.0), g, theme::TEXT);
            let g = widgets::galley(
                ui,
                t.artist_or_unknown(),
                FontId::proportional(12.5),
                theme::TEXT_2,
                tw,
            );
            ui.painter()
                .galley(pos2(cr.right() + 12.0, r.top() + 26.0), g, theme::TEXT_2);
            // Petite barre proportionnelle aux écoutes, sous le nombre.
            let bar = Rect::from_min_size(pos2(r.right() - 82.0, r.center().y + 9.0), vec2(70.0, 3.0));
            ui.painter()
                .rect_filled(bar, CornerRadius::same(2), theme::white(0x14));
            let fill = Rect::from_min_size(bar.min, vec2(70.0 * count as f32 / max as f32, 3.0));
            theme::brand_fill(ui.painter(), fill, 2, 1.0);
            ui.painter().text(
                pos2(r.right() - 12.0, r.center().y - 6.0),
                Align2::RIGHT_CENTER,
                plural(count as usize, "écoute", "écoutes"),
                FontId::proportional(12.5),
                theme::TEXT_2,
            );
            if resp.double_clicked() {
                let list: Vec<usize> = tracks.iter().take(n).map(|t| t.0).collect();
                self.actions.push(Action::Play {
                    tracks: list,
                    start: k,
                });
            }
            resp.on_hover_text(tr("Double-clic pour écouter"));
        }
    }

    // ---------- Récap ----------

    pub(super) fn recap_page(&mut self, ui: &mut egui::Ui, year: i32) {
        let recap = recap(&self.lib, &self.listens, year);
        heading(ui, &trf!("{0}, en musique", year));
        ui.add_space(16.0);
        if recap.stats.listens == 0 {
            widgets::empty_state(
                ui,
                Icon::Sparkles,
                tr("Pas encore d'écoutes cette année-là"),
                tr("Écoute de la musique : ton récap se remplira tout seul."),
            );
            return;
        }
        let s = &recap.stats;
        egui::ScrollArea::vertical()
            .id_salt(("recap", year))
            .auto_shrink(false)
            .show(ui, |ui| {
                let w = (ui.available_width() - 16.0).min(980.0);
                ui.set_width(w);
                let mut k = 0;
                let mut card = |ui: &mut egui::Ui, h: f32| -> (Rect, f32) {
                    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                    let t = super::lists::stagger(ui.ctx(), k);
                    k += 1;
                    let r = r.translate(vec2(0.0, 24.0 * (1.0 - t)));
                    ui.add_space(14.0);
                    (r, t)
                };

                // 1. Le temps passé à écouter.
                let (r, t) = card(ui, 150.0);
                theme::brand_fill(ui.painter(), r, CornerRadius::same(16), t);
                let minutes = (s.seconds / 60.0).round() as u64;
                let shown =
                    (minutes as f32 * anim::appear(ui.ctx(), egui::Id::new(("recap_min", year)), 1.6)) as u64;
                ui.painter().text(
                    pos2(r.left() + 28.0, r.top() + 24.0),
                    Align2::LEFT_TOP,
                    tr("Tu as écouté"),
                    FontId::proportional(16.0),
                    theme::white((0xD0 as f32 * t) as u8),
                );
                ui.painter().text(
                    pos2(r.left() + 28.0, r.top() + 48.0),
                    Align2::LEFT_TOP,
                    trf!("{0} minutes", grouped(shown)),
                    theme::semibold(44.0),
                    Color32::WHITE.gamma_multiply(t),
                );
                ui.painter().text(
                    pos2(r.left() + 28.0, r.top() + 108.0),
                    Align2::LEFT_TOP,
                    trf!(
                        "{0} · {1} · {2}",
                        plural(s.listens as usize, "écoute", "écoutes"),
                        plural(s.tracks.len(), "morceau différent", "morceaux différents"),
                        plural(s.artists.len(), "artiste", "artistes")
                    ),
                    FontId::proportional(14.0),
                    theme::white((0xD0 as f32 * t) as u8),
                );

                // 2. Le morceau de l'année.
                if let Some(&(top, count)) = s.tracks.first() {
                    let (r, t) = card(ui, 200.0);
                    self.recap_panel(ui, r, t);
                    let tr_ = self.lib.tracks[top].clone();
                    let art = Rect::from_min_size(r.min + vec2(24.0, 24.0), vec2(152.0, 152.0));
                    let tex = self.covers.thumb(&tr_.path).cloned();
                    widgets::cover(ui.painter(), art, tex.as_ref(), 10, &tr_.album);
                    let x = art.right() + 28.0;
                    let tw = r.right() - x - 24.0;
                    let p = ui.painter();
                    p.text(
                        pos2(x, art.top() + 8.0),
                        Align2::LEFT_TOP,
                        tr("TON MORCEAU DE L'ANNÉE"),
                        FontId::proportional(12.0),
                        theme::accent_light().gamma_multiply(t),
                    );
                    let g = widgets::galley(ui, &tr_.title, theme::semibold(30.0), theme::TEXT, tw);
                    ui.painter()
                        .galley(pos2(x, art.top() + 30.0), g, theme::TEXT.gamma_multiply(t));
                    let g = widgets::galley(
                        ui,
                        tr_.artist_or_unknown(),
                        FontId::proportional(16.0),
                        theme::TEXT_2,
                        tw,
                    );
                    ui.painter()
                        .galley(pos2(x, art.top() + 74.0), g, theme::TEXT_2.gamma_multiply(t));
                    ui.painter().text(
                        pos2(x, art.top() + 112.0),
                        Align2::LEFT_TOP,
                        trf!("{0} cette année", plural(count as usize, "écoute", "écoutes")),
                        FontId::proportional(14.0),
                        theme::accent_light().gamma_multiply(t),
                    );
                }

                // 3. Artistes et morceaux préférés, côte à côte.
                let (r, t) = card(ui, 300.0);
                let half = (w - 14.0) / 2.0;
                let left = Rect::from_min_size(r.min, vec2(half, r.height()));
                let right =
                    Rect::from_min_size(pos2(r.left() + half + 14.0, r.top()), vec2(half, r.height()));
                for (rect, title, rows) in [
                    (
                        left,
                        tr("Tes artistes préférés"),
                        s.artists
                            .iter()
                            .take(5)
                            .map(|(a, n)| (a.clone(), plural(*n as usize, "écoute", "écoutes")))
                            .collect::<Vec<_>>(),
                    ),
                    (
                        right,
                        tr("Tes morceaux préférés"),
                        s.tracks
                            .iter()
                            .take(5)
                            .map(|&(i, n)| {
                                let t = &self.lib.tracks[i];
                                (
                                    format!("{} — {}", t.title, t.artist_or_unknown()),
                                    plural(n as usize, "écoute", "écoutes"),
                                )
                            })
                            .collect(),
                    ),
                ] {
                    self.recap_panel(ui, rect, t);
                    ui.painter().text(
                        pos2(rect.left() + 22.0, rect.top() + 20.0),
                        Align2::LEFT_TOP,
                        title,
                        theme::semibold(17.0),
                        theme::TEXT.gamma_multiply(t),
                    );
                    for (n, (name, count)) in rows.iter().enumerate() {
                        let y = rect.top() + 62.0 + n as f32 * 44.0;
                        ui.painter().text(
                            pos2(rect.left() + 34.0, y + 10.0),
                            Align2::CENTER_CENTER,
                            (n + 1).to_string(),
                            theme::semibold(22.0),
                            theme::accent_light().gamma_multiply(t),
                        );
                        let g = widgets::galley(
                            ui,
                            name,
                            FontId::proportional(15.0),
                            theme::TEXT,
                            rect.width() - 160.0,
                        );
                        ui.painter()
                            .galley(pos2(rect.left() + 60.0, y), g, theme::TEXT.gamma_multiply(t));
                        ui.painter().text(
                            pos2(rect.right() - 22.0, y + 10.0),
                            Align2::RIGHT_CENTER,
                            count,
                            FontId::proportional(12.5),
                            theme::TEXT_3.gamma_multiply(t),
                        );
                    }
                }

                // 4. Habitudes : genre, jour, mois, découvertes.
                let (r, t) = card(ui, 130.0);
                let facts: Vec<(String, &str)> = [
                    s.genres.first().map(|g| (g.0.clone(), tr("ton genre n°1"))),
                    recap
                        .weekday
                        .map(|d| (weekday_name(d).to_owned(), tr("ton jour le plus musical"))),
                    recap
                        .month
                        .map(|m| (month_name(m).to_owned(), tr("ton mois le plus musical"))),
                    Some((grouped(recap.discoveries as u64), tr("morceaux découverts"))),
                ]
                .into_iter()
                .flatten()
                .collect();
                let fw = (w - 14.0 * (facts.len() as f32 - 1.0)) / facts.len().max(1) as f32;
                for (n, (value, label)) in facts.iter().enumerate() {
                    let fr = Rect::from_min_size(
                        pos2(r.left() + n as f32 * (fw + 14.0), r.top()),
                        vec2(fw, r.height()),
                    );
                    self.recap_panel(ui, fr, t);
                    let g =
                        widgets::galley(ui, value, theme::semibold(24.0), theme::accent_light(), fw - 36.0);
                    ui.painter().galley(
                        pos2(fr.left() + 18.0, fr.top() + 28.0),
                        g,
                        theme::accent_light().gamma_multiply(t),
                    );
                    let g = widgets::galley(ui, label, FontId::proportional(13.0), theme::TEXT_2, fw - 36.0);
                    ui.painter().galley(
                        pos2(fr.left() + 18.0, fr.top() + 76.0),
                        g,
                        theme::TEXT_2.gamma_multiply(t),
                    );
                }

                // 5. Rejouer l'année.
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let top: Vec<usize> = s.tracks.iter().take(50).map(|t| t.0).collect();
                    if widgets::primary_button(ui, Some(Icon::Play), tr("Écouter tes 50 morceaux de l'année"))
                        .clicked()
                    {
                        self.actions.push(Action::Play {
                            tracks: top,
                            start: 0,
                        });
                    }
                    if let Some(first) = recap.first_day {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(trf!("Historique daté depuis le {0}.", date_text(first)))
                                .small()
                                .color(theme::TEXT_3),
                        );
                    }
                });
                ui.add_space(24.0);
            });
    }

    /// Fond d'une carte du récap.
    fn recap_panel(&self, ui: &egui::Ui, r: Rect, t: f32) {
        ui.painter().rect(
            r,
            CornerRadius::same(16),
            theme::white((0x0B as f32 * t) as u8),
            egui::Stroke::new(1.0_f32, theme::white((0x12 as f32 * t) as u8)),
            egui::StrokeKind::Inside,
        );
    }
}

/// Flèche « ouvrir » du bandeau du récap.
fn icons_chevron(ui: &egui::Ui, r: Rect, h: f32) {
    crate::icons::paint_centered(
        ui.painter(),
        Icon::ChevronRight,
        pos2(r.right() - 32.0 + 4.0 * h, r.center().y),
        24.0,
        Color32::WHITE,
    );
}

/// Barres horizontales : nom, nombre d'écoutes, longueur proportionnelle.
fn bars(ui: &mut egui::Ui, rows: &[(String, u64)], n: usize, w: f32, id: &str) {
    let max = rows.first().map(|r| r.1).unwrap_or(1).max(1);
    for (k, (name, count)) in rows.iter().take(n).enumerate() {
        let (r, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
        let t = anim::appear(ui.ctx(), egui::Id::new((id, k)), anim::SLOW);
        let full = r.width() - 110.0;
        let bar = Rect::from_min_size(
            pos2(r.left(), r.top() + 6.0),
            vec2((full * *count as f32 / max as f32 * t).max(4.0), 22.0),
        );
        ui.painter()
            .rect_filled(bar, CornerRadius::same(5), theme::alpha(theme::accent(), 0x40));
        let g = widgets::galley(ui, name, FontId::proportional(13.5), theme::TEXT, full - 16.0);
        ui.painter().galley(
            pos2(r.left() + 10.0, bar.center().y - g.size().y / 2.0),
            g,
            theme::TEXT,
        );
        ui.painter().text(
            pos2(r.right() - 8.0, bar.center().y),
            Align2::RIGHT_CENTER,
            plural(*count as usize, "écoute", "écoutes"),
            FontId::proportional(12.5),
            theme::TEXT_2,
        );
    }
}

/// Une cellule de CSV : entre guillemets si besoin (séparateur, guillemet, retour à la ligne).
fn cell(text: &str, sep: char) -> String {
    if text.contains(sep) || text.contains('"') || text.contains('\n') {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// Date et heure locales d'un horodatage : « 2026-11-10 », « 14:05 ».
fn date_time(time: u64) -> (String, String) {
    let (y, m, d) = history::civil(history::local_day(time));
    let secs = (time as i64 + history::local_offset()).rem_euclid(86_400);
    (
        format!("{y:04}-{m:02}-{d:02}"),
        format!("{:02}:{:02}", secs / 3600, secs / 60 % 60),
    )
}

impl SterMusic {
    /// Exporte les statistiques en CSV : une ligne par morceau (`per_track`), ou une ligne par
    /// écoute de l'historique. Le séparateur suit la langue (« ; » pour un tableur français).
    pub(super) fn export_stats(&mut self, per_track: bool) {
        let sep = match crate::i18n::lang() {
            crate::i18n::Lang::Fr => ';',
            crate::i18n::Lang::En => ',',
        };
        let name = if per_track {
            tr("statistiques-morceaux.csv")
        } else {
            tr("historique-ecoutes.csv")
        };
        let Some(file) = rfd::FileDialog::new()
            .set_title(tr("Exporter les statistiques"))
            .set_file_name(name)
            .add_filter("CSV", &["csv"])
            .save_file()
        else {
            return;
        };
        let row = |cells: &[String]| -> String {
            let mut line = cells
                .iter()
                .map(|c| cell(c, sep))
                .collect::<Vec<_>>()
                .join(&sep.to_string());
            line.push('\n');
            line
        };
        let info = |path: &std::path::Path| -> (String, String, String) {
            match self.lib.find(path).map(|i| &self.lib.tracks[i]) {
                Some(t) => (t.title.clone(), t.artist.clone(), t.album.clone()),
                None => (
                    path.file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    String::new(),
                    String::new(),
                ),
            }
        };
        // Marque d'ordre des octets : LibreOffice et Excel reconnaissent alors l'UTF-8 (accents).
        let mut out = String::from("\u{feff}");
        let mut rows = 0;
        if per_track {
            out += &row(&[
                tr("Titre").into(),
                tr("Artiste").into(),
                tr("Album").into(),
                tr("Écoutes").into(),
                tr("Minutes écoutées").into(),
                tr("Dernière écoute").into(),
                tr("Fichier").into(),
            ]);
            let mut plays: Vec<(&std::path::PathBuf, &super::PlayStat)> =
                self.settings.plays.iter().filter(|(_, s)| s.count > 0).collect();
            plays.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| a.0.cmp(b.0)));
            for (path, s) in plays {
                let (title, artist, album) = info(path);
                let last = if s.last > 0 {
                    let (d, h) = date_time(s.last);
                    format!("{d} {h}")
                } else {
                    String::new()
                };
                out += &row(&[
                    title,
                    artist,
                    album,
                    s.count.to_string(),
                    format!("{:.1}", s.seconds / 60.0),
                    last,
                    path.display().to_string(),
                ]);
                rows += 1;
            }
        } else {
            out += &row(&[
                tr("Date").into(),
                tr("Heure").into(),
                tr("Titre").into(),
                tr("Artiste").into(),
                tr("Album").into(),
                tr("Secondes écoutées").into(),
                tr("Fichier").into(),
            ]);
            for l in &self.listens {
                let (title, artist, album) = info(&l.path);
                let (d, h) = date_time(l.time);
                let secs = if l.seconds > 0 {
                    l.seconds.to_string()
                } else {
                    String::new()
                };
                out += &row(&[d, h, title, artist, album, secs, l.path.display().to_string()]);
                rows += 1;
            }
        }
        match std::fs::write(&file, out) {
            Ok(()) => self.toast(trf!(
                "{0} exportées : {1}",
                plural(rows, "ligne", "lignes"),
                file.display()
            )),
            Err(e) => self.toast(trf!("Export impossible : {e}", e)),
        }
    }

    /// Efface toutes les statistiques : compteurs d'écoutes et historique daté.
    pub(super) fn clear_stats(&mut self) {
        self.settings.plays.clear();
        self.listens.clear();
        self.listening = None;
        crate::history::save(&[]);
        self.smart_cache.borrow_mut().clear();
        self.toast(tr("Statistiques effacées"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Track;
    use std::path::PathBuf;

    fn track(path: &str, artist: &str, genre: &str) -> Track {
        Track {
            path: PathBuf::from(path),
            title: path.into(),
            artist: artist.into(),
            album: "Album".into(),
            album_artist: String::new(),
            genre: genre.into(),
            year: None,
            track_no: None,
            disc_no: None,
            duration_ms: 180_000,
            mtime: 0,
            composer: String::new(),
            schema: 0,
            cue: None,
            rating: None,
            remote: None,
        }
    }

    #[test]
    fn year_recap() {
        let lib = Library::new(vec![
            track("/a", "X", "Rock"),
            track("/b", "Y", "Pop"),
            track("/c", "X", "Rock"),
        ]);
        // 2025 puis 2026 (midi, pour ne pas dépendre du fuseau horaire).
        let day = |d: i64| (d * 86_400 + 12 * 3600) as u64;
        let l = |d: i64, p: &str| Listen {
            time: day(d),
            path: PathBuf::from(p),
            seconds: 0,
        };
        let all = vec![
            l(20_300, "/a"),
            l(20_460, "/a"),
            l(20_461, "/a"),
            l(20_462, "/b"),
            l(20_463, "/c"),
        ];
        let r = recap(&lib, &all, 2026);
        assert_eq!(r.stats.listens, 4);
        assert_eq!(lib.tracks[r.stats.tracks[0].0].path, PathBuf::from("/a"));
        assert_eq!(r.stats.artists[0], ("X".to_owned(), 3));
        assert_eq!(r.stats.genres[0], ("Rock".to_owned(), 3));
        // « /a » était déjà écouté en 2025 : seuls /b et /c sont des découvertes.
        assert_eq!(r.discoveries, 2);
        // Ex æquo : toujours le même ordre (alphabétique), d'un calcul à l'autre.
        for _ in 0..20 {
            let again = recap(&lib, &all, 2026);
            assert_eq!(again.stats.artists, r.stats.artists);
            assert_eq!(again.stats.genres, r.stats.genres);
        }
        assert!((r.stats.seconds - 4.0 * 180.0).abs() < 1e-6);
    }
}
