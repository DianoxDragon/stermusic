//! Pages : accueil, bibliothèque, album, artiste, file d'attente, playlists, recherche,
//! en cours de lecture, paramètres.

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, UiBuilder,
};

use super::lists::{self, ListOpts};
use super::{Action, BackdropKind, LibTab, Page, PlaylistId, SongSort, SterMusic, View};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::{fmt_duration, fmt_long, plural};
use crate::trf;
use crate::{anim, theme, widgets};

const PAD_X: f32 = 32.0;

/// Textes de l'en-tête d'une page de détail (album, artiste, playlist).
pub(super) struct Header<'a> {
    pub kind: &'a str,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub info: &'a str,
}

/// Tri de la page Playlists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum PlSort {
    /// Favoris, intelligentes, puis les miennes dans l'ordre de leur création.
    #[default]
    Default,
    Name,
    Count,
    Duration,
}

pub(super) fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::semibold(28.0)).color(theme::TEXT));
}

fn meta(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::TEXT_2));
}

impl SterMusic {
    /// Un visualiseur dans « En cours de lecture » : toute la page (« plein écran »), ou une bande
    /// sous le contenu avec un long fondu vers le haut.
    fn paint_np_style(
        &self,
        ui: &egui::Ui,
        area: Rect,
        palette: [Color32; 3],
        intensity: f32,
        style: usize,
        cover: (Rect, Option<egui::TextureId>),
    ) {
        let fill = crate::shader::STYLES[style].1;
        let mut w = crate::viz::waves(&self.spectrum, palette, intensity * self.settings.viz_intensity);
        w.mode = style as i32;
        let rect = if fill {
            w.intensity *= 0.6;
            w.fade_top = 0.08;
            w.fade_bottom = 0.08;
            area
        } else {
            w.base = if style == 3 || style == 34 { 0.88 } else { 0.84 };
            w.amp = if style == 0 { 0.42 } else { 0.8 };
            w.fade_top = 0.55;
            w.fade_bottom = 0.10;
            Rect::from_min_max(pos2(area.left(), area.bottom() - area.height() * 0.62), area.max)
        };
        set_cover(&mut w, rect, cover);
        self.viz_tune(style).apply(&mut w, style, rect.size());
        crate::shader::paint(ui.painter(), rect, w);
    }

