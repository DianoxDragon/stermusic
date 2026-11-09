//! Pages : accueil, bibliothèque, album, artiste, file d'attente, playlists, recherche,
//! en cours de lecture, paramètres.

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, UiBuilder,
};

use super::lists::{self, ListOpts};
use super::{Action, BackdropKind, LibTab, Page, PlaylistId, SongSort, SterMusic, View};
use crate::icons::{self, Icon};
use crate::library::{fmt_duration, fmt_long, plural};
use crate::{anim, theme, widgets};

const PAD_X: f32 = 32.0;

/// Textes de l'en-tête d'une page de détail (album, artiste, playlist).
struct Header<'a> {
    kind: &'a str,
    title: &'a str,
    subtitle: &'a str,
    info: &'a str,
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::semibold(28.0)).color(theme::TEXT));
}

fn meta(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::TEXT_2));
}

impl SterMusic {
    /// Égaliseur graphique 10 bandes : interrupteur, préréglages, pré-amplification et curseurs.
    pub(super) fn equalizer_panel(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.horizontal(|ui| {
            let mut on = self.settings.eq_on;
            if widgets::switch(ui, &mut on).changed() {
                self.settings.eq_on = on;
                changed = true;
            }
            ui.label(
                RichText::new("Égaliseur")
                    .font(theme::semibold(15.0))
                    .color(theme::TEXT),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_button(ui, Icon::Refresh, "Tout remettre à 0", 30.0, 15.0).clicked() {
                    self.settings.eq_gains = [0.0; 10];
                    self.settings.eq_preamp = 0.0;
                    self.settings.eq_preset = "Plat".into();
                    changed = true;
                }
            });
        });
        ui.add_space(6.0);
        // Préréglages en puces (une liste déroulante, dans la bulle de la barre de lecture,
        // fermerait la bulle au moindre clic).
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (name, gains) in crate::dsp::PRESETS {
                if segment(ui, name, self.settings.eq_preset == name).clicked() {
                    self.settings.eq_preset = name.into();
                    self.settings.eq_gains = gains;
                    self.settings.eq_on = true;
                    changed = true;
                }
            }
        });
        ui.add_space(10.0);
        // Éteint : les curseurs restent réglables, mais estompés.
        let on = self.settings.eq_on;
        ui.scope(|ui| {
            if !on {
                ui.set_opacity(0.45);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let height = 150.0;
                let column = |ui: &mut egui::Ui, id: usize, value: &mut f32, label: &str| -> bool {
                    ui.vertical(|ui| {
                        ui.set_width(34.0);
                        ui.vertical_centered(|ui| {
                            let shown = if value.abs() < 0.05 {
                                "0".to_owned()
                            } else if value.fract().abs() > 0.01 {
                                format!("{:+.1}", value)
                            } else {
                                format!("{:+.0}", value)
                            };
                            ui.label(RichText::new(shown).small().color(theme::TEXT_3));
                            let c = widgets::vslider(ui, ("eq", id), value, crate::dsp::MAX_DB, height);
                            ui.label(RichText::new(label).small().color(theme::TEXT_2));
                            c
                        })
                        .inner
                    })
                    .inner
                };
                let mut pre = self.settings.eq_preamp;
                if column(ui, 99, &mut pre, "Préampli") {
                    self.settings.eq_preamp = pre;
                    changed = true;
                }
                ui.add_space(10.0);
                let (r, _) = ui.allocate_exact_size(vec2(1.0, height + 30.0), Sense::hover());
                ui.painter()
                    .vline(r.center().x, r.y_range(), Stroke::new(1.0_f32, theme::stroke()));
                ui.add_space(10.0);
                for (i, freq) in crate::dsp::BANDS.iter().enumerate() {
                    let label = if *freq >= 1000.0 {
                        format!("{}k", freq / 1000.0)
                    } else {
                        format!("{freq}")
                    };
                    let mut v = self.settings.eq_gains[i];
                    if column(ui, i, &mut v, &label) {
                        self.settings.eq_gains[i] = v;
                        self.settings.eq_preset = "Personnalisé".into();
                        changed = true;
                    }
                }
            });
        });
        if changed {
            self.push_audio();
        }
    }

    /// « Tout lire » qui réagit : quand cette liste est en cours de lecture, il devient
    /// « Pause » / « Reprendre » (avec l'égaliseur animé à côté).
    fn play_all_button(&mut self, ui: &mut egui::Ui, tracks: &[usize]) {
        let loaded = self.player.as_ref().is_some_and(|p| p.has_track());
        let here = loaded && self.current().is_some_and(|c| tracks.contains(&c));
        let playing = self.is_playing();
        let (icon, label) = match (here, playing) {
            (true, true) => (Icon::Pause, "Pause"),
            (true, false) => (Icon::Play, "Reprendre"),
            _ => (Icon::Play, "Tout lire"),
        };
        if widgets::primary_button(ui, Some(icon), label).clicked() {
            self.actions.push(if here {
                Action::TogglePlay
            } else {
                Action::Play {
                    tracks: tracks.to_vec(),
                    start: 0,
                }
            });
        }
        if here {
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 34.0), Sense::hover());
            widgets::equalizer(ui.painter(), r.center(), 14.0, theme::accent_light(), playing);
        }
    }

    /// Champ de recherche aligné à droite sur la ligne des boutons de l'en-tête (album, artiste).
    fn detail_search_field(&mut self, ui: &mut egui::Ui, hint: &str) {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let w = ui.available_width().clamp(160.0, 280.0);
            widgets::search_field(
                ui,
                egui::Id::new("detail_search"),
                &mut self.detail_filter,
                hint,
                w,
            );
        });
    }

    /// Les morceaux de `tracks` qui correspondent à la recherche de la page.
    fn filtered(&self, tracks: &[usize]) -> Vec<usize> {
        tracks
            .iter()
            .copied()
            .filter(|&t| self.matches_filter(t))
            .collect()
    }

    fn no_match(ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.label(RichText::new("Aucun titre ne correspond à la recherche.").color(theme::TEXT_3));
    }

    fn matches_filter(&self, t: usize) -> bool {
        let q = self.detail_filter.trim().to_lowercase();
        q.is_empty() || {
            let tr = &self.lib.tracks[t];
            q.split_whitespace()
                .all(|w| tr.title.to_lowercase().contains(w) || tr.artist.to_lowercase().contains(w))
        }
    }

    /// Zone de contenu : un calque légèrement plus clair, arrondi en haut à gauche (Fluent).
    pub(super) fn content(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let now_playing = self.view == View::NowPlaying;
        if !now_playing {
            ui.painter().rect(
                rect,
                CornerRadius {
                    nw: 10,
                    ne: 0,
                    sw: 0,
                    se: 0,
                },
                theme::white(0x07),
                Stroke::new(1.0_f32, theme::white(0x0C)),
                StrokeKind::Inside,
            );
        }
        let view = self.view.clone();
        // Apparition de la page : fondu et léger glissement vers le haut ; ses éléments suivent en cascade.
        let t = lists::page_progress(ui.ctx(), &format!("{view:?}"));
        let inner = Rect::from_min_max(rect.min + vec2(PAD_X, 24.0), rect.max - vec2(PAD_X * 0.5, 0.0))
            .translate(vec2(0.0, 18.0 * (1.0 - t)));
        ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
            ui.set_opacity(t);
            match view {
                View::Page(Page::Home) => self.home(ui),
                View::Page(Page::Library) => self.library_page(ui),
                View::Page(Page::Queue) => self.queue_page(ui),
                View::Page(Page::Playlists) => self.playlists_page(ui),
                View::Page(Page::Settings) => self.settings_page(ui),
                View::Album(a) => self.album_page(ui, a),
                View::Artist(a) => self.artist_page(ui, a),
                View::Playlist(id) => self.playlist_page(ui, id),
                View::Search => self.search_page(ui),
                View::NowPlaying => self.now_playing_page(ui),
            }
        });
    }

    /// Bibliothèque vide : invitation à choisir un dossier.
    fn welcome(&mut self, ui: &mut egui::Ui) {
        if self.scan_rx.is_some() {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.spinner();
                ui.add_space(8.0);
                meta(
                    ui,
                    &format!(
                        "Analyse de la bibliothèque… {}",
                        plural(self.scan_count, "morceau", "morceaux")
                    ),
                );
            });
            return;
        }
        widgets::empty_state(
            ui,
            Icon::Music,
            "Bienvenue dans SterMusic",
            "Ajoute un dossier de musique pour construire ta bibliothèque.",
        );
        ui.vertical_centered(|ui| {
            let default = crate::library::default_music_dir();
            ui.horizontal(|ui| {
                let w = 190.0 + if default.is_some() { 230.0 } else { 0.0 };
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                if widgets::primary_button(ui, Some(Icon::FolderPlus), "Ajouter un dossier").clicked() {
                    self.actions.push(Action::AddFolder);
                }
                if let Some(dir) = default {
                    let name = dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if widgets::secondary_button(ui, Some(Icon::Music), &format!("Utiliser « {name} »"))
                        .clicked()
                    {
                        self.actions.push(Action::UseFolder(dir));
                    }
                }
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new("Astuce : tu peux aussi glisser un dossier dans la fenêtre.")
                    .small()
                    .color(theme::TEXT_3),
            );
        });
    }

    // ---------- Accueil ----------

    fn home(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Accueil");
        ui.add_space(16.0);
        if self.lib.is_empty() {
            self.welcome(ui);
            return;
        }
        // Albums écoutés récemment, puis ajoutés récemment, puis morceaux les plus écoutés.
        let mut recent: Vec<(u64, usize)> = self
            .settings
            .plays
            .iter()
            .filter_map(|(p, s)| self.lib.find(p).map(|t| (s.last, self.lib.track_album[t])))
            .collect();
        recent.sort_by_key(|r| std::cmp::Reverse(r.0));
        let mut seen = std::collections::HashSet::new();
        let recent_albums: Vec<usize> = recent
            .into_iter()
            .map(|(_, a)| a)
            .filter(|a| seen.insert(*a))
            .take(24)
            .collect();
        let mut added: Vec<usize> = (0..self.lib.albums.len()).collect();
        added.sort_by_key(|&a| std::cmp::Reverse(self.lib.albums[a].added));
        added.truncate(24);
        let mut top: Vec<(u32, usize)> = self
            .settings
            .plays
            .iter()
            .filter_map(|(p, s)| self.lib.find(p).map(|t| (s.count, t)))
            .collect();
        top.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let top: Vec<usize> = top.into_iter().map(|(_, t)| t).take(8).collect();

        egui::ScrollArea::vertical()
            .id_salt("home")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                self.hero(ui);
                ui.add_space(28.0);
                let mut c = self.view_ctx();
                if !recent_albums.is_empty() {
                    lists::section(ui, "Écoutés récemment", None);
                    lists::grid(ui, "recent", recent_albums.len(), Some(1), |ui, k, w| {
                        lists::album_card(ui, &mut c, recent_albums[k], w)
                    });
                    ui.add_space(20.0);
                }
                if lists::section(ui, "Ajoutés récemment", Some("Tous les albums")) {
                    c.actions.push(Action::Go(View::Page(Page::Library)));
                }
                lists::grid(ui, "added", added.len(), Some(1), |ui, k, w| {
                    lists::album_card(ui, &mut c, added[k], w)
                });
                if !top.is_empty() {
                    ui.add_space(20.0);
                    lists::section(ui, "Les plus écoutés", None);
                    lists::track_list_inline(
                        ui,
                        &mut c,
                        &top,
                        &ListOpts {
                            cover: true,
                            album: true,
                            ..Default::default()
                        },
                    );
                }
                ui.add_space(24.0);
            });
    }

    /// Bandeau d'accueil : reprendre la lecture, ou tout lire en aléatoire.
    fn hero(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(w, 168.0), Sense::hover());
        let painter = ui.painter();
        let lights = self.settings.visualizer && self.current().is_some();
        theme::brand_fill(painter, rect, 14, if lights { 0.16 } else { 0.30 });
        painter.rect_stroke(
            rect,
            CornerRadius::same(14),
            Stroke::new(1.0_f32, theme::white(0x14)),
            StrokeKind::Inside,
        );
        // Fond du bandeau : des lumières qui dérivent aux couleurs de la pochette et suivent la
        // musique (visualiseur), sinon un halo fixe.
        if self.settings.visualizer && self.current().is_some() {
            let palette = crate::viz::harmonize(self.viz_palette());
            if crate::shader::available() {
                // De vraies vagues, découpées aux coins arrondis du bandeau, plus discrètes côté texte.
                let mut w = crate::viz::waves(&self.spectrum, palette, 0.75);
                w.base = 0.74;
                w.amp = 0.5;
                w.fade_top = 0.2;
                w.fade_bottom = 0.12;
                w.left = 1.0;
                w.radius = 14.0;
                crate::shader::paint(ui.painter(), rect, w);
            } else {
                crate::viz::drifting_lights(ui.painter(), rect, &self.spectrum, palette);
            }
        } else {
            theme_glow(ui.painter(), rect);
        }
        let painter = ui.painter();

        let current = self.current();
        let inner = rect.shrink(24.0);
        let cover = Rect::from_min_size(inner.min, vec2(inner.height(), inner.height()));
        let (eyebrow, title, sub) = match current {
            Some(i) => {
                let t = &self.lib.tracks[i];
                let tex = self.covers.thumb(&t.path).cloned();
                painter.add(
                    egui::Shadow {
                        offset: [0, 6],
                        blur: 18,
                        spread: 0,
                        color: Color32::from_black_alpha(100),
                    }
                    .as_shape(cover, CornerRadius::same(10)),
                );
                widgets::cover(painter, cover, tex.as_ref(), 10, &t.album);
                let eyebrow = if self.is_playing() {
                    "EN COURS DE LECTURE"
                } else {
                    "REPRENDRE LA LECTURE"
                };
                (
                    eyebrow,
                    t.title.clone(),
                    format!("{} — {}", t.artist_or_unknown(), t.album_or_unknown()),
                )
            }
            None => {
                painter.rect_filled(cover, CornerRadius::same(10), theme::white(0x10));
                icons::paint_centered(painter, Icon::Sparkles, cover.center(), 44.0, theme::white(0xC0));
                (
                    "TA BIBLIOTHÈQUE",
                    format!(
                        "{} à écouter",
                        plural(self.lib.tracks.len(), "morceau", "morceaux")
                    ),
                    format!(
                        "{} · {}",
                        plural(self.lib.albums.len(), "album", "albums"),
                        plural(self.lib.artists.len(), "artiste", "artistes")
                    ),
                )
            }
        };
        let x = cover.right() + 22.0;
        let tw = inner.right() - x;
        painter.text(
            pos2(x, inner.top() + 6.0),
            Align2::LEFT_TOP,
            eyebrow,
            FontId::proportional(11.5),
            theme::accent_light(),
        );
        let g = widgets::galley(ui, &title, theme::semibold(26.0), theme::TEXT, tw);
        ui.painter().galley(pos2(x, inner.top() + 24.0), g, theme::TEXT);
        let g = widgets::galley(ui, &sub, FontId::proportional(14.0), theme::TEXT_2, tw);
        ui.painter().galley(pos2(x, inner.top() + 62.0), g, theme::TEXT_2);

        let buttons = Rect::from_min_max(pos2(x, inner.bottom() - 36.0), inner.max);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(buttons)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                if current.is_some() {
                    let playing = self.is_playing();
                    if widgets::primary_button(
                        ui,
                        Some(if playing { Icon::Pause } else { Icon::Play }),
                        if playing { "Pause" } else { "Reprendre" },
                    )
                    .clicked()
                    {
                        self.actions.push(Action::TogglePlay);
                    }
                    if widgets::secondary_button(ui, Some(Icon::Maximize), "En cours de lecture").clicked() {
                        self.actions.push(Action::Go(View::NowPlaying));
                    }
                } else if widgets::primary_button(ui, Some(Icon::Shuffle), "Tout lire en aléatoire").clicked()
                {
                    self.actions
                        .push(Action::PlayShuffled((0..self.lib.tracks.len()).collect()));
                }
            },
        );
    }

    // ---------- Bibliothèque ----------

    fn library_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            heading(ui, "Bibliothèque musicale");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if !self.lib.is_empty()
                    && widgets::secondary_button(ui, Some(Icon::Shuffle), "Tout lire en aléatoire").clicked()
                {
                    self.actions
                        .push(Action::PlayShuffled((0..self.lib.tracks.len()).collect()));
                }
                if self.scan_rx.is_some() {
                    ui.spinner();
                    ui.label(
                        RichText::new(format!("Analyse… {}", self.scan_count))
                            .small()
                            .color(theme::TEXT_3),
                    );
                }
            });
        });
        ui.add_space(14.0);
        if self.lib.is_empty() {
            self.welcome(ui);
            return;
        }

        // Onglets, avec un soulignement qui glisse de l'un à l'autre.
        let tabs = [
            (LibTab::Songs, "Morceaux"),
            (LibTab::Albums, "Albums"),
            (LibTab::Artists, "Artistes"),
        ];
        let mut target = None;
        let tabs_rect = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (tab, label) in tabs {
                    let selected = self.settings.lib_tab == tab;
                    let galley =
                        ui.fonts(|f| f.layout_no_wrap(label.to_owned(), theme::semibold(15.0), theme::TEXT));
                    let (r, resp) =
                        ui.allocate_exact_size(vec2(galley.size().x + 24.0, 34.0), Sense::click());
                    widgets::paint_interaction(ui, r, 6, &resp);
                    let color = if selected { theme::TEXT } else { theme::TEXT_2 };
                    ui.painter()
                        .galley(r.center() - galley.size() / 2.0, galley, color);
                    if resp.clicked() {
                        self.settings.lib_tab = tab;
                    }
                    if selected {
                        target = Some(r);
                    }
                }
                if self.settings.lib_tab == LibTab::Songs {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(12.0);
                        let sorts = [
                            (SongSort::Title, "Titre"),
                            (SongSort::Artist, "Artiste"),
                            (SongSort::Album, "Album"),
                            (SongSort::Added, "Date d'ajout"),
                        ];
                        let current = sorts
                            .iter()
                            .find(|s| s.0 == self.settings.song_sort)
                            .map(|s| s.1)
                            .unwrap_or("");
                        egui::ComboBox::from_id_salt("song_sort")
                            .selected_text(current)
                            .width(130.0)
                            .show_ui(ui, |ui| {
                                for (s, l) in sorts {
                                    ui.selectable_value(&mut self.settings.song_sort, s, l);
                                }
                            });
                        ui.label(RichText::new("Trier par").color(theme::TEXT_3));
                    });
                }
            })
            .response
            .rect;
        if let Some(r) = target {
            let x = anim::value(
                ui.ctx(),
                egui::Id::new("tab_underline_x"),
                r.center().x,
                anim::NORMAL,
                anim::Easing::OutExpo,
            );
            let w = anim::value(
                ui.ctx(),
                egui::Id::new("tab_underline_w"),
                r.width() - 24.0,
                anim::NORMAL,
                anim::Easing::OutExpo,
            );
            let line = Rect::from_center_size(pos2(x, tabs_rect.bottom() + 1.0), vec2(w, 3.0));
            theme::brand_fill(ui.painter(), line, 2, 1.0);
        }
        ui.add_space(14.0);

        match self.settings.lib_tab {
            LibTab::Songs => {
                let songs = self.sorted_songs().to_vec();
                let opts = ListOpts {
                    cover: true,
                    album: true,
                    ..Default::default()
                };
                lists::header(ui, &opts);
                let mut c = self.view_ctx();
                lists::track_list(ui, &mut c, "songs", &songs, &opts);
            }
            LibTab::Albums => {
                let n = self.lib.albums.len();
                let mut c = self.view_ctx();
                lists::virtual_grid(ui, "albums", n, |ui, k, w| lists::album_card(ui, &mut c, k, w));
            }
            LibTab::Artists => {
                let n = self.lib.artists.len();
                let mut c = self.view_ctx();
                lists::virtual_grid(ui, "artists", n, |ui, k, w| lists::artist_card(ui, &mut c, k, w));
            }
        }
    }

    // ---------- Album et artiste ----------

    /// En-tête de détail : grande image, titres et boutons d'action.
    fn detail_header(
        &mut self,
        ui: &mut egui::Ui,
        art: impl FnOnce(&mut Self, &egui::Ui, Rect),
        text: Header,
        buttons: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        let Header {
            kind,
            title,
            subtitle,
            info,
        } = text;
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(w, 212.0), Sense::hover());
        let art_rect = Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(200.0, 200.0));
        art(self, ui, art_rect);
        let x = art_rect.right() + 28.0;
        let tw = rect.right() - x - 16.0;
        let p = ui.painter();
        p.text(
            pos2(x, art_rect.top() + 18.0),
            Align2::LEFT_TOP,
            kind,
            FontId::proportional(12.0),
            theme::accent_light(),
        );
        let g = widgets::galley(ui, title, theme::semibold(34.0), theme::TEXT, tw);
        ui.painter()
            .galley(pos2(x, art_rect.top() + 36.0), g, theme::TEXT);
        let g = widgets::galley(ui, subtitle, FontId::proportional(16.0), theme::TEXT, tw);
        ui.painter()
            .galley(pos2(x, art_rect.top() + 86.0), g, theme::TEXT);
        let g = widgets::galley(ui, info, FontId::proportional(13.5), theme::TEXT_2, tw);
        ui.painter()
            .galley(pos2(x, art_rect.top() + 112.0), g, theme::TEXT_2);
        let b = Rect::from_min_max(
            pos2(x, art_rect.bottom() - 40.0),
            pos2(rect.right(), art_rect.bottom()),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(b)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| buttons(self, ui),
        );
        ui.add_space(18.0);
    }

    fn album_page(&mut self, ui: &mut egui::Ui, a: usize) {
        let Some(album) = self.lib.albums.get(a).cloned() else {
            return;
        };
        let mut info = vec![
            plural(album.tracks.len(), "morceau", "morceaux"),
            fmt_long(album.duration),
        ];
        if let Some(y) = album.year {
            info.insert(0, y.to_string());
        }
        if !album.genre.is_empty() {
            info.insert(info.len() - 2, album.genre.clone());
        }
        let first = self.lib.tracks[album.tracks[0]].path.clone();
        egui::ScrollArea::vertical()
            .id_salt(("album", a))
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                let tracks = album.tracks.clone();
                self.detail_header(
                    ui,
                    |s, ui, r| {
                        let tex = s.covers.thumb(&first).cloned();
                        ui.painter().add(
                            egui::Shadow {
                                offset: [0, 8],
                                blur: 24,
                                spread: 0,
                                color: Color32::from_black_alpha(110),
                            }
                            .as_shape(r, CornerRadius::same(10)),
                        );
                        widgets::cover(ui.painter(), r, tex.as_ref(), 10, &album.title);
                    },
                    Header {
                        kind: "ALBUM",
                        title: &album.title,
                        subtitle: &album.artist,
                        info: &info.join(" · "),
                    },
                    |s, ui| {
                        s.play_all_button(ui, &tracks);
                        if widgets::secondary_button(ui, Some(Icon::Shuffle), "Aléatoire").clicked() {
                            s.actions.push(Action::PlayShuffled(tracks.clone()));
                        }
                        let resp = widgets::secondary_button(ui, Some(Icon::Plus), "Ajouter à");
                        let popup_id = ui.make_persistent_id("album_add_to");
                        if resp.clicked() {
                            ui.memory_mut(|m| m.toggle_popup(popup_id));
                        }
                        let c = s.view_ctx();
                        egui::popup::popup_below_widget(
                            ui,
                            popup_id,
                            &resp,
                            egui::PopupCloseBehavior::CloseOnClick,
                            |ui| {
                                ui.set_min_width(230.0);
                                if widgets::menu_item(ui, Some(Icon::ListEnd), "File d'attente").clicked() {
                                    c.actions.push(Action::Enqueue(tracks.clone()));
                                }
                                if widgets::menu_item(ui, Some(Icon::Plus), "Nouvelle playlist…").clicked()
                                {
                                    c.actions.push(Action::NewPlaylist(tracks.clone()));
                                }
                                for (k, pl) in c.settings.playlists.iter().enumerate() {
                                    if widgets::menu_item(ui, Some(Icon::ListMusic), &pl.name).clicked() {
                                        c.actions.push(Action::AddToPlaylist(k, tracks.clone()));
                                    }
                                }
                            },
                        );
                        let artist = s.lib.album_artist[a];
                        if widgets::secondary_button(ui, Some(Icon::Mic), "Artiste").clicked() {
                            s.actions.push(Action::Go(View::Artist(artist)));
                        }
                        if tracks.len() >= 8 {
                            s.detail_search_field(ui, "Rechercher un titre");
                        }
                    },
                );
                let shown = self.filtered(&album.tracks);
                if shown.is_empty() {
                    Self::no_match(ui);
                }
                let mut c = self.view_ctx();
                let opts = ListOpts {
                    track_no: true,
                    ..Default::default()
                };
                lists::header(ui, &opts);
                lists::track_list_inline(ui, &mut c, &shown, &opts);
                ui.add_space(24.0);
            });
    }

    fn artist_page(&mut self, ui: &mut egui::Ui, ar: usize) {
        let Some(artist) = self.lib.artists.get(ar).cloned() else {
            return;
        };
        let total: std::time::Duration = artist.tracks.iter().map(|&t| self.lib.tracks[t].duration()).sum();
        let info = format!(
            "{} · {} · {}",
            plural(artist.albums.len(), "album", "albums"),
            plural(artist.tracks.len(), "morceau", "morceaux"),
            fmt_long(total)
        );
        let first = artist
            .albums
            .first()
            .map(|&a| self.lib.tracks[self.lib.albums[a].tracks[0]].path.clone());
        egui::ScrollArea::vertical()
            .id_salt(("artist", ar))
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                let tracks = artist.tracks.clone();
                self.detail_header(
                    ui,
                    |s, ui, r| {
                        let tex = first.as_ref().and_then(|p| s.covers.thumb(p).cloned());
                        ui.painter().circle_filled(
                            r.center() + vec2(0.0, 6.0),
                            r.width() / 2.0 + 2.0,
                            Color32::from_black_alpha(90),
                        );
                        widgets::circle_cover(ui, r.center(), r.width() / 2.0, tex.as_ref(), &artist.name);
                    },
                    Header {
                        kind: "ARTISTE",
                        title: &artist.name,
                        subtitle: "",
                        info: &info,
                    },
                    |s, ui| {
                        s.play_all_button(ui, &tracks);
                        if widgets::secondary_button(ui, Some(Icon::Shuffle), "Aléatoire").clicked() {
                            s.actions.push(Action::PlayShuffled(tracks.clone()));
                        }
                        if widgets::secondary_button(ui, Some(Icon::ListEnd), "Ajouter à la file").clicked()
                        {
                            s.actions.push(Action::Enqueue(tracks.clone()));
                        }
                        s.detail_search_field(ui, "Rechercher chez cet artiste");
                    },
                );
                // La recherche garde les albums dont le titre correspond, ou qui contiennent un titre trouvé.
                let shown = self.filtered(&artist.tracks);
                let query = self.detail_filter.trim().to_lowercase();
                let albums: Vec<usize> = artist
                    .albums
                    .iter()
                    .copied()
                    .filter(|&a| {
                        query.is_empty()
                            || self.lib.albums[a].title.to_lowercase().contains(&query)
                            || self.lib.albums[a].tracks.iter().any(|t| shown.contains(t))
                    })
                    .collect();
                let mut c = self.view_ctx();
                if !albums.is_empty() {
                    lists::section(ui, "Albums", None);
                    lists::grid(ui, "artist_albums", albums.len(), None, |ui, k, w| {
                        lists::album_card(ui, &mut c, albums[k], w)
                    });
                    ui.add_space(12.0);
                }
                lists::section(ui, "Morceaux", None);
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

    // ---------- File d'attente ----------

    fn queue_page(&mut self, ui: &mut egui::Ui) {
        let total: std::time::Duration = self.queue.iter().map(|&t| self.lib.tracks[t].duration()).sum();
        ui.horizontal(|ui| {
            heading(ui, "File d'attente");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if !self.queue.is_empty() {
                    if widgets::secondary_button(ui, Some(Icon::Trash), "Effacer").clicked() {
                        self.actions.push(Action::QueueClear);
                    }
                    if widgets::secondary_button(ui, Some(Icon::ListPlus), "Enregistrer comme playlist")
                        .clicked()
                    {
                        self.actions.push(Action::SaveQueueAsPlaylist);
                    }
                    let w = (ui.available_width() - 16.0).clamp(160.0, 280.0);
                    widgets::search_field(
                        ui,
                        egui::Id::new("queue_search"),
                        &mut self.queue_filter,
                        "Rechercher dans la file",
                        w,
                    );
                }
            });
        });
        if self.queue.is_empty() {
            widgets::empty_state(
                ui,
                Icon::ListMusic,
                "La file d'attente est vide",
                "Lance un album, un artiste ou un morceau pour la remplir.",
            );
            return;
        }

        // Positions affichées : toute la file, ou celles qui correspondent à la recherche.
        let query = self.queue_filter.trim().to_lowercase();
        let positions: Vec<usize> = if query.is_empty() {
            (0..self.queue.len()).collect()
        } else {
            (0..self.queue.len())
                .filter(|&p| {
                    let t = &self.lib.tracks[self.queue[p]];
                    let text = format!("{} {} {}", t.title, t.artist, t.album).to_lowercase();
                    query.split_whitespace().all(|w| text.contains(w))
                })
                .collect()
        };
        let filtered = !query.is_empty();
        if filtered {
            meta(
                ui,
                &format!(
                    "{} sur {} · effacez la recherche pour réorganiser",
                    plural(positions.len(), "résultat", "résultats"),
                    self.queue.len()
                ),
            );
        } else {
            meta(
                ui,
                &format!(
                    "{} · {} · glisser une ligne pour la déplacer",
                    plural(self.queue.len(), "morceau", "morceaux"),
                    fmt_long(total)
                ),
            );
        }
        ui.add_space(12.0);

        let queue = self.queue.clone();
        let opts = ListOpts {
            cover: true,
            album: true,
            numbered: true,
            queue_pos: Some(self.queue_pos),
            ..Default::default()
        };
        lists::header(ui, &opts);
        let mut drag = if filtered { None } else { self.queue_drag };
        let mut rows: Vec<(usize, Rect)> = Vec::new();
        let pointer = ui.ctx().pointer_latest_pos();
        let mut c = self.view_ctx();
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical()
            .id_salt("queue")
            .auto_shrink(false)
            .show_rows(ui, lists::ROW_H, positions.len(), |ui, range| {
                for r in range {
                    let pos = positions[r];
                    let out = lists::track_row(ui, &mut c, &queue, pos, &opts);
                    rows.push((pos, out.rect));
                    if out.drag_started && !filtered {
                        drag = Some(pos);
                    }
                }
                ui.add_space(16.0);
                // Défilement automatique près des bords pendant le glissement (longues files).
                if let (Some(_), Some(p)) = (drag, pointer) {
                    let clip = ui.clip_rect();
                    let edge = 56.0;
                    let speed = if p.y < clip.top() + edge {
                        ((clip.top() + edge - p.y) / edge).min(1.5) * 22.0
                    } else if p.y > clip.bottom() - edge {
                        -((p.y - (clip.bottom() - edge)) / edge).min(1.5) * 22.0
                    } else {
                        0.0
                    };
                    if speed != 0.0 {
                        ui.scroll_with_delta(vec2(0.0, speed));
                        ui.ctx().request_repaint();
                    }
                }
            });

        // Glisser-déposer : la ligne d'origine s'estompe, un fantôme suit la souris, un trait
        // montre où elle ira ; le déplacement se fait au relâchement.
        let mut drop_to = None;
        if let (Some(from), Some(pointer), Some(&(first_pos, first_rect))) = (drag, pointer, rows.first()) {
            if let Some((_, r)) = rows.iter().find(|(p, _)| *p == from) {
                ui.painter().rect_filled(
                    r.shrink2(vec2(0.0, 2.0)),
                    CornerRadius::same(6),
                    theme::alpha(theme::bg(), 0xA0),
                );
            }
            let origin = first_rect.top() - first_pos as f32 * lists::ROW_H;
            let to = (((pointer.y - origin) / lists::ROW_H).round().max(0.0) as usize).min(queue.len());
            let y = origin + to as f32 * lists::ROW_H;
            let x = first_rect.x_range();
            let clip = ui.clip_rect();
            if y >= clip.top() - 2.0 && y <= clip.bottom() + 2.0 {
                let line = Rect::from_min_max(pos2(x.min, y - 1.5), pos2(x.max, y + 1.5));
                theme::brand_fill(ui.painter(), line, 2, 1.0);
                ui.painter()
                    .circle_filled(pos2(x.min, y), 4.0, theme::accent_light());
            }
            self.paint_drag_ghost(ui.ctx(), queue[from], pointer, x.span().min(460.0));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if ui.input(|i| i.pointer.any_released()) {
                drop_to = Some((from, to));
                drag = None;
            }
        }
        if let Some((from, to)) = drop_to {
            if to != from && to != from + 1 {
                self.actions.push(Action::QueueMove(from, to));
            }
        }
        self.queue_drag = drag;
    }

    /// Fantôme du morceau déplacé dans la file d'attente, qui suit la souris.
    fn paint_drag_ghost(&mut self, ctx: &egui::Context, track: usize, pointer: egui::Pos2, width: f32) {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("queue_ghost"),
        ));
        let rect = Rect::from_min_size(pointer + vec2(14.0, -20.0), vec2(width, 40.0));
        painter.add(
            egui::Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: Color32::from_black_alpha(140),
            }
            .as_shape(rect, CornerRadius::same(8)),
        );
        painter.rect(
            rect,
            CornerRadius::same(8),
            theme::alpha(theme::glass(), 0xF0),
            Stroke::new(1.0_f32, theme::alpha(theme::accent_light(), 0x80)),
            StrokeKind::Inside,
        );
        let t = self.lib.tracks[track].clone();
        let tex = self.covers.thumb(&t.path).cloned();
        let cover = Rect::from_min_size(rect.min + vec2(5.0, 5.0), vec2(30.0, 30.0));
        widgets::cover(&painter, cover, tex.as_ref(), 4, &t.album);
        let text = format!("{} — {}", t.title, t.artist_or_unknown());
        let mut job = egui::text::LayoutJob::single_section(
            text,
            egui::TextFormat::simple(FontId::proportional(13.5), theme::TEXT),
        );
        job.wrap = egui::text::TextWrapping {
            max_width: width - 52.0,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let g = ctx.fonts(|f| f.layout_job(job));
        painter.galley(
            pos2(cover.right() + 10.0, rect.center().y - g.size().y / 2.0),
            g,
            theme::TEXT,
        );
    }

    // ---------- Playlists ----------

    /// Mosaïque des pochettes d'une playlist (jusqu'à quatre albums différents).
    fn mosaic(&mut self, ui: &egui::Ui, rect: Rect, tracks: &[usize], favorites: bool, radius: u8) {
        let painter = ui.painter();
        if favorites {
            theme::brand_fill(painter, rect, radius, 1.0);
            icons::paint_centered(
                painter,
                Icon::HeartFilled,
                rect.center(),
                rect.width() * 0.32,
                Color32::WHITE,
            );
            return;
        }
        let mut albums = Vec::new();
        for &t in tracks {
            let a = self.lib.track_album[t];
            if !albums.contains(&a) {
                albums.push(a);
            }
            if albums.len() == 4 {
                break;
            }
        }
        if albums.len() < 4 {
            let tex = albums.first().and_then(|&a| {
                self.covers
                    .thumb(&self.lib.tracks[self.lib.albums[a].tracks[0]].path)
                    .cloned()
            });
            if albums.is_empty() {
                painter.rect_filled(rect, CornerRadius::same(radius), theme::white(0x0C));
                icons::paint_centered(
                    painter,
                    Icon::ListMusic,
                    rect.center(),
                    rect.width() * 0.3,
                    theme::TEXT_3,
                );
            } else {
                widgets::cover(
                    painter,
                    rect,
                    tex.as_ref(),
                    radius,
                    &self.lib.albums[albums[0]].title,
                );
            }
            return;
        }
        let half = rect.width() / 2.0;
        for (k, &a) in albums.iter().enumerate() {
            let r = Rect::from_min_size(
                rect.min + vec2((k % 2) as f32 * half, (k / 2) as f32 * half),
                vec2(half, half),
            );
            let tex = self
                .covers
                .thumb(&self.lib.tracks[self.lib.albums[a].tracks[0]].path)
                .cloned();
            let corner = |on: bool| if on { radius } else { 0 };
            let cr = CornerRadius {
                nw: corner(k == 0),
                ne: corner(k == 1),
                sw: corner(k == 2),
                se: corner(k == 3),
            };
            match tex {
                Some(t) => {
                    ui.painter().add(
                        egui::epaint::RectShape::filled(r, cr, Color32::WHITE)
                            .with_texture(t.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                    );
                }
                None => {
                    let (c1, _) = theme::seeded_colors(&self.lib.albums[a].title);
                    ui.painter().rect_filled(r, cr, c1);
                }
            }
        }
    }

    fn playlists_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            heading(ui, "Playlists");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if widgets::secondary_button(ui, Some(Icon::Download), "Importer (M3U)").clicked() {
                    self.actions.push(Action::ImportPlaylist);
                }
                if widgets::primary_button(ui, Some(Icon::Plus), "Nouvelle playlist").clicked() {
                    self.actions.push(Action::NewPlaylist(Vec::new()));
                }
            });
        });
        ui.add_space(18.0);
        let mut ids = vec![PlaylistId::Favorites];
        ids.extend((0..self.settings.playlists.len()).map(PlaylistId::User));
        egui::ScrollArea::vertical()
            .id_salt("playlists")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                let n = ids.len();
                let (cols, w) = lists::grid_layout(ui.available_width());
                for r in 0..n.div_ceil(cols) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = lists::GAP;
                        for &id in &ids[r * cols..((r + 1) * cols).min(n)] {
                            self.playlist_card(ui, id, w);
                        }
                    });
                    ui.add_space(lists::GAP * 0.6);
                }
            });
    }

    fn playlist_card(&mut self, ui: &mut egui::Ui, id: PlaylistId, w: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(w, lists::card_height(w)), Sense::hover());
        let resp = ui.interact(rect, ui.id().with(("pl_card", format!("{id:?}"))), Sense::click());
        let h = widgets::hover_t(ui, resp.id, resp.hovered());
        let tracks = self.playlist_tracks(id);
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
        self.mosaic(ui, cover, &tracks, id == PlaylistId::Favorites, 8);
        let name = self.playlist_name(id);
        let g = widgets::galley(ui, &name, theme::semibold(14.0), theme::TEXT, w);
        ui.painter()
            .galley(pos2(rect.left(), cover.bottom() + 10.0 + 3.0 * h), g, theme::TEXT);
        let g = widgets::galley(
            ui,
            &plural(tracks.len(), "morceau", "morceaux"),
            FontId::proportional(13.0),
            theme::TEXT_2,
            w,
        );
        ui.painter().galley(
            pos2(rect.left(), cover.bottom() + 31.0 + 3.0 * h),
            g,
            theme::TEXT_2,
        );
        if resp.clicked() {
            self.actions.push(Action::Go(View::Playlist(id)));
        }
        resp.context_menu(|ui| {
            if widgets::menu_item(ui, Some(Icon::PlayOutline), "Lire").clicked() {
                self.actions.push(Action::Play {
                    tracks: tracks.clone(),
                    start: 0,
                });
            }
            if widgets::menu_item(ui, Some(Icon::Shuffle), "Lecture aléatoire").clicked() {
                self.actions.push(Action::PlayShuffled(tracks.clone()));
            }
            if widgets::menu_item(ui, Some(Icon::Upload), "Exporter (M3U)").clicked() {
                self.actions.push(Action::ExportPlaylist(id));
            }
            if let PlaylistId::User(i) = id {
                ui.separator();
                if widgets::menu_item(ui, Some(Icon::Pencil), "Renommer").clicked() {
                    self.actions.push(Action::RenamePlaylist(i));
                }
                if widgets::menu_item(ui, Some(Icon::Trash), "Supprimer").clicked() {
                    self.actions.push(Action::DeletePlaylist(i));
                }
            }
        });
    }

    fn playlist_page(&mut self, ui: &mut egui::Ui, id: PlaylistId) {
        if let PlaylistId::User(i) = id {
            if i >= self.settings.playlists.len() {
                return;
            }
        }
        let tracks = self.playlist_tracks(id);
        let total: std::time::Duration = tracks.iter().map(|&t| self.lib.tracks[t].duration()).sum();
        let name = self.playlist_name(id);
        let info = format!(
            "{} · {}",
            plural(tracks.len(), "morceau", "morceaux"),
            fmt_long(total)
        );
        let t2 = tracks.clone();
        self.detail_header(
            ui,
            |s, ui, r| s.mosaic(ui, r, &t2, id == PlaylistId::Favorites, 10),
            Header {
                kind: "PLAYLIST",
                title: &name,
                subtitle: "",
                info: &info,
            },
            |s, ui| {
                ui.add_enabled_ui(!tracks.is_empty(), |ui| {
                    s.play_all_button(ui, &tracks);
                    if widgets::secondary_button(ui, Some(Icon::Shuffle), "Aléatoire").clicked() {
                        s.actions.push(Action::PlayShuffled(tracks.clone()));
                    }
                });
                if let PlaylistId::User(i) = id {
                    if widgets::icon_button(ui, Icon::Pencil, "Renommer", 34.0, 16.0).clicked() {
                        s.actions.push(Action::RenamePlaylist(i));
                    }
                }
                if widgets::icon_button(ui, Icon::Upload, "Exporter (M3U)", 34.0, 16.0).clicked() {
                    s.actions.push(Action::ExportPlaylist(id));
                }
                if let PlaylistId::User(i) = id {
                    if widgets::icon_button(ui, Icon::Trash, "Supprimer la playlist", 34.0, 16.0).clicked() {
                        s.actions.push(Action::DeletePlaylist(i));
                    }
                }
            },
        );
        if tracks.is_empty() {
            let (title, sub) = if id == PlaylistId::Favorites {
                (
                    "Aucun favori pour l'instant",
                    "Clique sur le cœur d'un morceau pour l'ajouter ici.",
                )
            } else {
                (
                    "Cette playlist est vide",
                    "Clic droit sur un morceau ou un album → « Ajouter à une playlist ».",
                )
            };
            widgets::empty_state(
                ui,
                if id == PlaylistId::Favorites {
                    Icon::Heart
                } else {
                    Icon::ListMusic
                },
                title,
                sub,
            );
            return;
        }
        let opts = ListOpts {
            cover: true,
            album: true,
            playlist: Some(id),
            ..Default::default()
        };
        lists::header(ui, &opts);
        let mut c = self.view_ctx();
        lists::track_list(ui, &mut c, &format!("pl{id:?}"), &tracks, &opts);
    }

    // ---------- Recherche ----------

    fn search_page(&mut self, ui: &mut egui::Ui) {
        heading(ui, &format!("Résultats pour « {} »", self.search.trim()));
        ui.add_space(12.0);
        let (tracks, albums, artists) = self.search_results.clone();
        if tracks.is_empty() && albums.is_empty() && artists.is_empty() {
            widgets::empty_state(
                ui,
                Icon::Search,
                "Aucun résultat",
                "Essaie avec un autre titre, artiste ou album.",
            );
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("search")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                let mut c = self.view_ctx();
                if !artists.is_empty() {
                    lists::section(ui, "Artistes", None);
                    lists::grid(ui, "found_artists", artists.len(), Some(1), |ui, k, w| {
                        lists::artist_card(ui, &mut c, artists[k], w)
                    });
                    ui.add_space(12.0);
                }
                if !albums.is_empty() {
                    lists::section(ui, "Albums", None);
                    lists::grid(ui, "found_albums", albums.len(), Some(2), |ui, k, w| {
                        lists::album_card(ui, &mut c, albums[k], w)
                    });
                    ui.add_space(12.0);
                }
                if !tracks.is_empty() {
                    lists::section(ui, &format!("Morceaux ({})", tracks.len()), None);
                    let shown: Vec<usize> = tracks.iter().copied().take(200).collect();
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
                }
                ui.add_space(24.0);
            });
    }

    // ---------- En cours de lecture ----------

    fn now_playing_page(&mut self, ui: &mut egui::Ui) {
        let Some(idx) = self.current() else {
            widgets::empty_state(
                ui,
                Icon::Music,
                "Rien en lecture",
                "Choisis un morceau dans ta bibliothèque.",
            );
            return;
        };
        let t = self.lib.tracks[idx].clone();
        let album = self.lib.track_album[idx];
        let artist = self.lib.album_artist[album];
        let rect = ui.max_rect();
        let upcoming: Vec<usize> = self
            .queue_pos
            .map(|p| self.queue.iter().skip(p + 1).take(5).copied().collect())
            .unwrap_or_default();
        let side = (rect.height() - 140.0)
            .min(rect.width() * 0.38)
            .clamp(160.0, 420.0);
        let total_w = side + 40.0 + (rect.width() * 0.42).min(480.0);
        let left = rect.center().x - total_w / 2.0;
        let cover = Rect::from_min_size(
            pos2(left, rect.top() + (rect.height() - side) / 2.0 - 30.0),
            vec2(side, side),
        );
        let appear = anim::appear(ui.ctx(), egui::Id::new(("np_cover", idx)), anim::SLOW);
        // En pause, la pochette se rétracte légèrement ; elle reprend sa taille à la lecture.
        let playing = self.is_playing();
        let live = anim::value(
            ui.ctx(),
            egui::Id::new("np_live"),
            if playing { 1.0 } else { 0.0 },
            0.45,
            anim::Easing::OutBack,
        );
        // Visualiseur : l'Aurore dans le bas de la page et un halo doux derrière la pochette
        // (la pochette elle-même ne bouge pas avec la musique).
        let viz = self.settings.visualizer;
        let cover = Rect::from_center_size(
            cover.center() + vec2(0.0, 12.0 * (1.0 - appear)),
            cover.size() * (0.96 + 0.04 * appear) * (0.93 + 0.07 * live),
        );
        if viz {
            let palette = crate::viz::harmonize(self.viz_palette());
            let area = ui.clip_rect();
            let intensity = 0.55 + 0.45 * live;
            if crate::shader::available() {
                // Vagues par la carte graphique : une grande zone qui s'estompe longuement vers le
                // haut (derrière le contenu), les vagues restant sous la pochette.
                let band =
                    Rect::from_min_max(pos2(area.left(), area.bottom() - area.height() * 0.62), area.max);
                let mut w = crate::viz::waves(&self.spectrum, palette, intensity);
                // Vagues sous le contenu (la liste « À suivre » reste lisible).
                w.base = 0.84;
                w.amp = 0.42;
                w.fade_top = 0.55;
                w.fade_bottom = 0.10;
                crate::shader::paint(ui.painter(), band, w);
                let low = Rect::from_min_max(pos2(band.left(), band.top() + band.height() * 0.45), band.max);
                crate::viz::particles(ui.painter(), low, &self.spectrum, palette[1], intensity);
            } else {
                // Repli (pilote sans shader) : l'Aurore calculée par le processeur.
                let top = (cover.bottom() - cover.height() * 0.08).max(area.bottom() - area.height() * 0.42);
                let band = Rect::from_min_max(pos2(area.left(), top), area.max);
                self.aurora
                    .paint(ui.painter(), band, &self.spectrum, palette, intensity);
            }
            crate::viz::halo(ui.painter(), cover, &self.spectrum, palette[0], intensity);
        }
        let tex = self.covers.large(&t.path).and_then(|l| l.cover.clone());
        ui.painter().add(
            egui::Shadow {
                offset: [0, 18],
                blur: 48,
                spread: 0,
                color: Color32::from_black_alpha(150),
            }
            .as_shape(cover, CornerRadius::same(14)),
        );
        widgets::cover(ui.painter(), cover, tex.as_ref(), 14, &t.album);

        let x = cover.right() + 40.0;
        let tw = rect.right() - x - 24.0;
        let mut y = cover.top() + 16.0;
        let p = ui.painter();
        widgets::equalizer(p, pos2(x + 7.0, y + 7.0), 13.0, theme::accent_light(), playing);
        p.text(
            pos2(x + 22.0, y),
            Align2::LEFT_TOP,
            if playing {
                "EN COURS DE LECTURE"
            } else {
                "EN PAUSE"
            },
            FontId::proportional(12.0),
            theme::accent_light(),
        );
        y += 22.0;
        let mut job = egui::text::LayoutJob::single_section(
            t.title.clone(),
            egui::TextFormat::simple(theme::semibold(36.0), theme::TEXT),
        );
        job.wrap = egui::text::TextWrapping {
            max_width: tw,
            max_rows: 2,
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        let g = ui.fonts(|f| f.layout_job(job));
        let h = g.size().y;
        ui.painter().galley(pos2(x, y), g, theme::TEXT);
        y += h + 8.0;
        for (text, size, color, target) in [
            (
                t.artist_or_unknown().to_owned(),
                18.0,
                theme::TEXT,
                View::Artist(artist),
            ),
            (
                t.album_or_unknown().to_owned(),
                15.0,
                theme::TEXT_2,
                View::Album(album),
            ),
        ] {
            let g = widgets::galley(ui, &text, FontId::proportional(size), color, tw);
            let r = Rect::from_min_size(pos2(x, y), g.size());
            let resp = ui.interact(r, ui.id().with(("np_link", size as i32)), Sense::click());
            let col = if resp.hovered() {
                theme::accent_light()
            } else {
                color
            };
            ui.painter().galley_with_override_text_color(r.min, g, col);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.actions.push(Action::Go(target));
            }
            y += size + 10.0;
        }
        let mut details = Vec::new();
        if let Some(year) = t.year {
            details.push(year.to_string());
        }
        if !t.genre.is_empty() {
            details.push(t.genre.clone());
        }
        details.push(fmt_duration(t.duration()));
        if let Some(ext) = t.path.extension() {
            details.push(ext.to_string_lossy().to_uppercase());
        }
        ui.painter().text(
            pos2(x, y),
            Align2::LEFT_TOP,
            details.join(" · "),
            FontId::proportional(13.0),
            theme::TEXT_3,
        );
        y += 34.0;

        if !upcoming.is_empty() {
            ui.painter().text(
                pos2(x, y),
                Align2::LEFT_TOP,
                "À SUIVRE",
                FontId::proportional(12.0),
                theme::TEXT_3,
            );
            y += 22.0;
            let pos = self.queue_pos.unwrap_or(0);
            for (k, &u) in upcoming.iter().enumerate() {
                let r = Rect::from_min_size(pos2(x - 8.0, y), vec2(tw.min(460.0), 44.0));
                if r.bottom() > rect.bottom() - 8.0 {
                    break;
                }
                let resp = ui.interact(r, ui.id().with(("np_next", k)), Sense::click());
                widgets::paint_interaction(ui, r, 6, &resp);
                let ut = &self.lib.tracks[u];
                let tex = self.covers.thumb(&ut.path).cloned();
                let cr = Rect::from_min_size(r.min + vec2(8.0, 5.0), vec2(34.0, 34.0));
                widgets::cover(ui.painter(), cr, tex.as_ref(), 4, &ut.album);
                let g = widgets::galley(
                    ui,
                    &ut.title,
                    FontId::proportional(14.0),
                    theme::TEXT,
                    r.width() - 64.0,
                );
                ui.painter()
                    .galley(pos2(cr.right() + 12.0, r.top() + 5.0), g, theme::TEXT);
                let g = widgets::galley(
                    ui,
                    ut.artist_or_unknown(),
                    FontId::proportional(12.5),
                    theme::TEXT_2,
                    r.width() - 64.0,
                );
                ui.painter()
                    .galley(pos2(cr.right() + 12.0, r.top() + 24.0), g, theme::TEXT_2);
                if resp.double_clicked() || resp.clicked() {
                    self.actions.push(Action::QueueJump(pos + 1 + k));
                }
                y += 46.0;
            }
        }
    }

    // ---------- Paramètres ----------

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Paramètres");
        ui.add_space(16.0);
        egui::ScrollArea::vertical().id_salt("settings").auto_shrink(false).show(ui, |ui| {
            ui.set_width((ui.available_width() - 16.0).min(900.0));

            subtitle(ui, "Bibliothèque");
            card(ui, |ui| {
                card_row(ui, Icon::FolderOpen, "Dossiers de musique", "SterMusic parcourt ces dossiers et leurs sous-dossiers.", |s, ui| {
                    if widgets::secondary_button(ui, Some(Icon::Refresh), "Rescanner").clicked() {
                        s.actions.push(Action::Rescan);
                    }
                    if widgets::secondary_button(ui, Some(Icon::FolderPlus), "Ajouter un dossier").clicked() {
                        s.actions.push(Action::AddFolder);
                    }
                }, self);
                for (i, dir) in self.settings.music_dirs.clone().iter().enumerate() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        icons::paint_centered(ui.painter(), Icon::Music, r.center(), 14.0, theme::TEXT_3);
                        let max = ui.available_width() - 44.0;
                        let g = widgets::galley(ui, &dir.display().to_string(), FontId::proportional(14.0), theme::TEXT_2, max);
                        ui.label(g).on_hover_text(dir.display().to_string());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if widgets::icon_button(ui, Icon::Close, "Retirer ce dossier", 28.0, 14.0).clicked() {
                                self.actions.push(Action::RemoveFolder(i));
                            }
                        });
                    });
                }
                if self.settings.music_dirs.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        ui.label(RichText::new("Aucun dossier pour l'instant.").color(theme::TEXT_3));
                    });
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    let status = if self.scan_rx.is_some() {
                        format!("Analyse en cours… {}", plural(self.scan_count, "morceau", "morceaux"))
                    } else {
                        format!(
                            "{} · {} · {}",
                            plural(self.lib.tracks.len(), "morceau", "morceaux"),
                            plural(self.lib.albums.len(), "album", "albums"),
                            plural(self.lib.artists.len(), "artiste", "artistes")
                        )
                    };
                    ui.label(RichText::new(status).small().color(theme::TEXT_3));
                });
            });

            subtitle(ui, "Apparence");
            card(ui, |ui| {
                let mut breeze = self.settings.breeze;
                card_row(ui, Icon::Sparkles, "Style Breeze (Kubuntu)", "Police Noto Sans, icônes Breeze du système, boutons de fenêtre et sélection de KDE.", |_, ui| {
                    widgets::switch(ui, &mut breeze);
                }, self);
                if breeze != self.settings.breeze {
                    self.actions.push(Action::SetBreeze(breeze));
                }
                separator(ui);
                let mut rounded = self.settings.rounded;
                card_row(ui, Icon::WinMaximize, "Coins arrondis", "Arrondit les coins de la fenêtre (sauf quand elle est agrandie).", |_, ui| {
                    widgets::switch(ui, &mut rounded);
                }, self);
                self.settings.rounded = rounded;
                separator(ui);

                // Fond de la fenêtre : trois choix, en boutons segmentés.
                card_row(ui, Icon::Palette, "Fond de la fenêtre", "Mica : ton fond d'écran flouté et teinté. Transparent : le flou de KWin.", |s, ui| {
                    let options = [(BackdropKind::Transparent, "Transparent"), (BackdropKind::Mica, "Mica"), (BackdropKind::Solid, "Uni")];
                    for (kind, label) in options {
                        if segment(ui, label, s.settings.backdrop == kind).clicked() {
                            s.settings.backdrop = kind;
                        }
                    }
                }, self);
                if self.settings.backdrop != BackdropKind::Solid {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        let label = if self.settings.backdrop == BackdropKind::Mica { "Intensité" } else { "Opacité" };
                        ui.add_sized([80.0, 20.0], egui::Label::new(RichText::new(label).color(theme::TEXT_2)));
                        let seek = widgets::seek_bar(ui, "mica_strength", self.settings.mica_strength, 220.0, true);
                        if let Some(v) = seek.preview.or(seek.commit) {
                            self.settings.mica_strength = v;
                        }
                        ui.label(RichText::new(format!("{:.0} %", self.settings.mica_strength * 100.0)).small().color(theme::TEXT_3));
                    });
                }
                if self.settings.backdrop == BackdropKind::Mica {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        let (text, color) = if self.kwin.active() {
                            ("Le Mica suit la fenêtre et le fond d'écran de chaque écran (script KWin actif).", theme::accent_light())
                        } else {
                            ("Le Mica ne suit pas la position de la fenêtre (KWin indisponible) : fond d'écran de l'écran principal.", theme::TEXT_3)
                        };
                        ui.label(RichText::new(text).small().color(color));
                    });
                }
                if self.settings.backdrop == BackdropKind::Transparent {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("Pour flouter l'arrière-plan, KWin doit connaître SterMusic : avec l'effet Better Blur (forceblur), ce bouton ajoute « stermusic » à sa liste de fenêtres.")
                                    .small()
                                    .color(theme::TEXT_3),
                            );
                            ui.add_space(4.0);
                            if widgets::secondary_button(ui, Some(Icon::Sparkles), "Activer le flou de KWin pour SterMusic").clicked() {
                                self.actions.push(Action::EnableKwinBlur);
                            }
                        });
                    });
                }
            });

            subtitle(ui, "Couleurs");
            card(ui, |ui| {
                let desc = if theme::kde_theme().is_some() { "« KDE » reprend les couleurs et l'accent de ton bureau ; les autres sont ceux de SterCode." } else { "Les seize thèmes de SterCode." };
                card_row(ui, Icon::Palette, "Thème", desc, |_, _| {}, self);
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(52.0);
                    ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                    for t in theme::kde_theme().into_iter().chain(theme::THEMES.iter()) {
                        let selected = self.settings.theme == t.id;
                        let (r, resp) = ui.allocate_exact_size(vec2(84.0, 64.0), Sense::click());
                        widgets::paint_interaction(ui, r, 8, &resp);
                        let sw = Rect::from_center_size(pos2(r.center().x, r.top() + 22.0), vec2(56.0, 30.0));
                        ui.painter().rect_filled(sw, CornerRadius::same(8), t.bg);
                        let dot = |k: f32, col: Color32| ui.painter().circle_filled(pos2(sw.left() + 18.0 + k * 20.0, sw.center().y), 8.0, col);
                        dot(0.0, t.accent);
                        dot(1.0, t.accent_2);
                        let stroke = if selected { Stroke::new(2.0_f32, t.accent_light) } else { Stroke::new(1.0_f32, theme::stroke_strong()) };
                        ui.painter().rect_stroke(sw, CornerRadius::same(8), stroke, StrokeKind::Outside);
                        let name = if t.id == theme::KDE_ID { "KDE" } else { t.name };
                        ui.painter().text(pos2(r.center().x, r.bottom() - 10.0), Align2::CENTER_CENTER, name, FontId::proportional(12.0), if selected { theme::TEXT } else { theme::TEXT_2 });
                        if resp.clicked() {
                            self.actions.push(Action::SetTheme(t.id));
                        }
                    }
                });
            });

            subtitle(ui, "Son");
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.vertical(|ui| self.equalizer_panel(ui));
                });
                separator(ui);
                let mut crossfade = self.settings.crossfade;
                card_row(ui, Icon::Shuffle, "Fondu enchaîné", "Le morceau suivant commence en fondu pendant que le précédent s'efface.", |_, ui| {
                    widgets::switch(ui, &mut crossfade);
                }, self);
                self.settings.crossfade = crossfade;
                if crossfade {
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        ui.add_sized([80.0, 20.0], egui::Label::new(RichText::new("Durée").color(theme::TEXT_2)));
                        let value = (self.settings.crossfade_secs - 1.0) / 11.0;
                        let seek = widgets::seek_bar(ui, "crossfade_secs", value, 220.0, true);
                        if let Some(v) = seek.preview.or(seek.commit) {
                            self.settings.crossfade_secs = (1.0 + v * 11.0).round();
                        }
                        ui.label(RichText::new(format!("{:.0} s", self.settings.crossfade_secs)).small().color(theme::TEXT_3));
                    });
                }
                separator(ui);
                let mut visualizer = self.settings.visualizer;
                card_row(ui, Icon::AudioLines, "Visualiseur", "Lumières qui suivent la musique, aux couleurs de la pochette (En cours de lecture, accueil).", |_, ui| {
                    widgets::switch(ui, &mut visualizer);
                }, self);
                self.settings.visualizer = visualizer;
                separator(ui);
                let mut leveling = self.settings.leveling;
                card_row(ui, Icon::Volume2, "Nivellement du volume", "Joue les morceaux à un volume semblable : gain ReplayGain du fichier s'il existe, sinon réglage automatique.", |_, ui| {
                    widgets::switch(ui, &mut leveling);
                }, self);
                if leveling != self.settings.leveling {
                    self.settings.leveling = leveling;
                    self.push_audio();
                }
            });

            subtitle(ui, "Lecture");
            card(ui, |ui| {
                let mut resume = self.settings.resume;
                card_row(ui, Icon::Clock, "Reprendre là où je me suis arrêté", "Au lancement, la file d'attente et la position sont restaurées (en pause).", |_, ui| {
                    widgets::switch(ui, &mut resume);
                }, self);
                self.settings.resume = resume;
                separator(ui);
                let (state, desc) = if self.mpris.available() {
                    ("Actif", "Touches multimédia du clavier, widget lecteur du bureau et écran de verrouillage (MPRIS).")
                } else {
                    ("Indisponible", "Le bus de session D-Bus n'a pas répondu : les touches multimédia ne piloteront pas SterMusic.")
                };
                card_row(ui, Icon::AudioLines, "Intégration au bureau", desc, |_, ui| {
                    ui.label(RichText::new(state).color(if state == "Actif" { theme::accent_light() } else { theme::TEXT_3 }));
                }, self);
            });

            subtitle(ui, "Raccourcis clavier");
            card(ui, |ui| {
                for (keys, what) in [
                    ("Espace", "Lecture / pause"),
                    ("Ctrl + → / ←", "Morceau suivant / précédent"),
                    ("→ / ←", "Avancer / reculer de 5 secondes"),
                    ("Ctrl + ↑ / ↓", "Monter / baisser le volume"),
                    ("Ctrl + F", "Rechercher"),
                    ("Ctrl + M", "Mini lecteur"),
                    ("Échap", "Retour"),
                ] {
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        ui.add_sized([160.0, 22.0], egui::Label::new(RichText::new(keys).color(theme::TEXT).monospace()));
                        ui.label(RichText::new(what).color(theme::TEXT_2));
                    });
                }
            });

            subtitle(ui, "À propos");
            card(ui, |ui| {
                card_row(ui, Icon::Info, &format!("SterMusic {}", env!("CARGO_PKG_VERSION")), "Lecteur de musique pour Linux, écrit en Rust (egui, rodio, Symphonia, lofty). Icônes Lucide (ISC).", |_, _| {}, self);
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    ui.label(RichText::new(format!("Données : {}", crate::library::data_dir().display())).small().color(theme::TEXT_3));
                });
            });
            ui.add_space(32.0);
        });
    }
}

