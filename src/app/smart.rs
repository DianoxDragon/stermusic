//! Playlists intelligentes : elles se remplissent toutes seules d'après des règles (« genre
//! contient Rock », « année inférieure à 2010 », « écoutes égales à 0 »…), et suivent la
//! bibliothèque et les écoutes. Quatre sont proposées d'office (modifiables, supprimables).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use eframe::egui::{self, Align, Layout, RichText};
use serde::{Deserialize, Serialize};

use super::views::segment;
use super::{now_secs, PlayStat, SterMusic};
use crate::i18n::{n_, tr};
use crate::icons::Icon;
use crate::library::{fold, plural, Library};
use crate::{theme, trf, widgets};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Field {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Year,
    /// Nombre d'écoutes.
    Plays,
    /// Jours depuis la dernière écoute (jamais écouté : infini).
    LastPlayed,
    /// Jours depuis l'ajout à la bibliothèque (date du fichier).
    Added,
    /// Durée, en minutes.
    Duration,
    Favorite,
    /// Note, de 0 (aucune) à 5 étoiles.
    Rating,
}

pub const FIELDS: [Field; 12] = [
    Field::Title,
    Field::Artist,
    Field::Album,
    Field::AlbumArtist,
    Field::Genre,
    Field::Year,
    Field::Plays,
    Field::LastPlayed,
    Field::Added,
    Field::Duration,
    Field::Favorite,
    Field::Rating,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Number,
    Flag,
}

impl Field {
    pub fn label(self) -> &'static str {
        tr(match self {
            Field::Title => n_("Titre"),
            Field::Artist => n_("Artiste"),
            Field::Album => n_("Album"),
            Field::AlbumArtist => n_("Artiste de l'album"),
            Field::Genre => n_("Genre"),
            Field::Year => n_("Année"),
            Field::Plays => n_("Nombre d'écoutes"),
            Field::LastPlayed => n_("Dernière écoute (jours)"),
            Field::Added => n_("Ajouté il y a (jours)"),
            Field::Duration => n_("Durée (minutes)"),
            Field::Favorite => n_("Favori"),
            Field::Rating => n_("Note (étoiles)"),
        })
    }

    fn kind(self) -> Kind {
        match self {
            Field::Title | Field::Artist | Field::Album | Field::AlbumArtist | Field::Genre => Kind::Text,
            Field::Favorite => Kind::Flag,
            _ => Kind::Number,
        }
    }

    fn ops(self) -> &'static [Op] {
        match self.kind() {
            Kind::Text => &[Op::Contains, Op::NotContains, Op::Is, Op::IsNot],
            Kind::Number => &[Op::Is, Op::IsNot, Op::Less, Op::More],
            Kind::Flag => &[Op::Is, Op::IsNot],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Contains,
    NotContains,
    Is,
    IsNot,
    Less,
    More,
}

impl Op {
    fn label(self, kind: Kind) -> &'static str {
        tr(match (self, kind) {
            (Op::Contains, _) => n_("contient"),
            (Op::NotContains, _) => n_("ne contient pas"),
            (Op::Is, Kind::Flag) => n_("oui"),
            (Op::IsNot, Kind::Flag) => n_("non"),
            (Op::Is, _) => n_("est"),
            (Op::IsNot, _) => n_("n'est pas"),
            (Op::Less, _) => n_("est inférieur à"),
            (Op::More, _) => n_("est supérieur à"),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub field: Field,
    pub op: Op,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    /// Ordre de la bibliothèque (artiste, album, piste).
    Library,
    MostPlayed,
    LeastPlayed,
    RecentlyPlayed,
    /// Écoutés il y a le plus longtemps (jamais écoutés d'abord).
    OldestPlayed,
    RecentlyAdded,
    Title,
}

pub const SORTS: [Sort; 7] = [
    Sort::Library,
    Sort::MostPlayed,
    Sort::LeastPlayed,
    Sort::RecentlyPlayed,
    Sort::OldestPlayed,
    Sort::RecentlyAdded,
    Sort::Title,
];

impl Sort {
    fn label(self) -> &'static str {
        tr(match self {
            Sort::Library => n_("Ordre de la bibliothèque"),
            Sort::MostPlayed => n_("Les plus écoutés d'abord"),
            Sort::LeastPlayed => n_("Les moins écoutés d'abord"),
            Sort::RecentlyPlayed => n_("Écoutés récemment d'abord"),
            Sort::OldestPlayed => n_("Pas écoutés depuis longtemps d'abord"),
            Sort::RecentlyAdded => n_("Ajoutés récemment d'abord"),
            Sort::Title => n_("Titre"),
        })
    }
}

/// Icône d'une playlist intelligente (barre latérale).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SmartIcon {
    #[default]
    Sparkles,
    Flame,
    EyeOff,
    History,
    CalendarPlus,
    Star,
    Zap,
    Moon,
    Sun,
    Coffee,
}