    /// Un visualiseur dans le bandeau de l'accueil : discret, zoomé (motifs larges et calmes),
    /// découpé aux coins arrondis.
    fn paint_banner_style(
        &self,
        ui: &egui::Ui,
        rect: Rect,
        palette: [Color32; 3],
        alpha: f32,
        style: usize,
        cover: (Rect, Option<egui::TextureId>),
    ) {
        let fill = crate::shader::STYLES[style].1;
        let mut w = crate::viz::waves(
            &self.spectrum,
            palette,
            if fill { 0.5 } else { 0.7 } * alpha * self.settings.viz_intensity,
        );
        w.mode = style as i32;
        w.scale = 3.0;
        w.base = 0.74;
        w.amp = 0.5;
        w.fade_top = if fill { 0.04 } else { 0.2 };
        w.fade_bottom = if fill { 0.04 } else { 0.12 };
        w.left = if fill { 0.0 } else { 1.0 };
        w.radius = 14.0;
        set_cover(&mut w, rect, cover);
        self.viz_tune(style).apply(&mut w, style, rect.size());
        crate::shader::paint(ui.painter(), rect, w);
    }

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
                RichText::new(tr("Égaliseur"))
                    .font(theme::semibold(15.0))
                    .color(theme::TEXT),
            );
            if self.settings.bit_perfect {
                ui.label(RichText::new(tr("(sans effet en sortie bit-perfect)")).small().color(theme::TEXT_3));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_button(ui, Icon::Refresh, tr("Tout remettre à 0"), 30.0, 15.0).clicked() {
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
                if segment(ui, tr(name), self.settings.eq_preset == name).clicked() {
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
                ui.multiply_opacity(0.45);
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
                if column(ui, 99, &mut pre, tr("Préampli")) {
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
    pub(super) fn play_all_button(&mut self, ui: &mut egui::Ui, tracks: &[usize]) {
        let loaded = self.player.as_ref().is_some_and(|p| p.has_track());
        let here = loaded && self.current().is_some_and(|c| tracks.contains(&c));
        let playing = self.is_playing();
        let (icon, label) = match (here, playing) {
            (true, true) => (Icon::Pause, tr("Pause")),
            (true, false) => (Icon::Play, tr("Reprendre")),
            _ => (Icon::Play, tr("Tout lire")),
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
    pub(super) fn detail_search_field(&mut self, ui: &mut egui::Ui, hint: &str) {
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

    /// En-tête d'une liste dont les colonnes trient (tri gardé par vue, le temps de la session),
    /// et la liste triée en conséquence.
    pub(super) fn sortable_header(&mut self, ui: &mut egui::Ui, opts: &ListOpts, view: &str, list: &mut [usize]) {
        let spec = self.list_sorts.entry(view.to_owned()).or_default();
        lists::header_sortable(ui, opts, Some(spec));
        spec.sort(&self.lib, list);
    }

    /// Les morceaux de `tracks` qui correspondent à la recherche de la page.
    pub(super) fn filtered(&self, tracks: &[usize]) -> Vec<usize> {
        tracks
            .iter()
            .copied()
            .filter(|&t| self.matches_filter(t))
            .collect()
    }

    pub(super) fn no_match(ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.label(RichText::new(tr("Rien ne correspond à la recherche.")).color(theme::TEXT_3));
    }

    fn matches_filter(&self, t: usize) -> bool {
        let q = crate::library::fold(self.detail_filter.trim());
        q.is_empty() || self.lib.track_matches(t, &q)
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
        // Le globe des webradios occupe toute la zone, d'un bord à l'autre.
        if view == View::Page(Page::Radio) && self.settings.radio_globe {
            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                ui.multiply_opacity(t);
                self.globe_view(ui, rect);
            });
            return;
        }
        let inner = Rect::from_min_max(rect.min + vec2(PAD_X, 24.0), rect.max - vec2(PAD_X * 0.5, 0.0))
            .translate(vec2(0.0, 18.0 * (1.0 - t)));
        ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
            ui.multiply_opacity(t);
            match view {
                View::Page(Page::Home) => self.home(ui),
                View::Page(Page::Library) => self.library_page(ui),
                View::Page(Page::Queue) => self.queue_page(ui),
                View::Page(Page::Playlists) => self.playlists_page(ui),
                View::Page(Page::Settings) => self.settings_page(ui),
                View::Page(Page::Tags) => self.tags_page(ui),
                View::Page(Page::Stats) => self.stats_page(ui),
                View::Page(Page::Radio) => self.radio_page(ui),
                View::Page(Page::Podcasts) => self.podcasts_page(ui),
                View::Page(Page::Tools) => self.tools_page(ui),
                View::Podcast(feed) => self.podcast_page(ui, &feed),
                View::Browse(kind, name) => self.browse_page(ui, kind, &name),
                View::Recap(year) => self.recap_page(ui, year),
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
                    &trf!(
                        "Analyse de la bibliothèque… {0}",
                        plural(self.scan_count, "morceau", "morceaux")
                    ),
                );
            });
            return;
        }
        widgets::empty_state(
            ui,
            Icon::Music,
            tr("Bienvenue dans SterMusic"),
            tr("Ajoute un dossier de musique pour construire ta bibliothèque."),
        );
        ui.vertical_centered(|ui| {
            let default = crate::library::default_music_dir();
            ui.horizontal(|ui| {
                let w = 190.0 + if default.is_some() { 230.0 } else { 0.0 };
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                if widgets::primary_button(ui, Some(Icon::FolderPlus), tr("Ajouter un dossier")).clicked() {
                    self.actions.push(Action::AddFolder);
                }
                if let Some(dir) = default {
                    let name = dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if widgets::secondary_button(ui, Some(Icon::Music), &trf!("Utiliser « {name} »", name))
                        .clicked()
                    {
                        self.actions.push(Action::UseFolder(dir));
                    }
                }
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(tr("Astuce : tu peux aussi glisser un dossier dans la fenêtre."))
                    .small()
                    .color(theme::TEXT_3),
            );
        });
    }

    // ---------- Accueil ----------

    fn home(&mut self, ui: &mut egui::Ui) {
        heading(ui, tr("Accueil"));
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
                    lists::section(ui, tr("Écoutés récemment"), None);
                    lists::grid(ui, "recent", recent_albums.len(), Some(1), |ui, k, w| {
                        lists::album_card(ui, &mut c, recent_albums[k], w)
                    });
                    ui.add_space(20.0);
                }
                if lists::section(ui, tr("Ajoutés récemment"), Some(tr("Tous les albums"))) {
                    c.actions.push(Action::Go(View::Page(Page::Library)));
                }
                lists::grid(ui, "added", added.len(), Some(1), |ui, k, w| {
                    lists::album_card(ui, &mut c, added[k], w)
                });
                if !top.is_empty() {
                    ui.add_space(20.0);
                    lists::section(ui, tr("Les plus écoutés"), None);
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
                // Visualiseur du bandeau (Caustiques par défaut : des reflets d'eau sur toute la
                // largeur), discret et découpé aux coins arrondis.
                let setting = self.settings.banner_style.min(crate::shader::AUTO);
                let now = ui.input(|i| i.time);
                let allowed = self.auto_allowed();
                let (style, fading) =
                    self.auto_banner
                        .pick(now, self.current(), setting, &allowed, self.settings.viz_auto_secs);
                if fading.is_some() {
                    ui.ctx().request_repaint();
                }
                let side = rect.height() - 48.0;
                let banner_cover = Rect::from_min_size(rect.min + vec2(24.0, 24.0), vec2(side, side));
                let blur = self.current().and_then(|i| {
                    let path = self.lib.tracks[i].path.clone();
                    self.covers
                        .large(&path)
                        .and_then(|l| l.blurred.as_ref().map(|t| t.id()))
                });
                if let Some((old, t)) = fading {
                    self.paint_banner_style(ui, rect, palette, 1.0 - t, old, (banner_cover, blur));
                }
                self.paint_banner_style(
                    ui,
                    rect,
                    palette,
                    fading.map(|(_, t)| t).unwrap_or(1.0),
                    style,
                    (banner_cover, blur),
                );
                if self.settings.viz_particles {
                    crate::viz::particles(
                        ui.painter(),
                        rect.shrink(6.0),
                        &self.spectrum,
                        palette[1],
                        0.6,
                        18,
                    );
                }
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
                    tr("EN COURS DE LECTURE")
                } else {
                    tr("REPRENDRE LA LECTURE")
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
                    tr("TA BIBLIOTHÈQUE"),
                    trf!(
                        "{0} à écouter",
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
                        if playing { tr("Pause") } else { tr("Reprendre") },
                    )
                    .clicked()
                    {
                        self.actions.push(Action::TogglePlay);
                    }
                    if widgets::secondary_button(ui, Some(Icon::Maximize), tr("En cours de lecture"))
                        .clicked()
                    {
                        self.actions.push(Action::Go(View::NowPlaying));
                    }
                } else if widgets::primary_button(ui, Some(Icon::Shuffle), tr("Tout lire en aléatoire"))
                    .clicked()
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
            heading(ui, tr("Bibliothèque musicale"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if !self.lib.is_empty() {
                    let all: Vec<usize> = (0..self.lib.tracks.len()).collect();
                    self.shuffle_split(ui, &all, tr("Tout lire en aléatoire"));
                }
                if self.scan_rx.is_some() {
                    ui.spinner();
                    ui.label(
                        RichText::new(trf!("Analyse… {0}", self.scan_count))
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
            (LibTab::Songs, tr("Morceaux")),
            (LibTab::Albums, tr("Albums")),
            (LibTab::Artists, tr("Artistes")),
            (LibTab::Folders, tr("Dossiers")),
            (LibTab::Browse, tr("Parcourir")),
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
                    let s = anim::toggle(ui.ctx(), resp.id.with("sel"), selected, anim::NORMAL);
                    let h = widgets::hover_t(ui, resp.id, resp.hovered());
                    let color = theme::mix(theme::TEXT_2, theme::TEXT, s.max(h * 0.6));
                    ui.painter()
                        .galley(r.center() - galley.size() / 2.0, galley, color);
                    if resp.clicked() {
                        self.settings.lib_tab = tab;
                    }
                    if selected {
                        target = Some(r);
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(12.0);
                    if self.settings.lib_tab == LibTab::Songs {
                        let sorts = [
                            (SongSort::Title, tr("Titre")),
                            (SongSort::Artist, tr("Artiste")),
                            (SongSort::Album, tr("Album")),
                            (SongSort::Added, tr("Date d'ajout")),
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
                                    // Choisir un ordre ici efface le tri par colonnes.
                                    if ui.selectable_value(&mut self.settings.song_sort, s, l).clicked() {
                                        self.settings.song_columns.0.clear();
                                    }
                                }
                            });
                        ui.label(RichText::new(tr("Trier par")).color(theme::TEXT_3));
                        ui.add_space(12.0);
                    }
                    let hint = match self.settings.lib_tab {
                        LibTab::Songs => tr("Filtrer : titre, artiste, album, genre, année…"),
                        LibTab::Albums => tr("Filtrer les albums"),
                        LibTab::Artists => tr("Filtrer les artistes"),
                        LibTab::Folders => tr("Filtrer ce dossier"),
                        LibTab::Browse => match self.settings.browse_kind {
                            super::browse::BrowseKind::Genre => tr("Filtrer les genres"),
                            super::browse::BrowseKind::Year => tr("Filtrer les années"),
                            super::browse::BrowseKind::Composer => tr("Filtrer les compositeurs"),
                        },
                    };
                    let w = (ui.available_width() - 8.0).clamp(140.0, 260.0);
                    widgets::search_field(ui, egui::Id::new("lib_filter"), &mut self.lib_filter, hint, w);
                });
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

        let tab_in = egui::Id::new("lib_tab_in");
        if anim::key_changed(ui.ctx(), tab_in, self.settings.lib_tab as u64) {
            anim::reset(ui.ctx(), tab_in);
            lists::restart_cascade(ui.ctx());
        }
        let t = anim::value_from(ui.ctx(), tab_in, 0.0, 1.0, anim::SLOW, anim::Easing::OutExpo);
        ui.multiply_opacity(t);

        let query = self.lib_filter.trim().to_owned();
        match self.settings.lib_tab {
            LibTab::Songs => {
                let songs = self.sorted_songs().to_vec();
                let mut songs = if query.is_empty() {
                    songs
                } else {
                    self.lib.filter_tracks(&songs, &query)
                };
                let opts = ListOpts {
                    cover: true,
                    album: true,
                    ..Default::default()
                };
                // Les titres des colonnes trient la liste, par-dessus l'ordre choisi dans « Trier par ».
                lists::header_sortable(ui, &opts, Some(&mut self.settings.song_columns));
                self.settings.song_columns.sort(&self.lib, &mut songs);
                if songs.is_empty() {
                    Self::no_match(ui);
                }
                let mut c = self.view_ctx();
                lists::track_list(ui, &mut c, "songs", &songs, &opts);
            }
            LibTab::Albums => {
                let albums: Vec<usize> = if query.is_empty() {
                    (0..self.lib.albums.len()).collect()
                } else {
                    self.lib.filter_albums(&query)
                };
                if albums.is_empty() {
                    Self::no_match(ui);
                }
                let mut c = self.view_ctx();
                lists::virtual_grid(ui, "albums", albums.len(), |ui, k, w| {
                    lists::album_card(ui, &mut c, albums[k], w)
                });
            }
            LibTab::Folders => self.folders_tab(ui, &query),
            LibTab::Browse => self.browse_tab(ui, &query),
            LibTab::Artists => {
                let artists: Vec<usize> = if query.is_empty() {
                    (0..self.lib.artists.len()).collect()
                } else {
                    self.lib.filter_artists(&query)
                };
                if artists.is_empty() {
                    Self::no_match(ui);
                }
                let mut c = self.view_ctx();
                lists::virtual_grid(ui, "artists", artists.len(), |ui, k, w| {
                    lists::artist_card(ui, &mut c, artists[k], w)
                });
            }
        }
    }

    // ---------- Album et artiste ----------

    /// En-tête de détail : grande image, titres et boutons d'action.
    pub(super) fn detail_header(
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
                        kind: tr("ALBUM"),
                        title: &album.title,
                        subtitle: &album.artist,
                        info: &info.join(" · "),
                    },
                    |s, ui| {
                        s.play_all_button(ui, &tracks);
                        s.shuffle_split(ui, &tracks, tr("Aléatoire"));
                        let resp = widgets::secondary_button(ui, Some(Icon::Plus), tr("Ajouter à"));
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
                                if widgets::menu_item(ui, Some(Icon::ListEnd), tr("File d'attente")).clicked()
                                {
                                    c.actions.push(Action::Enqueue(tracks.clone()));
                                }
                                if widgets::menu_item(ui, Some(Icon::Plus), tr("Nouvelle playlist…"))
                                    .clicked()
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
                        if widgets::secondary_button(ui, Some(Icon::Mic), tr("Artiste")).clicked() {
                            s.actions.push(Action::Go(View::Artist(artist)));
                        }
                        if widgets::icon_button(
                            ui,
                            Icon::Tag,
                            tr("Modifier les infos de l'album"),
                            34.0,
                            16.0,
                        )
                        .clicked()
                        {
                            let paths = tracks.iter().map(|&t| s.lib.tracks[t].path.clone()).collect();
                            s.actions.push(Action::EditTags(paths));
                        }
                        if tracks.len() >= 8 {
                            s.detail_search_field(ui, tr("Rechercher un titre"));
                        }
                    },
                );
                let mut shown = self.filtered(&album.tracks);
                if shown.is_empty() {
                    Self::no_match(ui);
                }
                let opts = ListOpts {
                    track_no: true,
                    ..Default::default()
                };
                self.sortable_header(ui, &opts, "album", &mut shown);
                let mut c = self.view_ctx();
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
                        kind: tr("ARTISTE"),
                        title: &artist.name,
                        subtitle: "",
                        info: &info,
                    },
                    |s, ui| {
                        s.play_all_button(ui, &tracks);
                        s.shuffle_split(ui, &tracks, tr("Aléatoire"));
                        if widgets::secondary_button(ui, Some(Icon::ListEnd), tr("Ajouter à la file"))
                            .clicked()
                        {
                            s.actions.push(Action::Enqueue(tracks.clone()));
                        }
                        s.detail_search_field(ui, tr("Rechercher chez cet artiste"));
                    },
                );
                // La recherche garde les albums dont le titre correspond, ou qui contiennent un titre trouvé.
                let shown = self.filtered(&artist.tracks);
                let query = crate::library::fold(self.detail_filter.trim());
                let albums: Vec<usize> = artist
                    .albums
                    .iter()
                    .copied()
                    .filter(|&a| {
                        query.is_empty()
                            || crate::library::fold(&self.lib.albums[a].title).contains(&query)
                            || self.lib.albums[a].tracks.iter().any(|t| shown.contains(t))
                    })
                    .collect();
                let mut c = self.view_ctx();
                if !albums.is_empty() {
                    lists::section(ui, tr("Albums"), None);
                    lists::grid(ui, "artist_albums", albums.len(), None, |ui, k, w| {
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

    // ---------- File d'attente ----------

    fn queue_page(&mut self, ui: &mut egui::Ui) {
        let total: std::time::Duration = self.queue.iter().map(|&t| self.lib.tracks[t].duration()).sum();
        ui.horizontal(|ui| {
            heading(ui, tr("File d'attente"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if !self.queue.is_empty() {
                    if widgets::secondary_button(ui, Some(Icon::Trash), tr("Effacer")).clicked() {
                        self.actions.push(Action::QueueClear);
                    }
                    if widgets::secondary_button(ui, Some(Icon::ListPlus), tr("Enregistrer comme playlist"))
                        .clicked()
                    {
                        self.actions.push(Action::SaveQueueAsPlaylist);
                    }
                    let w = (ui.available_width() - 16.0).clamp(160.0, 280.0);
                    widgets::search_field(
                        ui,
                        egui::Id::new("queue_search"),
                        &mut self.queue_filter,
                        tr("Rechercher dans la file"),
                        w,
                    );
                }
            });
        });
        if self.queue.is_empty() {
            widgets::empty_state(
                ui,
                Icon::ListMusic,
                tr("La file d'attente est vide"),
                tr("Lance un album, un artiste ou un morceau pour la remplir."),
            );
            return;
        }

        // Positions affichées : toute la file, ou celles qui correspondent à la recherche.
        let query = crate::library::fold(self.queue_filter.trim());
        let positions: Vec<usize> = if query.is_empty() {
            (0..self.queue.len()).collect()
        } else {
            (0..self.queue.len())
                .filter(|&p| {
                    let t = &self.lib.tracks[self.queue[p]];
                    let text = crate::library::fold(&format!("{} {} {}", t.title, t.artist, t.album));
                    query.split_whitespace().all(|w| text.contains(w))
                })
                .collect()
        };
        let filtered = !query.is_empty();
        if filtered {
            meta(
                ui,
                &trf!(
                    "{0} sur {1} · effacez la recherche pour réorganiser",
                    plural(positions.len(), "résultat", "résultats"),
                    self.queue.len()
                ),
            );
        } else {
            meta(
                ui,
                &trf!(
                    "{0} · {1} · glisser une ligne pour la déplacer",
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
    pub(super) fn mosaic(&mut self, ui: &egui::Ui, rect: Rect, tracks: &[usize], favorites: bool, radius: u8) {
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
            heading(ui, tr("Playlists"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if widgets::secondary_button(ui, Some(Icon::Download), tr("Importer (M3U)")).clicked() {
                    self.actions.push(Action::ImportPlaylist);
                }
                if widgets::secondary_button(ui, Some(Icon::Sparkles), tr("Playlist intelligente"))
                    .on_hover_text(tr("Une playlist qui se remplit toute seule d'après des règles"))
                    .clicked()
                {
                    self.actions.push(Action::NewSmartPlaylist);
                }
                if widgets::primary_button(ui, Some(Icon::Plus), tr("Nouvelle playlist")).clicked() {
                    self.actions.push(Action::NewPlaylist(Vec::new()));
                }
            });
        });
        ui.add_space(12.0);
        // Barre d'outils, comme dans la bibliothèque : affichage, tri, filtre.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if segment(ui, tr("Grille"), !self.settings.pl_list).clicked() {
                self.settings.pl_list = false;
            }
            if segment(ui, tr("Liste"), self.settings.pl_list).clicked() {
                self.settings.pl_list = true;
            }
            let pins = self.pinned().len();
            ui.add_space(10.0);
            ui.label(
                RichText::new(trf!("{0} épinglées sur {1}", pins, super::MAX_PINS))
                    .small()
                    .color(theme::TEXT_3),
            )
            .on_hover_text(tr("Les playlists épinglées (clic droit → Épingler) s'affichent dans la barre latérale."));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                let desc = self.settings.pl_sort_desc;
                let tip = if desc { tr("Ordre décroissant") } else { tr("Ordre croissant") };
                if self.settings.pl_sort != PlSort::Default
                    && widgets::icon_button(ui, if desc { Icon::Download } else { Icon::Upload }, tip, 30.0, 14.0).clicked()
                {
                    self.settings.pl_sort_desc = !desc;
                }
                let sorts = [
                    (PlSort::Default, tr("Ordre de création")),
                    (PlSort::Name, tr("Nom")),
                    (PlSort::Count, tr("Nombre de morceaux")),
                    (PlSort::Duration, tr("Durée")),
                ];
                let current = sorts.iter().find(|s| s.0 == self.settings.pl_sort).map(|s| s.1).unwrap_or("");
                egui::ComboBox::from_id_salt("pl_sort")
                    .selected_text(current)
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        for (s, l) in sorts {
                            ui.selectable_value(&mut self.settings.pl_sort, s, l);
                        }
                    });
                ui.label(RichText::new(tr("Trier par")).color(theme::TEXT_3));
                ui.add_space(12.0);
                let w = (ui.available_width() - 8.0).clamp(140.0, 260.0);
                widgets::search_field(ui, egui::Id::new("pl_filter"), &mut self.pl_filter, tr("Filtrer les playlists"), w);
            });
        });
        ui.add_space(14.0);

        // Toutes les playlists, avec ce qu'il faut pour les trier et les filtrer.
        let mut ids = vec![PlaylistId::Favorites];
        ids.extend((0..self.settings.smart_playlists.len()).map(PlaylistId::Smart));
        ids.extend((0..self.settings.playlists.len()).map(PlaylistId::User));
        struct Row {
            id: PlaylistId,
            name: String,
            tracks: Vec<usize>,
            duration: std::time::Duration,
        }
        let query = crate::library::fold(self.pl_filter.trim());
        let mut rows: Vec<Row> = ids
            .into_iter()
            .map(|id| {
                let tracks = self.playlist_tracks(id);
                Row {
                    id,
                    name: self.playlist_name(id),
                    duration: tracks.iter().map(|&t| self.lib.tracks[t].duration()).sum(),
                    tracks,
                }
            })
            .filter(|r| {
                let name = crate::library::fold(&r.name);
                query.split_whitespace().all(|w| name.contains(w))
            })
            .collect();
        match self.settings.pl_sort {
            PlSort::Default => {}
            PlSort::Name => rows.sort_by_cached_key(|r| crate::library::fold(&r.name)),
            PlSort::Count => rows.sort_by_key(|r| r.tracks.len()),
            PlSort::Duration => rows.sort_by_key(|r| r.duration),
        }
        if self.settings.pl_sort != PlSort::Default && self.settings.pl_sort_desc {
            rows.reverse();
        }
        if rows.is_empty() {
            Self::no_match(ui);
            return;
        }
        let list = self.settings.pl_list;
        egui::ScrollArea::vertical()
            .id_salt("playlists")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                if list {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for r in &rows {
                        self.playlist_row(ui, r.id, &r.name, &r.tracks, r.duration);
                    }
                    ui.add_space(16.0);
                    return;
                }
                let n = rows.len();
                let (cols, w) = lists::grid_layout(ui.available_width());
                for line in 0..n.div_ceil(cols) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = lists::GAP;
                        for r in &rows[line * cols..((line + 1) * cols).min(n)] {
                            self.playlist_card(ui, r.id, &r.name, &r.tracks, w);
                        }
                    });
                    ui.add_space(lists::GAP * 0.6);
                }
            });
    }

    /// Nature d'une playlist, en clair.
    fn playlist_kind(id: PlaylistId) -> &'static str {
        match id {
            PlaylistId::Favorites => tr("Favoris"),
            PlaylistId::Smart(_) => tr("Intelligente"),
            PlaylistId::User(_) => tr("Playlist"),
        }
    }

    /// Menu d'une playlist (clic droit sur sa carte, sa ligne ou son entrée de la barre latérale).
    pub(super) fn playlist_menu(&mut self, ui: &mut egui::Ui, id: PlaylistId, tracks: &[usize]) {
        if widgets::menu_item(ui, Some(Icon::PlayOutline), tr("Lire")).clicked() {
            self.actions.push(Action::Play {
                tracks: tracks.to_vec(),
                start: 0,
            });
        }
        if widgets::menu_item(ui, Some(Icon::Shuffle), tr("Lecture aléatoire")).clicked() {
            self.actions.push(Action::PlayShuffled(tracks.to_vec()));
        }
        ui.separator();
        let (icon, label) = if self.is_pinned(id) {
            (Icon::PinOff, tr("Désépingler de la barre latérale"))
        } else {
            (Icon::Pin, tr("Épingler dans la barre latérale"))
        };
        if widgets::menu_item(ui, Some(icon), label).clicked() {
            self.actions.push(Action::TogglePin(id));
        }
        if widgets::menu_item(ui, Some(Icon::Upload), tr("Exporter (M3U)")).clicked() {
            self.actions.push(Action::ExportPlaylist(id));
        }
        if let PlaylistId::Smart(i) = id {
            ui.separator();
            if widgets::menu_item(ui, Some(Icon::Sparkles), tr("Modifier les règles…")).clicked() {
                self.actions.push(Action::EditSmartPlaylist(i));
            }
        }
        if let PlaylistId::User(i) = id {
            ui.separator();
            if widgets::menu_item(ui, Some(Icon::Pencil), tr("Renommer")).clicked() {
                self.actions.push(Action::RenamePlaylist(i));
            }
            if widgets::menu_item(ui, Some(Icon::Trash), tr("Supprimer")).clicked() {
                self.actions.push(Action::DeletePlaylist(i));
            }
        }
    }

    /// Une playlist en ligne (affichage « Liste ») : mosaïque, nom, nature, morceaux, durée, épingle.
    fn playlist_row(&mut self, ui: &mut egui::Ui, id: PlaylistId, name: &str, tracks: &[usize], duration: std::time::Duration) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0), Sense::hover());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let resp = ui.interact(rect, ui.id().with(("pl_row", format!("{id:?}"))), Sense::click());
        widgets::paint_interaction(ui, rect, 8, &resp);
        let cover = Rect::from_min_size(pos2(rect.left() + 8.0, rect.top() + 6.0), vec2(48.0, 48.0));
        self.mosaic(ui, cover, tracks, id == PlaylistId::Favorites, 6);
        let x = cover.right() + 14.0;
        let right = rect.right() - 8.0;
        // Colonnes de droite : épingle, durée, nombre de morceaux, nature.
        let pin_r = Rect::from_center_size(pos2(right - 18.0, rect.center().y), vec2(32.0, 32.0));
        let pinned = self.is_pinned(id);
        let (icon, tip) = if pinned {
            (Icon::PinOff, tr("Désépingler de la barre latérale"))
        } else {
            (Icon::Pin, tr("Épingler dans la barre latérale"))
        };
        if (pinned || resp.hovered() || ui.rect_contains_pointer(pin_r))
            && ui
                .put(pin_r, |ui: &mut egui::Ui| widgets::toggle_icon(ui, pinned, icon, tip, 32.0, 15.0))
                .clicked()
        {
            self.actions.push(Action::TogglePin(id));
        }
        let p = ui.painter();
        let cy = rect.center().y;
        p.text(pos2(right - 60.0, cy), Align2::RIGHT_CENTER, fmt_long(duration), FontId::proportional(13.0), theme::TEXT_2);
        p.text(
            pos2(right - 190.0, cy),
            Align2::RIGHT_CENTER,
            plural(tracks.len(), "morceau", "morceaux"),
            FontId::proportional(13.0),
            theme::TEXT_2,
        );
        let wide = rect.width() > 760.0;
        if wide {
            p.text(pos2(right - 330.0, cy), Align2::RIGHT_CENTER, Self::playlist_kind(id), FontId::proportional(13.0), theme::TEXT_3);
        }
        let name_w = (right - if wide { 440.0 } else { 310.0 } - x).max(80.0);
        let g = widgets::galley(ui, name, theme::semibold(14.5), theme::TEXT, name_w);
        ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, theme::TEXT);
        if resp.clicked() {
            self.actions.push(Action::Go(View::Playlist(id)));
        }
        resp.context_menu(|ui| self.playlist_menu(ui, id, tracks));
    }

    fn playlist_card(&mut self, ui: &mut egui::Ui, id: PlaylistId, name: &str, tracks: &[usize], w: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(w, lists::card_height(w)), Sense::hover());
        let resp = ui.interact(rect, ui.id().with(("pl_card", format!("{id:?}"))), Sense::click());
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
        self.mosaic(ui, cover, tracks, id == PlaylistId::Favorites, 8);
        // Épinglée : une petite épingle dans le coin de la carte.
        if self.is_pinned(id) {
            let c = pos2(cover.right() - 18.0, cover.top() + 18.0);
            ui.painter().circle_filled(c, 13.0, Color32::from_black_alpha(150));
            icons::paint_centered(ui.painter(), Icon::Pin, c, 13.0, theme::accent_light());
        }
        let g = widgets::galley(ui, name, theme::semibold(14.0), theme::TEXT, w);
        ui.painter()
            .galley(pos2(rect.left(), cover.bottom() + 10.0 + 3.0 * h), g, theme::TEXT);
        let sub = if matches!(id, PlaylistId::Smart(_)) {
            format!("{} · {}", plural(tracks.len(), "morceau", "morceaux"), tr("intelligente"))
        } else {
            plural(tracks.len(), "morceau", "morceaux")
        };
        let g = widgets::galley(ui, &sub, FontId::proportional(13.0), theme::TEXT_2, w);
        ui.painter().galley(
            pos2(rect.left(), cover.bottom() + 31.0 + 3.0 * h),
            g,
            theme::TEXT_2,
        );
        if resp.clicked() {
            self.actions.push(Action::Go(View::Playlist(id)));
        }
        resp.context_menu(|ui| self.playlist_menu(ui, id, tracks));
    }

    fn playlist_page(&mut self, ui: &mut egui::Ui, id: PlaylistId) {
        match id {
            PlaylistId::User(i) if i >= self.settings.playlists.len() => return,
            PlaylistId::Smart(i) if i >= self.settings.smart_playlists.len() => return,
            _ => {}
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
                kind: if matches!(id, PlaylistId::Smart(_)) {
                    tr("PLAYLIST INTELLIGENTE")
                } else {
                    tr("PLAYLIST")
                },
                title: &name,
                subtitle: "",
                info: &info,
            },
            |s, ui| {
                ui.add_enabled_ui(!tracks.is_empty(), |ui| {
                    s.play_all_button(ui, &tracks);
                    s.shuffle_split(ui, &tracks, tr("Aléatoire"));
                });
                if let PlaylistId::User(i) = id {
                    if widgets::icon_button(ui, Icon::Pencil, tr("Renommer"), 34.0, 16.0).clicked() {
                        s.actions.push(Action::RenamePlaylist(i));
                    }
                }
                if let PlaylistId::Smart(i) = id {
                    if widgets::secondary_button(ui, Some(Icon::Sparkles), tr("Modifier les règles"))
                        .clicked()
                    {
                        s.actions.push(Action::EditSmartPlaylist(i));
                    }
                }
                if widgets::icon_button(ui, Icon::Upload, tr("Exporter (M3U)"), 34.0, 16.0).clicked() {
                    s.actions.push(Action::ExportPlaylist(id));
                }
                if let PlaylistId::User(i) = id {
                    if widgets::icon_button(ui, Icon::Trash, tr("Supprimer la playlist"), 34.0, 16.0)
                        .clicked()
                    {
                        s.actions.push(Action::DeletePlaylist(i));
                    }
                }
                let pinned = s.is_pinned(id);
                let (icon, tip) = if pinned {
                    (Icon::PinOff, tr("Désépingler de la barre latérale"))
                } else {
                    (Icon::Pin, tr("Épingler dans la barre latérale"))
                };
                if widgets::toggle_icon(ui, pinned, icon, tip, 34.0, 16.0).clicked() {
                    s.actions.push(Action::TogglePin(id));
                }
                if tracks.len() >= 8 {
                    s.detail_search_field(ui, tr("Filtrer cette playlist"));
                }
            },
        );
        if tracks.is_empty() {
            let (title, sub) = if id == PlaylistId::Favorites {
                (
                    tr("Aucun favori pour l'instant"),
                    tr("Clique sur le cœur d'un morceau pour l'ajouter ici."),
                )
            } else if matches!(id, PlaylistId::Smart(_)) {
                (
                    tr("Aucun morceau ne correspond pour l'instant"),
                    tr("Elle se remplira toute seule, ou modifie ses règles."),
                )
            } else {
                (
                    tr("Cette playlist est vide"),
                    tr("Clic droit sur un morceau ou un album → « Ajouter à une playlist »."),
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
            playlist: Some(id).filter(|id| !matches!(id, PlaylistId::Smart(_))),
            ..Default::default()
        };
        let mut shown = self.filtered(&tracks);
        self.sortable_header(ui, &opts, &format!("pl{id:?}"), &mut shown);
        if shown.is_empty() {
            Self::no_match(ui);
        }
        let mut c = self.view_ctx();
        lists::track_list(ui, &mut c, &format!("pl{id:?}"), &shown, &opts);
    }

    // ---------- Recherche ----------

    fn search_page(&mut self, ui: &mut egui::Ui) {
        heading(ui, &trf!("Résultats pour « {0} »", self.search.trim()));
        ui.add_space(12.0);
        let (tracks, albums, artists) = self.search_results.clone();
        if tracks.is_empty() && albums.is_empty() && artists.is_empty() {
            widgets::empty_state(
                ui,
                Icon::Search,
                tr("Aucun résultat"),
                tr("Essaie avec un autre titre, artiste ou album."),
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
                    lists::section(ui, tr("Artistes"), None);
                    lists::grid(ui, "found_artists", artists.len(), Some(1), |ui, k, w| {
                        lists::artist_card(ui, &mut c, artists[k], w)
                    });
                    ui.add_space(12.0);
                }
                if !albums.is_empty() {
                    lists::section(ui, tr("Albums"), None);
                    lists::grid(ui, "found_albums", albums.len(), Some(2), |ui, k, w| {
                        lists::album_card(ui, &mut c, albums[k], w)
                    });
                    ui.add_space(12.0);
                }
                if !tracks.is_empty() {
                    lists::section(ui, &trf!("Morceaux ({0})", tracks.len()), None);
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
                tr("Rien en lecture"),
                tr("Choisis un morceau dans ta bibliothèque."),
            );
            return;
        };
        // Le morceau affiché suit sa pochette : l'ancien reste à l'écran le temps que la nouvelle
        // soit chargée, puis pochette et textes changent ensemble, en fondu.
        let now = ui.input(|i| i.time);
        let current_path = self.lib.tracks[idx].path.clone();
        let loaded = self.covers.large(&current_path).map(|l| l.cover.clone());
        if self.ambience.cover.update(now, &current_path, loaded) {
            self.np_prev_text = self.np_track.and_then(|p| self.lib.tracks.get(p)).map(|p| {
                [p.title.clone(), p.artist_or_unknown().to_owned(), p.album_or_unknown().to_owned()]
            });
            self.np_track = Some(idx);
            self.np_dir = self.ambience.dir;
        }
        let real_idx = idx;
        let idx = self.np_track.filter(|&i| i < self.lib.tracks.len()).unwrap_or(idx);
        let fade = self.ambience.cover.t(now);
        if self.ambience.cover.fading(now) {
            ui.ctx().request_repaint();
        }
        let t = self.lib.tracks[idx].clone();
        let album = self.lib.track_album[idx];
        let artist = self.lib.album_artist[album];
        let rect = ui.max_rect();
        // Sur cette page, la molette règle le volume (voir `volume_wheel`).
        self.np_wheel = (Some(ui.clip_rect()), Vec::new());
        let upcoming: Vec<usize> = self
            .queue_pos
            .map(|p| self.queue.iter().skip(p + 1).take(5).copied().collect())
            .unwrap_or_default();
        // En plein écran, la pochette et les textes grandissent avec l'écran.
        let immersive = self.immersive();
        let max_side = if immersive { (rect.height() * 0.58).max(420.0) } else { 420.0 };
        let side = (rect.height() - 140.0)
            .min(rect.width() * 0.38)
            .clamp(160.0, max_side);
        let big = (side / 420.0).clamp(1.0, 1.7);
        let total_w = side + 40.0 + (rect.width() * 0.42).min(480.0 * big);
        let left = rect.center().x - total_w / 2.0;
        let cover = Rect::from_min_size(
            pos2(left, rect.top() + (rect.height() - side) / 2.0 - 30.0),
            vec2(side, side),
        );
        let appear = anim::appear(ui.ctx(), egui::Id::new("np_cover"), anim::SLOW);
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
                // Visualiseur choisi, ou automatique (au hasard) ; chaque changement en fondu.
                let setting = self.settings.viz_style.min(crate::shader::AUTO);
                let now = ui.input(|i| i.time);
                let allowed = self.auto_allowed();
                let (style, fading) =
                    self.auto_np
                        .pick(now, self.current(), setting, &allowed, self.settings.viz_auto_secs);
                let blur = self
                    .covers
                    .large(&current_path)
                    .and_then(|l| l.blurred.as_ref().map(|x| x.id()));
                if let Some((old, f)) = fading {
                    self.paint_np_style(ui, area, palette, intensity * (1.0 - f), old, (cover, blur));
                }
                let k = fading.map(|(_, f)| f).unwrap_or(1.0);
                self.paint_np_style(ui, area, palette, intensity * k, style, (cover, blur));
                if fading.is_some() {
                    ui.ctx().request_repaint();
                }
                if self.settings.viz_particles {
                    let low =
                        Rect::from_min_max(pos2(area.left(), area.bottom() - area.height() * 0.45), area.max);
                    crate::viz::particles(ui.painter(), low, &self.spectrum, palette[1], intensity, 64);
                }
                self.np_viz_controls(ui, area, style);
            } else {
                // Repli (pilote sans shader) : l'Aurore calculée par le processeur.
                let top = (cover.bottom() - cover.height() * 0.08).max(area.bottom() - area.height() * 0.42);
                let band = Rect::from_min_max(pos2(area.left(), top), area.max);
                self.aurora
                    .paint(ui.painter(), band, &self.spectrum, palette, intensity);
            }
            crate::viz::halo(ui.painter(), cover, &self.spectrum, palette[0], intensity);
        }
        ui.painter().add(
            egui::Shadow {
                offset: [0, 18],
                blur: 48,
                spread: 0,
                color: Color32::from_black_alpha(150),
            }
            .as_shape(cover, CornerRadius::same(14)),
        );
        // Changement de morceau : l'ancienne pochette glisse et s'efface, la nouvelle arrive de
        // l'autre côté (à droite pour le morceau suivant, à gauche pour le précédent).
        let dir = self.np_dir;
        let slide = cover.width() * 0.10;
        let cover_fade = &self.ambience.cover;
        if let Some(prev) = cover_fade.previous(now) {
            let mut p = ui.painter().clone();
            p.multiply_opacity(1.0 - fade);
            let old_seed = self.np_prev_text.as_ref().map(|s| s[2].as_str()).unwrap_or("");
            let r = cover.translate(vec2(-dir * slide * fade, 0.0));
            widgets::cover_now(&p, r.shrink(cover.width() * 0.02 * fade), prev, 14, old_seed);
        }
        {
            let mut p = ui.painter().clone();
            p.multiply_opacity(fade);
            let r = cover.translate(vec2(dir * slide * (1.0 - fade), 0.0));
            widgets::cover_now(&p, r, cover_fade.current(), 14, &t.album);
        }

        // Double-clic sur la pochette : plein écran (le bouton est dans la barre de lecture).
        let cover_resp = ui.interact(cover, ui.id().with("np_cover_click"), Sense::click());
        if cover_resp.double_clicked() {
            self.actions.push(Action::ToggleFullscreen);
        }

        let x0 = cover.right() + 40.0;
        let tw = rect.right() - x0 - 24.0;
        // Les textes de l'ancien morceau s'effacent en glissant ; ceux du nouveau arrivent.
        if let Some(old) = self.np_prev_text.as_ref().filter(|_| fade < 1.0) {
            let mut p = ui.painter().clone();
            p.multiply_opacity(1.0 - fade);
            let ox = x0 - dir * 28.0 * fade;
            let mut oy = cover.top() + 38.0;
            for (text, font, color) in [
                (&old[0], theme::semibold(36.0 * big), theme::TEXT),
                (&old[1], FontId::proportional(18.0 * big), theme::TEXT),
                (&old[2], FontId::proportional(15.0 * big), theme::TEXT_2),
            ] {
                let g = widgets::galley(ui, text, font, color, tw);
                let h = g.size().y;
                p.galley(pos2(ox, oy), g, color);
                oy += h + 8.0;
            }
        }
        let base_opacity = ui.opacity();
        ui.multiply_opacity(fade);
        let x = x0 + dir * 28.0 * (1.0 - fade);
        let mut y = cover.top() + 16.0;
        let p = ui.painter();
        widgets::equalizer(p, pos2(x + 7.0, y + 7.0), 13.0, theme::accent_light(), playing);
        p.text(
            pos2(x + 22.0, y),
            Align2::LEFT_TOP,
            if playing {
                tr("EN COURS DE LECTURE")
            } else {
                tr("EN PAUSE")
            },
            FontId::proportional(12.0),
            theme::accent_light(),
        );
        y += 22.0;
        let mut job = egui::text::LayoutJob::single_section(
            t.title.clone(),
            egui::TextFormat::simple(theme::semibold(36.0 * big), theme::TEXT),
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
                18.0 * big,
                theme::TEXT,
                View::Artist(artist),
            ),
            (
                t.album_or_unknown().to_owned(),
                15.0 * big,
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
        let file = t.cue.as_ref().map(|c| c.file.as_path()).unwrap_or(&t.path);
        if let Some(ext) = file.extension().filter(|_| t.remote.is_none()) {
            details.push(ext.to_string_lossy().to_uppercase());
        }
        let line = ui.painter().text(
            pos2(x, y),
            Align2::LEFT_TOP,
            details.join(" · "),
            FontId::proportional(13.0),
            theme::TEXT_3,
        );
        // Note du morceau, en petit au bout de la ligne : cliquer sur une étoile note.
        let rating = self.settings.ratings.get(&t.path).copied().unwrap_or(0);
        let mut preview = None;
        let mut stars = Vec::new();
        for k in 1..=5u8 {
            let c = pos2(line.right() + 16.0 + 15.0 * (k - 1) as f32, line.center().y);
            let r = ui.interact(
                Rect::from_center_size(c, vec2(15.0, 18.0)),
                ui.id().with(("np_star", k)),
                Sense::click(),
            );
            if r.hovered() {
                preview = Some(k);
            }
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.actions.push(Action::SetRating(idx, if rating == k { 0 } else { k }));
            }
            stars.push(c);
        }
        let shown = preview.unwrap_or(rating);
        for (k, c) in stars.into_iter().enumerate() {
            let on = (k as u8) < shown;
            let color = if preview.is_some() {
                theme::accent_light()
            } else if on {
                theme::alpha(theme::accent_light(), 0xC0)
            } else {
                theme::white(0x30)
            };
            icons::paint_centered(ui.painter(), if on { Icon::StarFilled } else { Icon::Star }, c, 12.0, color);
        }
        y += 34.0;
        ui.set_opacity(base_opacity);
        let x = x0;
        // Paroles et « À suivre » sont ceux du morceau réellement en cours.
        let idx = real_idx;
        let _ = idx;

        // Paroles ou « À suivre », au choix quand le morceau a des paroles.
        self.want_lyrics(ui.ctx());
        let has_lyrics = self.lyrics.lyrics.is_some();
        let show_lyrics = has_lyrics && self.settings.show_lyrics;
        if has_lyrics {
            let tabs = Rect::from_min_size(pos2(x, y - 8.0), vec2(tw, 32.0));
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(tabs)
                    .layout(Layout::left_to_right(Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if segment(ui, tr("Paroles"), show_lyrics).clicked() {
                        self.settings.show_lyrics = true;
                    }
                    if segment(ui, tr("À suivre"), !show_lyrics).clicked() {
                        self.settings.show_lyrics = false;
                    }
                },
            );
            y += 42.0;
        }
        if show_lyrics {
            let panel = Rect::from_min_max(pos2(x, y), pos2(x + tw.min(560.0), rect.bottom() - 8.0));
            if panel.height() > 60.0 {
                // Les paroles défilent à la molette : elle n'y règle pas le volume.
                self.np_wheel.1.push(panel);
                self.lyrics_panel(ui, panel);
            }
            return;
        }

        if !upcoming.is_empty() {
            ui.painter().text(
                pos2(x, y),
                Align2::LEFT_TOP,
                tr("À SUIVRE"),
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
        heading(ui, tr("Paramètres"));
        ui.add_space(16.0);
        egui::ScrollArea::vertical().id_salt("settings").auto_shrink(false).show(ui, |ui| {
            ui.set_width((ui.available_width() - 16.0).min(900.0));

            subtitle(ui, tr("Langue"));
            card(ui, |ui| {
                card_row(ui, Icon::Globe, tr("Langue de l'interface"), tr("Par défaut, celle de ton bureau."), |s, ui| {
                    for lang in crate::i18n::Lang::ALL.into_iter().rev() {
                        if segment(ui, lang.native_name(), crate::i18n::lang() == lang).clicked() {
                            s.actions.push(Action::SetLang(lang));
                        }
                    }
                }, self);
            });

            subtitle(ui, tr("Bibliothèque"));
            card(ui, |ui| {
                card_row(ui, Icon::FolderOpen, tr("Dossiers de musique"), tr("SterMusic parcourt ces dossiers et leurs sous-dossiers."), |s, ui| {
                    if widgets::secondary_button(ui, Some(Icon::Refresh), tr("Rescanner")).clicked() {
                        s.actions.push(Action::Rescan);
                    }
                    if widgets::secondary_button(ui, Some(Icon::FolderPlus), tr("Ajouter un dossier")).clicked() {
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
                            if widgets::icon_button(ui, Icon::Close, tr("Retirer ce dossier"), 28.0, 14.0).clicked() {
                                self.actions.push(Action::RemoveFolder(i));
                            }
                        });
                    });
                }
                if self.settings.music_dirs.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        ui.label(RichText::new(tr("Aucun dossier pour l'instant.")).color(theme::TEXT_3));
                    });
                }
                separator(ui);
                let mut auto = self.settings.auto_update;
                let desc = match (&self.watcher.error, auto) {
                    (Some(e), true) => trf!("Surveillance incomplète : {e}. « Rescanner » reste possible.", e),
                    _ => tr("Un morceau ajouté, modifié ou supprimé dans tes dossiers apparaît tout seul, sans « Rescanner ».").to_owned(),
                };
                card_row(ui, Icon::Refresh, tr("Mise à jour automatique"), &desc, |_, ui| {
                    widgets::switch(ui, &mut auto);
                }, self);
                if auto != self.settings.auto_update {
                    self.actions.push(Action::SetAutoUpdate(auto));
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    let status = if self.scan_rx.is_some() {
                        trf!("Analyse en cours… {0}", plural(self.scan_count, "morceau", "morceaux"))
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

            subtitle(ui, tr("Apparence"));
            card(ui, |ui| {
                let mut rounded = self.settings.rounded;
                card_row(ui, Icon::WinMaximize, tr("Coins arrondis"), tr("Arrondit les coins de la fenêtre (sauf quand elle est agrandie)."), |_, ui| {
                    widgets::switch(ui, &mut rounded);
                }, self);
                self.settings.rounded = rounded;
                separator(ui);

                // Fond de la fenêtre : trois choix, en boutons segmentés.
                card_row(ui, Icon::Palette, tr("Fond de la fenêtre"), tr("Mica : ton fond d'écran flouté et teinté. Transparent : le flou de KWin."), |s, ui| {
                    let options = [(BackdropKind::Transparent, tr("Transparent")), (BackdropKind::Mica, tr("Mica")), (BackdropKind::Solid, tr("Uni"))];
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
                        let label = if self.settings.backdrop == BackdropKind::Mica { tr("Intensité") } else { tr("Opacité") };
                        ui.add_sized([80.0, 20.0], egui::Label::new(RichText::new(label).color(theme::TEXT_2)));
                        let seek = widgets::seek_bar(ui, "mica_strength", self.settings.mica_strength, 220.0, true);
                        if let Some(v) = seek.preview.or(seek.commit) {
                            self.settings.mica_strength = v;
                        }
                        ui.label(RichText::new(crate::i18n::percent(self.settings.mica_strength * 100.0)).small().color(theme::TEXT_3));
                    });
                }
                if self.settings.backdrop == BackdropKind::Mica {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        let (text, color) = if self.kwin.active() {
                            (tr("Le Mica suit la fenêtre et le fond d'écran de chaque écran (script KWin actif)."), theme::accent_light())
                        } else {
                            (tr("Le Mica ne suit pas la position de la fenêtre (KWin indisponible) : fond d'écran de l'écran principal."), theme::TEXT_3)
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
                                RichText::new(tr("Pour flouter l'arrière-plan, KWin doit connaître SterMusic : avec l'effet Better Blur (forceblur), ce bouton ajoute « stermusic » à sa liste de fenêtres."))
                                    .small()
                                    .color(theme::TEXT_3),
                            );
                            ui.add_space(4.0);
                            if widgets::secondary_button(ui, Some(Icon::Sparkles), tr("Activer le flou de KWin pour SterMusic")).clicked() {
                                self.actions.push(Action::EnableKwinBlur);
                            }
                        });
                    });
                }
            });

            subtitle(ui, tr("Couleurs"));
            card(ui, |ui| {
                let system = theme::system_theme();
                let desc = match system {
                    Some(_) => tr("« Système » reprend les couleurs et l'accent de ton bureau. Enregistre-les en favori pour les garder même si tu changes celles du bureau."),
                    None => tr("Les seize thèmes de SterCode, et tes favoris."),
                };
                card_row(ui, Icon::Palette, tr("Thème"), desc, |s, ui| {
                    // Un favori ou un thème intégré est déjà figé : seul le thème du système vaut d'être copié.
                    if s.settings.theme == theme::KDE_ID
                        && widgets::secondary_button(ui, Some(Icon::Heart), tr("Enregistrer en favori"))
                            .on_hover_text(tr("Garde une copie des couleurs actuelles du bureau"))
                            .clicked()
                    {
                        s.actions.push(Action::SaveTheme);
                    }
                }, self);
                ui.add_space(6.0);
                let favorites = theme::saved();
                let groups: [(&str, Vec<&'static theme::Theme>); 3] = [
                    ("", system.into_iter().collect()),
                    (tr("Favoris"), favorites),
                    (tr("SterCode"), theme::THEMES.iter().collect()),
                ];
                for (label, themes) in groups {
                    if themes.is_empty() {
                        continue;
                    }
                    if !label.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(52.0);
                            ui.label(RichText::new(label).small().color(theme::TEXT_3));
                        });
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(52.0);
                        ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                        for t in themes {
                            self.theme_swatch(ui, t);
                        }
                    });
                    ui.add_space(4.0);
                }
                separator(ui);
                let mut adaptive = self.settings.adaptive_theme;
                card_row(ui, Icon::Image, tr("Thème adaptatif"), tr("Les couleurs de SterMusic suivent la pochette du morceau en cours, en fondu d'un morceau à l'autre. Sans pochette (ou rien en lecture), on revient au thème choisi ci-dessus."), |_, ui| {
                    widgets::switch(ui, &mut adaptive);
                }, self);
                self.settings.adaptive_theme = adaptive;
            });

            subtitle(ui, tr("Son"));
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.vertical(|ui| self.equalizer_panel(ui));
                });
                separator(ui);
                let mut crossfade = self.settings.crossfade;
                let desc = if self.settings.bit_perfect {
                    tr("Le morceau suivant commence en fondu pendant que le précédent s'efface. Sans effet en sortie bit-perfect.")
                } else {
                    tr("Le morceau suivant commence en fondu pendant que le précédent s'efface. Sans fondu, les morceaux s'enchaînent sans le moindre blanc.")
                };
                card_row(ui, Icon::Shuffle, tr("Fondu enchaîné"), desc, |_, ui| {
                    widgets::switch(ui, &mut crossfade);
                }, self);
                self.settings.crossfade = crossfade;
                if crossfade {
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        ui.add_sized([80.0, 20.0], egui::Label::new(RichText::new(tr("Durée")).color(theme::TEXT_2)));
                        let value = (self.settings.crossfade_secs - 1.0) / 11.0;
                        let seek = widgets::seek_bar(ui, "crossfade_secs", value, 220.0, true);
                        if let Some(v) = seek.preview.or(seek.commit) {
                            self.settings.crossfade_secs = (1.0 + v * 11.0).round();
                        }
                        ui.label(RichText::new(format!("{:.0} s", self.settings.crossfade_secs)).small().color(theme::TEXT_3));
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        let mut albums = self.settings.crossfade_albums;
                        widgets::switch(ui, &mut albums);
                        self.settings.crossfade_albums = albums;
                        ui.label(RichText::new(tr("Aussi entre deux morceaux d'un même album")).color(theme::TEXT_2))
                            .on_hover_text(tr("Désactivé, les morceaux d'un album s'enchaînent sans blanc, comme sur le disque (albums live, mixés, concept)."));
                    });
                }
                separator(ui);
                self.visualizer_settings(ui);
                separator(ui);
                let mut leveling = self.settings.leveling;
                card_row(ui, Icon::Volume2, tr("Nivellement du volume"), tr("Joue les morceaux à un volume semblable : gain ReplayGain du fichier s'il existe, sinon réglage automatique."), |_, ui| {
                    widgets::switch(ui, &mut leveling);
                }, self);
                if leveling != self.settings.leveling {
                    self.settings.leveling = leveling;
                    self.push_audio();
                }
                separator(ui);
                self.output_setting(ui);
                separator(ui);
                let mut exact = self.settings.bit_perfect;
                let desc = if crate::outputs::can_force_rate() {
                    tr("Le son du fichier part tel quel, à sa fréquence (44,1 kHz, 96 kHz…) : PipeWire la suit pendant la lecture au lieu de tout convertir en 48 kHz. Égaliseur, nivellement, fondu et volume de SterMusic sont mis de côté : le volume se règle dans le système.")
                } else {
                    tr("Le son du fichier part tel quel : égaliseur, nivellement, fondu et volume de SterMusic sont mis de côté. pw-metadata (PipeWire) est introuvable : le système peut encore convertir la fréquence.")
                };
                card_row(ui, Icon::AudioLines, tr("Sortie bit-perfect"), desc, |_, ui| {
                    widgets::switch(ui, &mut exact);
                }, self);
                if exact != self.settings.bit_perfect {
                    self.set_bit_perfect(exact);
                }
            });

            subtitle(ui, tr("Lecture"));
            card(ui, |ui| {
                let mut resume = self.settings.resume;
                card_row(ui, Icon::Clock, tr("Reprendre là où je me suis arrêté"), tr("Au lancement, la file d'attente et la position sont restaurées (en pause)."), |_, ui| {
                    widgets::switch(ui, &mut resume);
                }, self);
                self.settings.resume = resume;
                separator(ui);
                let mut online = self.settings.lyrics_online;
                card_row(ui, Icon::Mic, tr("Paroles en ligne"), tr("Sans fichier .lrc ni paroles dans les tags, les paroles sont cherchées sur lrclib.net (le titre, l'artiste et l'album y sont envoyés)."), |_, ui| {
                    widgets::switch(ui, &mut online);
                }, self);
                if online != self.settings.lyrics_online {
                    self.settings.lyrics_online = online;
                    self.lyrics.forget();
                }
                separator(ui);
                let (state, desc) = if self.mpris.available() {
                    (tr("Actif"), tr("Touches multimédia du clavier, widget lecteur du bureau et écran de verrouillage (MPRIS)."))
                } else {
                    (tr("Indisponible"), tr("Le bus de session D-Bus n'a pas répondu : les touches multimédia ne piloteront pas SterMusic."))
                };
                card_row(ui, Icon::AudioLines, tr("Intégration au bureau"), desc, |_, ui| {
                    ui.label(RichText::new(state).color(if state == tr("Actif") { theme::accent_light() } else { theme::TEXT_3 }));
                }, self);
                separator(ui);
                let mut tray = self.settings.tray;
                let desc = if self.kwin.active() {
                    tr("Lecture, morceau suivant ou précédent depuis la barre des tâches (clic du milieu : lecture / pause, molette : morceaux). Fermer la fenêtre la cache alors dans la zone de notification, la musique continue : pour quitter, « Quitter » dans le menu de l'icône.")
                } else {
                    tr("Lecture, morceau suivant ou précédent depuis la barre des tâches (clic du milieu : lecture / pause, molette : morceaux). Hors de KDE Plasma, fermer la fenêtre quitte quand même SterMusic.")
                };
                card_row(ui, Icon::Bell, tr("Icône dans la zone de notification"), desc, |_, ui| {
                    widgets::switch(ui, &mut tray);
                }, self);
                if tray != self.settings.tray {
                    self.settings.tray = tray;
                    if tray {
                        self.start_tray();
                    } else {
                        self.stop_tray();
                    }
                }
                self.playback_extra_settings(ui);
            });

            self.servers_settings(ui);
            self.scrobble_settings(ui);

            subtitle(ui, tr("Éditeur de tags"));
            card(ui, |ui| {
                card_row(ui, Icon::Tag, tr("Modifier des fichiers"), tr("Page « Tags » de la barre latérale, ou clic droit sur un morceau, un album, une sélection ; même hors bibliothèque."), |s, ui| {
                    if widgets::secondary_button(ui, Some(Icon::FolderOpen), tr("Choisir des fichiers…")).clicked() {
                        s.actions.push(Action::PickTagFiles);
                    }
                }, self);
                separator(ui);
                card_row(ui, Icon::Library, tr("Entretien de la bibliothèque"), tr("Pochettes manquantes à compléter, volume ReplayGain à mesurer, morceaux en double à repérer."), |s, ui| {
                    for tool in super::lib_tools::LibTool::ALL.into_iter().rev() {
                        if widgets::secondary_button(ui, Some(tool.icon()), tool.label()).on_hover_text(tool.hint()).clicked() {
                            s.actions.push(Action::LibTool(tool));
                        }
                    }
                }, self);
                separator(ui);
                card_row(ui, Icon::Music, tr("Version des tags MP3"), tr("ID3v2.4 : la plus récente. ID3v2.3 : pour les vieux autoradios et l'Explorateur de Windows."), |s, ui| {
                    for (v23, label) in [(true, "ID3v2.3"), (false, "ID3v2.4")] {
                        if segment(ui, label, s.settings.tag_id3v23 == v23).clicked() {
                            s.settings.tag_id3v23 = v23;
                        }
                    }
                }, self);
                separator(ui);
                let mut keep = self.settings.tag_keep_mtime;
                card_row(ui, Icon::Clock, tr("Conserver la date de modification"), tr("Un album retouché ne remonte pas dans « Ajoutés récemment »."), |_, ui| {
                    widgets::switch(ui, &mut keep);
                }, self);
                self.settings.tag_keep_mtime = keep;
            });

            subtitle(ui, tr("Statistiques"));
            card(ui, |ui| {
                let listens = self.listens.len();
                let desc = trf!(
                    "{0} dans l'historique daté. Pour un tableur (LibreOffice, Excel) : une ligne par morceau, ou une ligne par écoute.",
                    plural(listens, "écoute", "écoutes")
                );
                card_row(ui, Icon::Upload, tr("Exporter les statistiques"), &desc, |s, ui| {
                    if widgets::secondary_button(ui, None, tr("Écoutes (CSV)")).clicked() {
                        s.actions.push(Action::ExportStats(false));
                    }
                    if widgets::secondary_button(ui, None, tr("Morceaux (CSV)")).clicked() {
                        s.actions.push(Action::ExportStats(true));
                    }
                }, self);
                separator(ui);
                card_row(ui, Icon::Trash, tr("Effacer les statistiques"), tr("Remet à zéro les compteurs d'écoutes et l'historique (après confirmation)."), |s, ui| {
                    if widgets::secondary_button(ui, Some(Icon::Trash), tr("Effacer…")).clicked() {
                        s.actions.push(Action::ClearStats);
                    }
                }, self);
            });

            subtitle(ui, tr("Raccourcis clavier"));
            card(ui, |ui| {
                for (keys, what) in [
                    (tr("Espace"), tr("Lecture / pause")),
                    (tr("Ctrl + → / ←"), tr("Morceau suivant / précédent")),
                    ("→ / ←", tr("Avancer / reculer de 5 secondes")),
                    (tr("Ctrl + ↑ / ↓"), tr("Monter / baisser le volume")),
                    (tr("Ctrl + F"), tr("Rechercher")),
                    (tr("Ctrl + M"), tr("Mini lecteur")),
                    (tr("Échap"), tr("Retour")),
                    (tr("Ctrl / Maj + clic"), tr("Sélectionner plusieurs morceaux")),
                    (tr("Ctrl + S"), tr("Enregistrer les tags (éditeur)")),
                    (tr("Ctrl + Maj + S"), tr("Enregistrer les tags de la sélection")),
                    (tr("Suppr"), tr("Retirer la sélection de la liste (éditeur)")),
                ] {
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        ui.add_sized([160.0, 22.0], egui::Label::new(RichText::new(keys).color(theme::TEXT).monospace()));
                        ui.label(RichText::new(what).color(theme::TEXT_2));
                    });
                }
            });

            subtitle(ui, tr("À propos"));
            card(ui, |ui| {
                card_row(ui, Icon::Info, &format!("SterMusic {}", env!("CARGO_PKG_VERSION")), tr("Lecteur de musique pour Linux, écrit en Rust (egui, rodio, Symphonia, lofty). Icônes Lucide (ISC)."), |_, _| {}, self);
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    ui.label(RichText::new(trf!("Données : {0}", crate::library::data_dir().display())).small().color(theme::TEXT_3));
                });
            });
            ui.add_space(32.0);
        });
    }
}

