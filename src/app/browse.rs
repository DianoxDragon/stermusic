//! Parcourir la bibliothèque par genre, par année ou par compositeur.

use std::collections::HashMap;

use eframe::egui::{self, pos2, vec2, Color32, CornerRadius, FontId, Rect, RichText, Sense};
use serde::{Deserialize, Serialize};

use super::lists::{self, ListOpts};
use super::views::{segment, Header};
use super::{Action, SterMusic, View};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::library::{fmt_long, fold, plural, split_genres, Library};
use crate::{theme, widgets};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BrowseKind {
    #[default]
    Genre,
    Year,
    Composer,
}

impl BrowseKind {
    pub const ALL: [BrowseKind; 3] = [BrowseKind::Genre, BrowseKind::Year, BrowseKind::Composer];

    pub fn label(self) -> &'static str {
        match self {
            BrowseKind::Genre => tr("Genres"),
            BrowseKind::Year => tr("Années"),
            BrowseKind::Composer => tr("Compositeurs"),
        }
    }

    fn kind_label(self) -> &'static str {
        match self {
            BrowseKind::Genre => tr("GENRE"),
            BrowseKind::Year => tr("ANNÉE"),
            BrowseKind::Composer => tr("COMPOSITEUR"),
        }
    }

    fn icon(self) -> Icon {
        match self {
            BrowseKind::Genre => Icon::Shapes,
            BrowseKind::Year => Icon::Calendar,
            BrowseKind::Composer => Icon::Feather,
        }
    }
}

/// Un genre, une année ou un compositeur, avec ses morceaux et ses albums.
#[derive(Clone, Debug)]
pub struct Group {
    pub name: String,
    pub tracks: Vec<usize>,
    pub albums: Vec<usize>,
}

/// Les regroupements, recalculés quand la bibliothèque change.
#[derive(Default)]
pub struct BrowseIndex {
    pub genres: Vec<Group>,
    /// Années, de la plus récente à la plus ancienne.
    pub years: Vec<Group>,
    pub composers: Vec<Group>,
}

impl BrowseIndex {
    pub fn build(lib: &Library) -> Self {
        let mut genres = Grouper::default();
        let mut years = Grouper::default();
        let mut composers = Grouper::default();
        for (i, t) in lib.tracks.iter().enumerate() {
            let album = lib.track_album[i];
            for g in split_genres(&t.genre) {
                genres.add(g, i, album);
            }
            if let Some(y) = t.year.filter(|&y| y > 0) {
                years.add(&y.to_string(), i, album);
            }
            for c in t
                .composer
                .split([';', '/'])
                .map(str::trim)
                .filter(|c| !c.is_empty())
            {
                composers.add(c, i, album);
            }
        }
        let mut years = years.finish();
        years.sort_by(|a, b| b.name.cmp(&a.name));
        let mut genres = genres.finish();
        genres.sort_by_cached_key(|g| fold(&g.name));
        let mut composers = composers.finish();
        composers.sort_by_cached_key(|g| fold(&g.name));
        Self {
            genres,
            years,
            composers,
        }
    }

    pub fn groups(&self, kind: BrowseKind) -> &[Group] {
        match kind {
            BrowseKind::Genre => &self.genres,
            BrowseKind::Year => &self.years,
            BrowseKind::Composer => &self.composers,
        }
    }

    pub fn find(&self, kind: BrowseKind, name: &str) -> Option<&Group> {
        let key = fold(name);
        self.groups(kind).iter().find(|g| fold(&g.name) == key)
    }
}

/// Regroupe sans tenir compte de la casse ni des accents (« Électro » et « electro »), sous
/// l'orthographe la plus fréquente.
#[derive(Default)]
struct Grouper {
    groups: HashMap<String, Pile>,
}

/// Un regroupement en cours : ses orthographes (et combien de fois chacune), ses morceaux, ses albums.
type Pile = (HashMap<String, usize>, Vec<usize>, Vec<usize>);