impl SmartIcon {
    pub const ALL: [SmartIcon; 10] = [
        SmartIcon::Sparkles,
        SmartIcon::Flame,
        SmartIcon::EyeOff,
        SmartIcon::History,
        SmartIcon::CalendarPlus,
        SmartIcon::Star,
        SmartIcon::Zap,
        SmartIcon::Moon,
        SmartIcon::Sun,
        SmartIcon::Coffee,
    ];

    pub fn icon(self) -> Icon {
        match self {
            SmartIcon::Sparkles => Icon::Sparkles,
            SmartIcon::Flame => Icon::Flame,
            SmartIcon::EyeOff => Icon::EyeOff,
            SmartIcon::History => Icon::History,
            SmartIcon::CalendarPlus => Icon::CalendarPlus,
            SmartIcon::Star => Icon::Star,
            SmartIcon::Zap => Icon::Zap,
            SmartIcon::Moon => Icon::Moon,
            SmartIcon::Sun => Icon::Sun,
            SmartIcon::Coffee => Icon::Coffee,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmartPlaylist {
    pub name: String,
    #[serde(default)]
    pub icon: SmartIcon,
    pub rules: Vec<Rule>,
    /// Toutes les règles (sinon : au moins une).
    pub match_all: bool,
    pub sort: Sort,
    /// Nombre maximal de morceaux (0 : pas de limite).
    pub limit: usize,
    /// Épinglée dans la barre latérale.
    #[serde(default)]
    pub pinned: bool,
}

impl SmartPlaylist {
    /// Les quatre playlists proposées d'office, dans la langue de l'interface au moment de leur
    /// création.
    pub fn presets() -> Vec<SmartPlaylist> {
        let rule = |field, op, value: &str| Rule {
            field,
            op,
            value: value.to_owned(),
        };
        vec![
            SmartPlaylist {
                pinned: false,
                name: tr("Les plus écoutés").to_owned(),
                icon: SmartIcon::Flame,
                rules: vec![rule(Field::Plays, Op::More, "0")],
                match_all: true,
                sort: Sort::MostPlayed,
                limit: 50,
            },
            SmartPlaylist {
                pinned: false,
                name: tr("Jamais écoutés").to_owned(),
                icon: SmartIcon::EyeOff,
                rules: vec![rule(Field::Plays, Op::Is, "0")],
                match_all: true,
                sort: Sort::RecentlyAdded,
                limit: 0,
            },
            SmartPlaylist {
                pinned: false,
                name: tr("Redécouvertes").to_owned(),
                icon: SmartIcon::History,
                rules: vec![
                    rule(Field::Favorite, Op::Is, ""),
                    rule(Field::LastPlayed, Op::More, "180"),
                ],
                match_all: true,
                sort: Sort::OldestPlayed,
                limit: 0,
            },
            SmartPlaylist {
                pinned: false,
                name: tr("Ajoutés ce mois-ci").to_owned(),
                icon: SmartIcon::CalendarPlus,
                rules: vec![rule(Field::Added, Op::Less, "31")],
                match_all: true,
                sort: Sort::RecentlyAdded,
                limit: 0,
            },
        ]
    }

    /// Playlists enregistrées avant l'arrivée des icônes : celles proposées d'office (mêmes
    /// règles) reprennent leur icône.
    pub fn guess_icon(&mut self) {
        if self.icon != SmartIcon::Sparkles {
            return;
        }
        if let Some(p) = Self::presets()
            .into_iter()
            .find(|p| p.rules == self.rules && p.sort == self.sort && p.limit == self.limit)
        {
            self.icon = p.icon;
        }
    }

    fn new_empty() -> Self {
        SmartPlaylist {
            pinned: false,
            name: tr("Nouvelle playlist intelligente").to_owned(),
            icon: SmartIcon::Star,
            rules: vec![Rule {
                field: Field::Genre,
                op: Op::Contains,
                value: String::new(),
            }],
            match_all: true,
            sort: Sort::Library,
            limit: 0,
        }
    }
}

/// Ce dont l'évaluation a besoin, en plus de la bibliothèque.
pub struct Facts<'a> {
    pub plays: &'a HashMap<PathBuf, PlayStat>,
    pub favorites: &'a HashSet<PathBuf>,
    pub ratings: &'a HashMap<PathBuf, u8>,
    pub now: u64,
}

fn number(value: &str) -> Option<f64> {
    value.trim().replace(',', ".").parse().ok()
}

fn matches(rule: &Rule, lib: &Library, i: usize, f: &Facts) -> bool {
    let t = &lib.tracks[i];
    let stat = f.plays.get(&t.path).copied().unwrap_or_default();
    let days = |since: u64| -> f64 {
        if since == 0 {
            f64::INFINITY
        } else {
            f.now.saturating_sub(since) as f64 / 86_400.0
        }
    };
    match rule.field.kind() {
        Kind::Flag => {
            let fav = f.favorites.contains(&t.path);
            (rule.op == Op::Is) == fav
        }
        Kind::Text => {
            let text = match rule.field {
                Field::Title => t.title.as_str(),
                Field::Artist => t.artist_or_unknown(),
                Field::Album => t.album_or_unknown(),
                Field::AlbumArtist => t.album_artist_or_artist(),
                _ => t.genre.as_str(),
            };
            let (text, value) = (fold(text), fold(rule.value.trim()));
            match rule.op {
                Op::Contains => text.contains(&value),
                Op::NotContains => !text.contains(&value),
                Op::Is => text == value,
                Op::IsNot => text != value,
                _ => false,
            }
        }
        Kind::Number => {
            let x = match rule.field {
                Field::Year => match t.year {
                    Some(y) => y as f64,
                    // Sans année, aucune comparaison ne peut être vraie (sauf « n'est pas »).
                    None => return rule.op == Op::IsNot,
                },
                Field::Plays => stat.count as f64,
                Field::LastPlayed => days(stat.last),
                Field::Added => days(t.mtime),
                Field::Rating => f.ratings.get(&t.path).copied().unwrap_or(0) as f64,
                _ => t.duration_ms as f64 / 60_000.0,
            };
            let Some(v) = number(&rule.value) else {
                return true;
            };
            match rule.op {
                // Les jours et les minutes sont comparés à l'unité près.
                Op::Is => x.floor() == v.floor() || x == v,
                Op::IsNot => !(x.floor() == v.floor() || x == v),
                Op::Less => x < v,
                Op::More => x > v,
                _ => false,
            }
        }
    }
}

/// Les morceaux de la playlist, triés et limités.
pub fn evaluate(p: &SmartPlaylist, lib: &Library, f: &Facts) -> Vec<usize> {
    let mut out: Vec<usize> = (0..lib.tracks.len())
        .filter(|&i| {
            if p.rules.is_empty() {
                return true;
            }
            if p.match_all {
                p.rules.iter().all(|r| matches(r, lib, i, f))
            } else {
                p.rules.iter().any(|r| matches(r, lib, i, f))
            }
        })
        .collect();
    let stat = |i: usize| f.plays.get(&lib.tracks[i].path).copied().unwrap_or_default();
    match p.sort {
        Sort::Library => {}
        Sort::MostPlayed => {
            out.sort_by_key(|&i| (std::cmp::Reverse(stat(i).count), std::cmp::Reverse(stat(i).last)))
        }
        Sort::LeastPlayed => out.sort_by_key(|&i| (stat(i).count, stat(i).last)),
        Sort::RecentlyPlayed => out.sort_by_key(|&i| std::cmp::Reverse(stat(i).last)),
        Sort::OldestPlayed => out.sort_by_key(|&i| stat(i).last),
        Sort::RecentlyAdded => out.sort_by_key(|&i| std::cmp::Reverse(lib.tracks[i].mtime)),
        Sort::Title => out.sort_by_cached_key(|&i| fold(&lib.tracks[i].title)),
    }
    if p.limit > 0 {
        out.truncate(p.limit);
    }
    out
}

/// Playlist intelligente en cours de création ou de modification.
pub struct Edit {
    /// `None` : nouvelle playlist.
    pub index: Option<usize>,
    pub draft: SmartPlaylist,
    limit_text: String,
}

impl Edit {
    pub fn new(index: Option<usize>, draft: Option<SmartPlaylist>) -> Self {
        let draft = draft.unwrap_or_else(SmartPlaylist::new_empty);
        let limit_text = if draft.limit > 0 {
            draft.limit.to_string()
        } else {
            String::new()
        };
        Self {
            index,
            draft,
            limit_text,
        }
    }
}

impl SterMusic {
    pub(super) fn smart_facts(&self) -> Facts<'_> {
        Facts {
            plays: &self.settings.plays,
            favorites: &self.settings.favorites,
            ratings: &self.settings.ratings,
            now: now_secs(),
        }
    }

    /// Fenêtre de création ou de modification d'une playlist intelligente.
    pub(super) fn smart_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut edit) = self.smart_edit.take() else {
            return;
        };
        let count = evaluate(&edit.draft, &self.lib, &self.smart_facts()).len();
        let (mut close, mut save, mut delete) = (false, false, false);
        let modal = egui::Modal::new(egui::Id::new("smart_playlist"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .inner_margin(24.0)
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                let width = 620.0;
                ui.set_width(width);
                super::lib_tools::title(
                    ui,
                    Icon::Sparkles,
                    if edit.index.is_some() {
                        tr("Modifier la playlist intelligente")
                    } else {
                        tr("Nouvelle playlist intelligente")
                    },
                    "",
                );
                ui.add_space(14.0);
                widgets::text_field(
                    ui,
                    egui::Id::new("smart_name"),
                    &mut edit.draft.name,
                    tr("Nom de la playlist"),
                    width,
                    1,
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(RichText::new(tr("Icône :")).color(theme::TEXT_2));
                    ui.add_space(4.0);
                    for icon in SmartIcon::ALL {
                        let on = edit.draft.icon == icon;
                        let resp = widgets::toggle_icon(ui, on, icon.icon(), "", 32.0, 17.0);
                        if resp.clicked() {
                            edit.draft.icon = icon;
                        }
                    }
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new(tr("Morceaux qui respectent")).color(theme::TEXT_2));
                    if segment(ui, tr("toutes les règles"), edit.draft.match_all).clicked() {
                        edit.draft.match_all = true;
                    }
                    if segment(ui, tr("au moins une règle"), !edit.draft.match_all).clicked() {
                        edit.draft.match_all = false;
                    }
                });
                ui.add_space(10.0);
                let mut remove = None;
                for (k, rule) in edit.draft.rules.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        egui::ComboBox::from_id_salt(("smart_field", k))
                            .selected_text(rule.field.label())
                            .width(190.0)
                            .show_ui(ui, |ui| {
                                for f in FIELDS {
                                    if ui.selectable_label(rule.field == f, f.label()).clicked()
                                        && rule.field != f
                                    {
                                        rule.field = f;
                                        // Une opération qui n'existe pas pour ce champ revient à la première.
                                        if !f.ops().contains(&rule.op) {
                                            rule.op = f.ops()[0];
                                        }
                                    }
                                }
                            });
                        let kind = rule.field.kind();
                        egui::ComboBox::from_id_salt(("smart_op", k))
                            .selected_text(rule.op.label(kind))
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                for &op in rule.field.ops() {
                                    ui.selectable_value(&mut rule.op, op, op.label(kind));
                                }
                            });
                        if kind != Kind::Flag {
                            let hint = if kind == Kind::Number { "0" } else { "" };
                            widgets::text_field(
                                ui,
                                egui::Id::new(("smart_value", k)),
                                &mut rule.value,
                                hint,
                                200.0,
                                1,
                            );
                            if kind == Kind::Number {
                                rule.value.retain(|c| c.is_ascii_digit() || c == '.' || c == ',');
                            }
                        }
                        if widgets::icon_button(ui, Icon::Close, tr("Retirer cette règle"), 30.0, 14.0)
                            .clicked()
                        {
                            remove = Some(k);
                        }
                    });
                    ui.add_space(4.0);
                }
                if let Some(k) = remove {
                    edit.draft.rules.remove(k);
                }
                if widgets::secondary_button(ui, Some(Icon::Plus), tr("Ajouter une règle")).clicked() {
                    edit.draft.rules.push(Rule {
                        field: Field::Genre,
                        op: Op::Contains,
                        value: String::new(),
                    });
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(tr("Trier :")).color(theme::TEXT_2));
                    egui::ComboBox::from_id_salt("smart_sort")
                        .selected_text(edit.draft.sort.label())
                        .width(240.0)
                        .show_ui(ui, |ui| {
                            for s in SORTS {
                                ui.selectable_value(&mut edit.draft.sort, s, s.label());
                            }
                        });
                    ui.add_space(12.0);
                    ui.label(RichText::new(tr("Au plus :")).color(theme::TEXT_2));
                    widgets::text_field(
                        ui,
                        egui::Id::new("smart_limit"),
                        &mut edit.limit_text,
                        tr("tous"),
                        70.0,
                        1,
                    );
                    edit.limit_text.retain(|c| c.is_ascii_digit());
                    edit.draft.limit = edit.limit_text.parse().unwrap_or(0);
                    ui.label(RichText::new(tr("morceaux")).color(theme::TEXT_2));
                });
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(trf!(
                            "{0} pour l'instant",
                            plural(count, "morceau correspond", "morceaux correspondent")
                        ))
                        .color(theme::accent_light()),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::primary_button(ui, None, tr("Enregistrer")).clicked() {
                            save = true;
                        }
                        if widgets::secondary_button(ui, None, tr("Annuler")).clicked() {
                            close = true;
                        }
                        if edit.index.is_some()
                            && widgets::icon_button(
                                ui,
                                Icon::Trash,
                                tr("Supprimer cette playlist"),
                                34.0,
                                16.0,
                            )
                            .clicked()
                        {
                            delete = true;
                        }
                    });
                });
                ui.add_space(2.0);
            });
        if save {
            if edit.draft.name.trim().is_empty() {
                edit.draft.name = tr("Playlist intelligente").to_owned();
            }
            match edit.index {
                Some(i) if i < self.settings.smart_playlists.len() => {
                    self.settings.smart_playlists[i] = edit.draft
                }
                _ => {
                    self.settings.smart_playlists.push(edit.draft);
                    let i = self.settings.smart_playlists.len() - 1;
                    self.actions.push(super::Action::Go(super::View::Playlist(
                        super::PlaylistId::Smart(i),
                    )));
                }
            }
            self.smart_cache.borrow_mut().clear();
        } else if delete {
            if let Some(i) = edit.index.filter(|&i| i < self.settings.smart_playlists.len()) {
                self.settings.smart_playlists.remove(i);
                self.smart_cache.borrow_mut().clear();
                if matches!(self.view, super::View::Playlist(super::PlaylistId::Smart(_))) {
                    self.view = super::View::Page(super::Page::Playlists);
                    self.history.clear();
                }
            }
        } else if !(close || modal.should_close()) {
            self.smart_edit = Some(edit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Track;

    fn track(path: &str, title: &str, genre: &str, year: Option<u32>, mtime: u64) -> Track {
        Track {
            path: PathBuf::from(path),
            title: title.into(),
            artist: "A".into(),
            album: "B".into(),
            album_artist: String::new(),
            genre: genre.into(),
            year,
            track_no: None,
            disc_no: None,
            duration_ms: 200_000,
            mtime,
            composer: String::new(),
            schema: 0,
            cue: None,
            rating: None,
            remote: None,
        }
    }

    #[test]
    fn rules() {
        let lib = Library::new(vec![
            track("/a", "Un", "Rock alternatif", Some(1995), 1_000),
            track("/b", "Deux", "Électro", Some(2015), 2_000),
            track("/c", "Trois", "rock", None, 3_000),
        ]);
        let now = 1_000 * 86_400;
        let mut plays = HashMap::new();
        plays.insert(
            PathBuf::from("/a"),
            PlayStat {
                count: 5,
                last: now - 200 * 86_400,
                seconds: 0.0,
            },
        );
        plays.insert(
            PathBuf::from("/b"),
            PlayStat {
                count: 2,
                last: now - 86_400,
                seconds: 0.0,
            },
        );
        let favorites: HashSet<PathBuf> = [PathBuf::from("/a"), PathBuf::from("/c")].into();
        let ratings = HashMap::new();
        let f = Facts {
            plays: &plays,
            favorites: &favorites,
            ratings: &ratings,
            now,
        };
        let titles = |p: &SmartPlaylist| -> Vec<String> {
            evaluate(p, &lib, &f)
                .iter()
                .map(|&i| lib.tracks[i].title.clone())
                .collect()
        };
        let mut p = SmartPlaylist {
            pinned: false,
            name: String::new(),
            icon: SmartIcon::default(),
            rules: vec![Rule {
                field: Field::Genre,
                op: Op::Contains,
                value: "ROCK".into(),
            }],
            match_all: true,
            sort: Sort::Title,
            limit: 0,
        };
        assert_eq!(titles(&p), ["Trois", "Un"]);
        p.rules.push(Rule {
            field: Field::Year,
            op: Op::Less,
            value: "2010".into(),
        });
        assert_eq!(titles(&p), ["Un"]);
        p.match_all = false;
        assert_eq!(titles(&p), ["Trois", "Un"]);
        // Les quatre playlists d'office.
        let presets = SmartPlaylist::presets();
        assert_eq!(titles(&presets[0]), ["Un", "Deux"]);
        assert_eq!(titles(&presets[1]), ["Trois"]);
        // Favoris pas écoutés depuis plus de 180 jours (jamais écouté compris).
        let mut rediscover = titles(&presets[2]);
        rediscover.sort();
        assert_eq!(rediscover, ["Trois", "Un"]);
        p.limit = 1;
        assert_eq!(titles(&p).len(), 1);
    }
}