pub(super) fn subtitle(ui: &mut egui::Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).font(theme::semibold(15.0)).color(theme::TEXT));
    ui.add_space(4.0);
}

/// Carte de réglages (calque translucide arrondi, comme les paramètres de Windows 11).
pub(super) fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
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

pub(super) fn separator(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, theme::stroke()));
    ui.add_space(6.0);
}

/// Ligne de réglage : icône, titre, description à gauche ; contrôles à droite.
pub(super) fn card_row(
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
        // Les contrôles d'abord, à droite : le texte prend la place qui reste et passe à la ligne
        // au lieu de filer sous les boutons.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            right(app, ui);
            ui.add_space(12.0);
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(RichText::new(title).color(theme::TEXT)).wrap());
                ui.add(egui::Label::new(RichText::new(desc).small().color(theme::TEXT_2)).wrap());
            });
        });
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

impl SterMusic {
    /// Vignette d'un thème : son fond et ses deux couleurs, son nom ; clic droit sur un favori
    /// pour le renommer ou le supprimer.
    fn theme_swatch(&mut self, ui: &mut egui::Ui, t: &'static theme::Theme) {
        let selected = self.settings.theme == t.id;
        let favorite = t.id.starts_with("fav-");
        let (r, resp) = ui.allocate_exact_size(vec2(96.0, 64.0), Sense::click());
        widgets::paint_interaction(ui, r, 8, &resp);
        let sw = Rect::from_center_size(pos2(r.center().x, r.top() + 22.0), vec2(56.0, 30.0));
        ui.painter().rect_filled(sw, CornerRadius::same(8), t.bg);
        let dot = |k: f32, col: Color32| {
            ui.painter()
                .circle_filled(pos2(sw.left() + 18.0 + k * 20.0, sw.center().y), 8.0, col)
        };
        dot(0.0, t.accent);
        dot(1.0, t.accent_2);
        let stroke = if selected {
            Stroke::new(2.0_f32, t.accent_light)
        } else {
            Stroke::new(1.0_f32, theme::stroke_strong())
        };
        ui.painter()
            .rect_stroke(sw, CornerRadius::same(8), stroke, StrokeKind::Outside);
        let name = if t.id == theme::KDE_ID {
            theme::system_label()
        } else {
            tr(t.name)
        };
        let color = if selected { theme::TEXT } else { theme::TEXT_2 };
        let g = widgets::galley(ui, name, FontId::proportional(12.0), color, r.width() - 6.0);
        ui.painter().galley(
            pos2(
                r.center().x - g.size().x / 2.0,
                r.bottom() - 10.0 - g.size().y / 2.0,
            ),
            g,
            color,
        );
        let resp = if favorite {
            resp.on_hover_text(trf!("{name} (clic droit : renommer, supprimer)", name))
        } else {
            resp.on_hover_text(name)
        };
        if resp.clicked() {
            self.actions.push(Action::SetTheme(t.id));
        }
        if favorite {
            resp.context_menu(|ui| {
                if widgets::menu_item(ui, Some(Icon::Pencil), tr("Renommer…")).clicked() {
                    self.actions.push(Action::RenameTheme(t.id.to_owned()));
                }
                if widgets::menu_item(ui, Some(Icon::Trash), tr("Supprimer")).clicked() {
                    self.actions.push(Action::DeleteTheme(t.id.to_owned()));
                }
            });
        }
    }
}