fn subtitle(ui: &mut egui::Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).font(theme::semibold(15.0)).color(theme::TEXT));
    ui.add_space(4.0);
}

/// Carte de réglages (calque translucide arrondi, comme les paramètres de Windows 11).
fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(theme::white(0x08))
        .stroke(Stroke::new(1.0_f32, theme::white(0x0E)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    ui.add_space(6.0);
}

fn separator(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, theme::stroke()));
    ui.add_space(6.0);
}

/// Ligne de réglage : icône, titre, description à gauche ; contrôles à droite.
fn card_row(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    desc: &str,
    right: impl FnOnce(&mut SterMusic, &mut egui::Ui),
    app: &mut SterMusic,
) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(36.0, 40.0), Sense::hover());
        icons::paint_centered(ui.painter(), icon, r.center(), 20.0, theme::TEXT_2);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(title).color(theme::TEXT));
            ui.label(RichText::new(desc).small().color(theme::TEXT_2));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| right(app, ui));
    });
}

/// Halo radial discret dans le bandeau d'accueil.
fn theme_glow(painter: &egui::Painter, rect: Rect) {
    let center = pos2(
        rect.right() - rect.width() * 0.18,
        rect.top() + rect.height() * 0.2,
    );
    widgets::soft_glow(
        painter,
        rect,
        center,
        rect.height() * 1.1,
        theme::alpha(theme::accent_2(), 0x40),
    );
}

/// Bouton segmenté (choix exclusif), sélection à l'accent.
fn segment(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), FontId::proportional(13.5), theme::TEXT));
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 26.0, 30.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, CornerRadius::same(6), theme::alpha(theme::accent(), 0x55));
        painter.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0_f32, theme::accent_light()),
            StrokeKind::Inside,
        );
    } else {
        painter.rect_filled(rect, CornerRadius::same(6), theme::white(0x0C));
        painter.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0_f32, theme::stroke()),
            StrokeKind::Inside,
        );
    }
    widgets::paint_interaction(ui, rect, 6, &response);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        if selected { theme::TEXT } else { theme::TEXT_2 },
    );
    response
}