impl Grouper {
    fn add(&mut self, name: &str, track: usize, album: usize) {
        let (spellings, tracks, albums) = self.groups.entry(fold(name)).or_default();
        *spellings.entry(name.to_owned()).or_default() += 1;
        if tracks.last() != Some(&track) {
            tracks.push(track);
        }
        if !albums.contains(&album) {
            albums.push(album);
        }
    }

    fn finish(self) -> Vec<Group> {
        self.groups
            .into_values()
            .map(|(spellings, tracks, albums)| Group {
                // À égalité : celle qui a une majuscule, puis le plus d'accents (« Électro »).
                name: spellings
                    .into_iter()
                    .max_by_key(|(n, count)| {
                        let upper = n.chars().next().is_some_and(char::is_uppercase);
                        (
                            *count,
                            upper,
                            n.chars().filter(|c| !c.is_ascii()).count(),
                            std::cmp::Reverse(n.clone()),
                        )
                    })
                    .map(|(n, _)| n)
                    .unwrap_or_default(),
                tracks,
                albums,
            })
            .collect()
    }
}

impl SterMusic {
    fn browse_index(&mut self) -> &BrowseIndex {
        if self.browse_index.is_none() {
            self.browse_index = Some(BrowseIndex::build(&self.lib));
        }
        self.browse_index.as_ref().unwrap()
    }