/// Bouton segmenté (choix exclusif), sélection à l'accent.
pub(super) fn segment(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), FontId::proportional(13.5), theme::TEXT));
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 26.0, 30.0), Sense::click());
    // La sélection passe d'un segment à l'autre en fondu, avec un petit rebond à l'appui.
    let s = anim::toggle(ui.ctx(), response.id.with("sel"), selected, anim::NORMAL);
    let press = anim::value(
        ui.ctx(),
        response.id.with("press"),
        if response.is_pointer_button_down_on() { 1.0 } else { 0.0 },
        anim::NORMAL,
        anim::Easing::OutBack,
    );
    let rect = Rect::from_center_size(rect.center(), rect.size() * (1.0 - 0.04 * press));
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(6),
        theme::mix(theme::white(0x0C), theme::alpha(theme::accent(), 0x55), s),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0_f32, theme::mix(theme::stroke(), theme::accent_light(), s)),
        StrokeKind::Inside,
    );
    widgets::paint_interaction(ui, rect, 6, &response);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        theme::mix(theme::TEXT_2, theme::TEXT, s),
    );
    response
}

/// Pochette pour le shader : sa version floutée (Pochette liquide) et sa position (Anneau).
fn set_cover(w: &mut crate::shader::Waves, rect: Rect, (cover, texture): (Rect, Option<egui::TextureId>)) {
    w.texture = texture;
    w.focus = [
        cover.center().x - rect.left(),
        cover.center().y - rect.top(),
        cover.width() / 2.0,
    ];
}