    /// Onglet « Parcourir » de la bibliothèque : genres, années ou compositeurs.
    pub(super) fn browse_tab(&mut self, ui: &mut egui::Ui, query: &str) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for kind in BrowseKind::ALL {
                if segment(ui, kind.label(), self.settings.browse_kind == kind).clicked() {
                    self.settings.browse_kind = kind;
                }
            }
        });
        ui.add_space(14.0);
        let kind = self.settings.browse_kind;
        let q = fold(query);
        let groups: Vec<Group> = self
            .browse_index()
            .groups(kind)
            .iter()
            .filter(|g| q.is_empty() || fold(&g.name).contains(&q))
            .cloned()
            .collect();
        if groups.is_empty() {
            let empty = if !q.is_empty() {
                tr("Rien ne correspond à la recherche.")
            } else {
                match kind {
                    BrowseKind::Genre => tr("Aucun morceau n'a de genre dans ses tags."),
                    BrowseKind::Year => tr("Aucun morceau n'a d'année dans ses tags."),
                    BrowseKind::Composer => tr("Aucun morceau n'a de compositeur dans ses tags."),
                }
            };
            ui.add_space(12.0);
            ui.label(RichText::new(empty).color(theme::TEXT_3));
            return;
        }
        if kind == BrowseKind::Year {
            self.years_by_decade(ui, &groups);
            return;
        }
        lists::virtual_grid(ui, &format!("browse_{kind:?}"), groups.len(), |ui, k, w| {
            self.group_card(ui, kind, &groups[k], w);
        });
    }

    /// Années rangées par décennie.
    fn years_by_decade(&mut self, ui: &mut egui::Ui, groups: &[Group]) {
        egui::ScrollArea::vertical()
            .id_salt("browse_years")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                let mut start = 0;
                while start < groups.len() {
                    let decade = |g: &Group| g.name.parse::<u32>().unwrap_or(0) / 10;
                    let d = decade(&groups[start]);
                    let end = groups[start..]
                        .iter()
                        .position(|g| decade(g) != d)
                        .map_or(groups.len(), |p| start + p);
                    let tracks: usize = groups[start..end].iter().map(|g| g.tracks.len()).sum();
                    lists::section(ui, &crate::trf!("Années {0}", d * 10), None);
                    ui.label(
                        RichText::new(plural(tracks, "morceau", "morceaux"))
                            .color(theme::TEXT_3)
                            .small(),
                    );
                    ui.add_space(8.0);
                    let slice = &groups[start..end];
                    lists::grid(ui, &format!("decade_{d}"), slice.len(), None, |ui, k, w| {
                        self.group_card(ui, BrowseKind::Year, &slice[k], w);
                    });
                    ui.add_space(14.0);
                    start = end;
                }
            });
    }

    /// Carte d'un genre, d'une année ou d'un compositeur : mosaïque de ses pochettes.
    fn group_card(&mut self, ui: &mut egui::Ui, kind: BrowseKind, group: &Group, w: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(w, lists::card_height(w)), Sense::hover());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let resp = ui.interact(rect, ui.id().with(("group", kind, &group.name)), Sense::click());
        let h = widgets::hover_t(ui, resp.id, resp.hovered());
        let cover = Rect::from_min_size(rect.min, vec2(w, w)).translate(vec2(0.0, -3.0 * h));
        ui.painter().add(
            egui::Shadow {
                offset: [0, (4.0 + 6.0 * h) as i8],
                blur: (10.0 + 14.0 * h) as u8,
                spread: 0,
                color: Color32::from_black_alpha((70.0 + 50.0 * h) as u8),
            }
            .as_shape(cover, CornerRadius::same(8)),
        );
        let first: Vec<usize> = group
            .albums
            .iter()
            .map(|&a| self.lib.albums[a].tracks[0])
            .collect();
        self.mosaic(ui, cover, &first, false, 8);
        // Voile en bas de la mosaïque, avec l'icône du regroupement.
        let band = Rect::from_min_max(pos2(cover.left(), cover.bottom() - 38.0), cover.max);
        ui.painter().rect_filled(
            band,
            CornerRadius {
                sw: 8,
                se: 8,
                ..Default::default()
            },
            Color32::from_black_alpha(120),
        );
        crate::icons::paint_centered(
            ui.painter(),
            kind.icon(),
            pos2(band.left() + 20.0, band.center().y),
            16.0,
            Color32::WHITE,
        );
        let g = widgets::galley(ui, &group.name, theme::semibold(14.0), theme::TEXT, w);
        ui.painter()
            .galley(pos2(rect.left(), cover.bottom() + 10.0 + 3.0 * h), g, theme::TEXT);
        let sub = format!(
            "{} · {}",
            plural(group.albums.len(), "album", "albums"),
            plural(group.tracks.len(), "morceau", "morceaux")
        );
        let g = widgets::galley(ui, &sub, FontId::proportional(13.0), theme::TEXT_2, w);
        ui.painter().galley(
            pos2(rect.left(), cover.bottom() + 31.0 + 3.0 * h),
            g,
            theme::TEXT_2,
        );
        if resp.clicked() {
            self.actions
                .push(Action::Go(View::Browse(kind, group.name.clone())));
        }
        let tracks = group.tracks.clone();
        resp.context_menu(|ui| {
            if widgets::menu_item(ui, Some(Icon::PlayOutline), tr("Lire")).clicked() {
                self.actions.push(Action::Play {
                    tracks: tracks.clone(),
                    start: 0,
                });
            }
            if widgets::menu_item(ui, Some(Icon::Shuffle), tr("Lecture aléatoire")).clicked() {
                self.actions.push(Action::PlayShuffled(tracks.clone()));
            }
            if widgets::menu_item(ui, Some(Icon::ListEnd), tr("Ajouter à la file d'attente")).clicked() {
                self.actions.push(Action::Enqueue(tracks.clone()));
            }
        });
    }

    /// Page d'un genre, d'une année ou d'un compositeur : ses albums, puis ses morceaux.
    pub(super) fn browse_page(&mut self, ui: &mut egui::Ui, kind: BrowseKind, name: &str) {
        let Some(group) = self.browse_index().find(kind, name).cloned() else {
            ui.label(RichText::new(tr("Plus rien ici : la bibliothèque a changé.")).color(theme::TEXT_3));
            return;
        };
        let total: std::time::Duration = group.tracks.iter().map(|&t| self.lib.tracks[t].duration()).sum();
        let info = format!(
            "{} · {} · {}",
            plural(group.albums.len(), "album", "albums"),
            plural(group.tracks.len(), "morceau", "morceaux"),
            fmt_long(total)
        );
        // Artistes les plus présents, en sous-titre.
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for &t in &group.tracks {
            *counts.entry(self.lib.tracks[t].artist_or_unknown()).or_default() += 1;
        }
        let mut top: Vec<(&str, usize)> = counts.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let subtitle = top.iter().take(3).map(|(a, _)| *a).collect::<Vec<_>>().join(", ");
        let first: Vec<usize> = group
            .albums
            .iter()
            .map(|&a| self.lib.albums[a].tracks[0])
            .collect();
        egui::ScrollArea::vertical()
            .id_salt(("browse", kind, name))
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                let tracks = group.tracks.clone();
                self.detail_header(
                    ui,
                    |s, ui, r| {
                        ui.painter().add(
                            egui::Shadow {
                                offset: [0, 8],
                                blur: 24,
                                spread: 0,
                                color: Color32::from_black_alpha(110),
                            }
                            .as_shape(r, CornerRadius::same(10)),
                        );
                        s.mosaic(ui, r, &first, false, 10);
                    },
                    Header {
                        kind: kind.kind_label(),
                        title: &group.name,
                        subtitle: &subtitle,
                        info: &info,
                    },
                    |s, ui| {
                        s.play_all_button(ui, &tracks);
                        if widgets::secondary_button(ui, Some(Icon::Shuffle), tr("Aléatoire")).clicked() {
                            s.actions.push(Action::PlayShuffled(tracks.clone()));
                        }
                        if widgets::secondary_button(ui, Some(Icon::ListEnd), tr("Ajouter à la file"))
                            .clicked()
                        {
                            s.actions.push(Action::Enqueue(tracks.clone()));
                        }
                        if tracks.len() >= 8 {
                            s.detail_search_field(ui, tr("Rechercher un titre"));
                        }
                    },
                );
                let shown = self.filtered(&group.tracks);
                let query = fold(self.detail_filter.trim());
                let albums: Vec<usize> = group
                    .albums
                    .iter()
                    .copied()
                    .filter(|&a| {
                        query.is_empty()
                            || fold(&self.lib.albums[a].title).contains(&query)
                            || self.lib.albums[a].tracks.iter().any(|t| shown.contains(t))
                    })
                    .collect();
                let mut c = self.view_ctx();
                if !albums.is_empty() {
                    lists::section(ui, tr("Albums"), None);
                    lists::grid(ui, "group_albums", albums.len(), Some(4), |ui, k, w| {
                        lists::album_card(ui, &mut c, albums[k], w)
                    });
                    ui.add_space(12.0);
                }
                lists::section(ui, tr("Morceaux"), None);
                if shown.is_empty() {
                    Self::no_match(ui);
                }
                let mut c = self.view_ctx();
                lists::track_list_inline(
                    ui,
                    &mut c,
                    &shown,
                    &ListOpts {
                        cover: true,
                        album: true,
                        ..Default::default()
                    },
                );
                ui.add_space(24.0);
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Track;

    fn track(path: &str, genre: &str, year: Option<u32>, composer: &str) -> Track {
        Track {
            path: path.into(),
            title: path.into(),
            artist: "A".into(),
            album: path.into(),
            album_artist: String::new(),
            genre: genre.into(),
            year,
            track_no: None,
            disc_no: None,
            duration_ms: 1000,
            mtime: 0,
            composer: composer.into(),
            schema: 0,
            cue: None,
            rating: None,
            remote: None,
        }
    }

    #[test]
    fn groups() {
        let lib = Library::new(vec![
            track("1", "Électro; Rock", Some(1997), "Bach"),
            track("2", "electro", Some(1997), ""),
            track("3", "Électro", None, "Bach / Mozart"),
            track("4", "", Some(2021), ""),
        ]);
        let idx = BrowseIndex::build(&lib);
        let names: Vec<&str> = idx.genres.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Électro", "Rock"]);
        assert_eq!(idx.genres[0].tracks.len(), 3);
        let years: Vec<&str> = idx.years.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(years, ["2021", "1997"]);
        assert_eq!(idx.composers.len(), 2);
        assert_eq!(
            idx.find(BrowseKind::Composer, "bach").map(|g| g.tracks.len()),
            Some(2)
        );
    }
}
