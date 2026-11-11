//! Choisir son visualiseur.
//!
//! - Dans « En cours de lecture » : une pastille en haut à droite (‹ nom › ; molette ou V /
//!   Maj+V pour passer au suivant ou au précédent) ; un clic sur le nom ouvre, en bas de la
//!   page, un bandeau d'aperçus animés où l'on choisit d'un clic, la musique continuant derrière.
//! - Dans les paramètres : l'aperçu de chaque emplacement, et une galerie (aperçus animés) pour
//!   choisir, ou pour composer le tirage du mode automatique.
//!
//! Les aperçus sont dessinés par le même shader que le visualiseur, sur le son en cours (ou sur
//! un son imaginaire quand rien ne joue).

use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind};

use super::views::{card_row, separator};
use super::{SterMusic, View};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::shader::{AUTO, STYLES};
use crate::{theme, trf, widgets};

/// Emplacement d'un visualiseur.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    NowPlaying,
    Banner,
}

/// Galerie ouverte depuis les paramètres : choisir pour un emplacement, ou composer le tirage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gallery {
    Choose(Slot),
    Pool,
}

/// Rythmes du mode automatique (secondes ; 0 : à chaque morceau seulement).
const AUTO_RHYTHMS: [(f32, &str); 4] = [
    (0.0, crate::i18n::n_("À chaque morceau")),
    (45.0, crate::i18n::n_("Toutes les 45 s")),
    (120.0, crate::i18n::n_("Toutes les 2 min")),
    (300.0, crate::i18n::n_("Toutes les 5 min")),
];

impl SterMusic {
    pub(super) fn slot_setting(&self, slot: Slot) -> usize {
        match slot {
            Slot::NowPlaying => self.settings.viz_style,
            Slot::Banner => self.settings.banner_style,
        }
        .min(AUTO)
    }

    fn set_slot(&mut self, slot: Slot, style: usize) {
        match slot {
            Slot::NowPlaying => self.settings.viz_style = style.min(AUTO),
            Slot::Banner => self.settings.banner_style = style.min(AUTO),
        }
        self.viz_flash = self.egui_ctx.input(|i| i.time);
    }

    /// Visualiseur suivant (`step` = 1) ou précédent (−1) dans « En cours de lecture ». En
    /// automatique, on part de celui qui est à l'écran.
    pub(super) fn step_viz(&mut self, step: i32) {
        let from = if self.settings.viz_style >= AUTO {
            self.np_shown
        } else {
            self.settings.viz_style
        };
        let n = AUTO as i32;
        let next = ((from as i32 + step) % n + n) % n;
        self.set_slot(Slot::NowPlaying, next as usize);
    }

    /// Réglages propres au visualiseur `style` (ceux d'origine s'il n'y a pas touché).
    pub(super) fn viz_tune(&self, style: usize) -> crate::viz::Tune {
        STYLES
            .get(style)
            .and_then(|(name, _)| self.settings.viz_tunes.get(*name))
            .copied()
            .unwrap_or_default()
    }

    fn set_viz_tune(&mut self, style: usize, tune: crate::viz::Tune) {
        let name = STYLES[style].0;
        if tune == crate::viz::Tune::default() {
            self.settings.viz_tunes.remove(name);
        } else {
            self.settings.viz_tunes.insert(name.to_owned(), tune);
        }
    }

    // ---------- Aperçus ----------

    /// Ce qu'il faut pour dessiner les aperçus d'une image : le son en cours (ou un son
    /// imaginaire quand rien ne joue), la palette, la pochette floutée, le tirage automatique.
    fn previews(&mut self, now: f64) -> Previews {
        let palette = crate::viz::harmonize(self.viz_palette());
        let mut w = crate::viz::waves(&self.spectrum, palette, 1.0);
        if !self.is_playing() {
            let t = now as f32;
            for (i, l) in w.levels.iter_mut().enumerate() {
                let fi = i as f32;
                *l = (0.42 + 0.28 * (t * 1.3 + fi * 0.55).sin() - fi * 0.012).clamp(0.05, 1.0);
            }
            for (i, v) in w.wave.iter_mut().enumerate() {
                let fi = i as f32;
                *v = 0.55 * (fi * 0.21 + t * 2.0).sin() + 0.3 * (fi * 0.47 - t * 1.3).sin();
            }
            w.kick = (1.0 - (t * 2.0).fract() * 3.0).max(0.0) * 0.7;
            w.bass = 0.45;
            w.energy = 0.45;
            w.time = t * 0.45 * self.settings.viz_speed;
        }
        w.texture = self.current().and_then(|i| {
            let path = self.lib.tracks[i].path.clone();
            self.covers.large(&path).and_then(|l| l.blurred.as_ref().map(|t| t.id()))
        });
        Previews {
            waves: w,
            tunes: (0..AUTO).map(|i| self.viz_tune(i)).collect(),
            pool: self.auto_allowed(),
            intensity: self.settings.viz_intensity,
            now,
        }
    }

    // ---------- En cours de lecture ----------

    /// Pastille ‹ nom › en haut à droite de la page, et bandeau d'aperçus s'il est ouvert.
    pub(super) fn np_viz_controls(&mut self, ui: &mut egui::Ui, area: Rect, shown: usize) {
        self.np_shown = shown;
        let now = ui.input(|i| i.time);
        let setting = self.slot_setting(Slot::NowPlaying);
        let name = if setting >= AUTO {
            trf!("{0} · auto", tr(STYLES[shown].0))
        } else {
            tr(STYLES[shown].0).to_owned()
        };
        let font = FontId::proportional(13.0);
        let text_w = ui.fonts(|f| f.layout_no_wrap(name.clone(), font.clone(), Color32::WHITE).size().x);
        let h = 34.0;
        let w = text_w + 34.0 + 3.0 * 30.0 + 8.0;
        let pill = Rect::from_min_size(pos2(area.right() - 16.0 - w, area.top() + 10.0), vec2(w, h));
        let resp = ui.interact(pill, ui.id().with("np_viz_pill"), Sense::hover());
        // Sur la pastille, la molette change de visualiseur, pas le volume.
        self.np_wheel.1.push(pill.expand(4.0));
        let flash = ((now - self.viz_flash) as f32 / 1.2).clamp(0.0, 1.0);
        // Les flèches et le nom, posés par-dessus, prennent le survol : on regarde le pointeur.
        let over_pill = ui.rect_contains_pointer(pill);
        let _ = resp;
        let hot = over_pill || self.viz_strip || self.viz_strip_tune || flash < 1.0;
        let a = crate::anim::value(
            ui.ctx(),
            ui.id().with("np_viz_pill_a"),
            if hot { 1.0 } else { 0.0 },
            0.25,
            crate::anim::Easing::Out,
        );
        let painter = ui.painter();
        painter.rect_filled(pill, CornerRadius::same(17), Color32::from_black_alpha((60.0 + 70.0 * a) as u8));
        let border = theme::mix(theme::white(0x22), theme::accent_light(), (1.0 - flash) * 0.9);
        painter.rect_stroke(pill, CornerRadius::same(17), Stroke::new(1.0_f32, border), StrokeKind::Inside);
        let text_color = theme::alpha(theme::TEXT, (150.0 + 105.0 * a) as u8);
        let mut step = 0;
        // ‹ précédent
        let left = Rect::from_min_size(pill.min, vec2(30.0, h));
        let l = ui.interact(left, ui.id().with("np_viz_prev"), Sense::click()).on_hover_text(tr("Visualiseur précédent (Maj+V)"));
        if l.hovered() {
            painter.rect_filled(left.shrink(3.0), CornerRadius::same(14), theme::white(0x16));
        }
        icons::paint_centered(painter, Icon::ChevronLeft, left.center(), 14.0, text_color);
        if l.clicked() {
            step = -1;
        }
        // › suivant
        let right = Rect::from_min_size(pos2(pill.right() - 30.0, pill.top()), vec2(30.0, h));
        let r = ui.interact(right, ui.id().with("np_viz_next"), Sense::click()).on_hover_text(tr("Visualiseur suivant (V)"));
        if r.hovered() {
            painter.rect_filled(right.shrink(3.0), CornerRadius::same(14), theme::white(0x16));
        }
        icons::paint_centered(painter, Icon::ChevronRight, right.center(), 14.0, text_color);
        if r.clicked() {
            step = 1;
        }
        // Réglages de l'effet à l'écran : une carte sous la pastille.
        let gear = Rect::from_min_size(pos2(right.left() - 30.0, pill.top()), vec2(30.0, h));
        let g = ui
            .interact(gear, ui.id().with("np_viz_tune"), Sense::click())
            .on_hover_text(tr("Régler cet effet"));
        let tuned = self.viz_tune(shown) != crate::viz::Tune::default();
        if g.hovered() || self.viz_strip_tune {
            painter.rect_filled(gear.shrink(3.0), CornerRadius::same(14), theme::white(0x16));
        }
        icons::paint_centered(
            painter,
            Icon::Equalizer,
            gear.center(),
            14.0,
            if tuned || self.viz_strip_tune { theme::accent_light() } else { text_color },
        );
        if g.clicked() {
            self.viz_strip_tune = !self.viz_strip_tune;
            self.viz_strip = false;
        }
        // Le nom : ouvre ou ferme le bandeau d'aperçus.
        let mid = Rect::from_min_max(pos2(left.right(), pill.top()), pos2(gear.left(), pill.bottom()));
        let m = ui
            .interact(mid, ui.id().with("np_viz_name"), Sense::click())
            .on_hover_text(tr("Choisir le visualiseur (molette : changer)"));
        if m.hovered() || self.viz_strip {
            painter.rect_filled(mid.shrink2(vec2(0.0, 3.0)), CornerRadius::same(14), theme::white(0x10));
        }
        let icon_color = if setting >= AUTO { theme::accent_light() } else { text_color };
        icons::paint_centered(
            painter,
            if setting >= AUTO { Icon::Shuffle } else { Icon::Sparkles },
            pos2(mid.left() + 16.0, mid.center().y),
            15.0,
            icon_color,
        );
        painter.text(pos2(mid.left() + 30.0, mid.center().y), Align2::LEFT_CENTER, &name, font, text_color);
        if m.clicked() {
            self.viz_strip = !self.viz_strip;
            self.viz_strip_scroll = self.viz_strip;
            self.viz_strip_tune = false;
        }
        // Molette sur la pastille : visualiseur suivant ou précédent.
        if over_pill {
            let dy = ui.input(|i| i.raw_scroll_delta.y);
            if dy.abs() > 0.5 {
                step = if dy < 0.0 { 1 } else { -1 };
            }
        }
        if step != 0 {
            self.step_viz(step);
        }
        if flash < 1.0 || a > 0.0 && a < 1.0 {
            ui.ctx().request_repaint();
        }
        if self.viz_strip {
            self.np_viz_strip(ui, area, now);
        }
        if self.viz_strip_tune {
            self.np_viz_tune_card(ui, pill);
        }
    }

    /// Carte des réglages de l'effet à l'écran, sous la pastille : elle laisse voir la pochette
    /// et le bas de la page, où vivent la plupart des effets.
    fn np_viz_tune_card(&mut self, ui: &mut egui::Ui, pill: Rect) {
        let shown = self.np_shown.min(AUTO - 1);
        let tune = self.viz_tune(shown);
        let open = crate::anim::appear(ui.ctx(), egui::Id::new("np_viz_tune_card"), crate::anim::FAST);
        let width = 372.0;
        let pos = pos2(pill.right() - width, pill.bottom() + 8.0 - 10.0 * (1.0 - open));
        let mut tuned = None;
        let mut close = false;
        egui::Area::new(egui::Id::new("np_viz_tune_area"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(theme::alpha(theme::bg(), (235.0 * open) as u8))
                    .stroke(Stroke::new(1.0_f32, theme::white(0x1C)))
                    .corner_radius(14)
                    .inner_margin(14.0)
                    .shadow(egui::Shadow {
                        offset: [0, 10],
                        blur: 36,
                        spread: 0,
                        color: Color32::from_black_alpha((130.0 * open) as u8),
                    })
                    .show(ui, |ui| {
                        ui.set_width(width - 28.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(trf!("Régler « {0} »", tr(STYLES[shown].0)))
                                    .font(theme::semibold(15.0))
                                    .color(theme::TEXT),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if widgets::icon_button(ui, Icon::Close, tr("Fermer (Échap)"), 26.0, 13.0).clicked() {
                                    close = true;
                                }
                            });
                        });
                        ui.label(
                            egui::RichText::new(tr("Ces réglages ne valent que pour cet effet."))
                                .small()
                                .color(theme::TEXT_3),
                        );
                        ui.add_space(8.0);
                        tuned = tune_sliders(ui, shown, tune);
                    });
            });
        if close {
            self.viz_strip_tune = false;
        }
        if let Some(t) = tuned {
            self.set_viz_tune(shown, t);
        }
        ui.ctx().request_repaint();
    }

    /// Bandeau d'aperçus au bas de « En cours de lecture », au premier plan (par-dessus la
    /// pochette et les textes).
    fn np_viz_strip(&mut self, ui: &mut egui::Ui, area: Rect, now: f64) {
        let open = crate::anim::appear(ui.ctx(), egui::Id::new("np_viz_strip"), crate::anim::FAST);
        let tile = vec2(176.0, 99.0);
        let height = tile.y + 24.0 + 56.0;
        let panel = Rect::from_min_size(
            pos2(area.left() + 16.0, area.bottom() - height - 16.0 + 24.0 * (1.0 - open)),
            vec2(area.width() - 32.0, height),
        );
        let setting = self.slot_setting(Slot::NowPlaying);
        let pv = self.previews(now);
        let scroll_to_current = std::mem::take(&mut self.viz_strip_scroll);
        let mut chosen = None;
        let mut close = false;
        egui::Area::new(egui::Id::new("np_viz_strip_area"))
            .order(egui::Order::Foreground)
            .fixed_pos(panel.min)
            .show(ui.ctx(), |ui| {
                ui.set_min_size(panel.size());
                ui.set_max_size(panel.size());
                let painter = ui.painter();
                painter.add(
                    egui::Shadow {
                        offset: [0, 10],
                        blur: 40,
                        spread: 0,
                        color: Color32::from_black_alpha((140.0 * open) as u8),
                    }
                    .as_shape(panel, CornerRadius::same(16)),
                );
                painter.rect_filled(panel, CornerRadius::same(16), theme::alpha(theme::bg(), (235.0 * open) as u8));
                painter.rect_stroke(panel, CornerRadius::same(16), Stroke::new(1.0_f32, theme::white(0x1C)), StrokeKind::Inside);
                // Le fond du bandeau retient les clics (rien ne passe à la page dessous).
                let _ = ui.interact(panel, ui.id().with("np_viz_strip_bg"), Sense::click());
                let title = painter.text(
                    pos2(panel.left() + 20.0, panel.top() + 22.0),
                    Align2::LEFT_CENTER,
                    tr("Visualiseur"),
                    theme::semibold(15.0),
                    theme::TEXT,
                );
                let hint_x = title.right() + 14.0;
                if hint_x + 460.0 < panel.right() - 48.0 {
                    ui.painter().text(
                        pos2(hint_x, panel.top() + 22.0),
                        Align2::LEFT_CENTER,
                        tr("Un clic pour l'essayer, la musique continue. V / Maj+V : suivant, précédent."),
                        FontId::proportional(12.0),
                        theme::TEXT_3,
                    );
                }
                let close_r = Rect::from_center_size(pos2(panel.right() - 24.0, panel.top() + 22.0), vec2(28.0, 28.0));
                if ui
                    .put(close_r, |ui: &mut egui::Ui| widgets::icon_button(ui, Icon::Close, tr("Fermer (Échap)"), 28.0, 14.0))
                    .clicked()
                {
                    close = true;
                }
                let inner = Rect::from_min_max(
                    pos2(panel.left() + 12.0, panel.top() + 44.0),
                    pos2(panel.right() - 12.0, panel.bottom() - 8.0),
                );
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                let hovered = child.rect_contains_pointer(inner);
                let out = egui::ScrollArea::horizontal()
                    .id_salt("np_viz_strip_scroll")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(&mut child, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.0;
                            for style in std::iter::once(AUTO).chain(0..AUTO) {
                                let resp = viz_tile(ui, &pv, tile, style, style == setting, None);
                                if resp.clicked() {
                                    chosen = Some(style);
                                }
                                // À l'ouverture, le visualiseur choisi est amené à l'écran.
                                if style == setting && scroll_to_current {
                                    resp.scroll_to_me(Some(egui::Align::Center));
                                }
                            }
                        });
                    });
                // La molette (verticale) fait défiler le bandeau de côté.
                if hovered {
                    let dy = ui.input(|i| i.raw_scroll_delta.y);
                    if dy != 0.0 {
                        let mut state = out.state;
                        let max = (out.content_size.x - out.inner_rect.width()).max(0.0);
                        state.offset.x = (state.offset.x - dy).clamp(0.0, max);
                        state.store(ui.ctx(), out.id);
                    }
                }
            });
        if close {
            self.viz_strip = false;
        }
        if let Some(style) = chosen {
            self.set_slot(Slot::NowPlaying, style);
        }
        ui.ctx().request_repaint();
    }

    // ---------- Paramètres ----------

    /// Section « Visualiseur » de la carte Son.
    pub(super) fn visualizer_settings(&mut self, ui: &mut egui::Ui) {
        let mut visualizer = self.settings.visualizer;
        card_row(ui, Icon::AudioLines, tr("Visualiseur"), tr("Lumières qui suivent la musique, aux couleurs de la pochette (En cours de lecture, accueil)."), |_, ui| {
            widgets::switch(ui, &mut visualizer);
        }, self);
        self.settings.visualizer = visualizer;
        if !visualizer || !crate::shader::available() {
            return;
        }
        let now = ui.input(|i| i.time);
        let pv = self.previews(now);
        ui.add_space(6.0);
        // Les deux emplacements, côte à côte : aperçu, nom, « Changer ».
        ui.horizontal_wrapped(|ui| {
            ui.add_space(52.0);
            ui.spacing_mut().item_spacing = vec2(18.0, 12.0);
            for (slot, label) in [(Slot::NowPlaying, tr("En cours de lecture")), (Slot::Banner, tr("Bandeau de l'accueil"))] {
                let setting = self.slot_setting(slot);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(label).small().color(theme::TEXT_3));
                    ui.add_space(2.0);
                    let shown = if setting >= AUTO { AUTO } else { setting };
                    let resp = viz_tile(ui, &pv, vec2(208.0, 117.0), shown, false, None);
                    if resp.clicked() {
                        self.viz_gallery = Some(Gallery::Choose(slot));
                        self.viz_strip_scroll = true;
                    }
                    resp.on_hover_text(tr("Changer…"));
                });
            }
        });
        ui.add_space(8.0);
        separator(ui);
        // Mode automatique : rythme, et visualiseurs du tirage.
        let pool = self.auto_allowed().len();
        let desc = trf!("Un visualiseur au hasard, en fondu enchaîné. Dans le tirage : {0} sur {1}.", pool, AUTO);
        card_row(ui, Icon::Shuffle, tr("Mode automatique"), &desc, |s, ui| {
            if widgets::secondary_button(ui, Some(Icon::Pencil), tr("Choisir le tirage…")).clicked() {
                s.viz_gallery = Some(Gallery::Pool);
            }
        }, self);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(52.0);
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (secs, label) in AUTO_RHYTHMS {
                if super::views::segment(ui, tr(label), (self.settings.viz_auto_secs - secs).abs() < 0.5).clicked() {
                    self.settings.viz_auto_secs = secs;
                }
            }
        });
        ui.add_space(6.0);
        separator(ui);
        // Luminosité, vitesse, poussières.
        for (label, tip, key, min, max) in [
            (tr("Luminosité"), tr("Plus discret ou plus éclatant."), "viz_intensity", 0.4, 1.6),
            (tr("Vitesse"), tr("Mouvements plus calmes ou plus vifs (la musique les accélère toujours)."), "viz_speed", 0.4, 2.0),
        ] {
            ui.horizontal(|ui| {
                ui.add_space(52.0);
                ui.add_sized([120.0, 22.0], egui::Label::new(egui::RichText::new(label).color(theme::TEXT_2)))
                    .on_hover_text(tip);
                let value = if key == "viz_intensity" { self.settings.viz_intensity } else { self.settings.viz_speed };
                let seek = widgets::seek_bar(ui, key, (value - min) / (max - min), 220.0, true);
                if let Some(v) = seek.preview.or(seek.commit) {
                    let v = min + v * (max - min);
                    // Un cran magnétique sur la valeur d'origine.
                    let v = if (v - 1.0).abs() < 0.04 { 1.0 } else { v };
                    if key == "viz_intensity" {
                        self.settings.viz_intensity = v;
                    } else {
                        self.settings.viz_speed = v;
                    }
                }
                ui.label(egui::RichText::new(crate::i18n::percent(value * 100.0)).small().color(theme::TEXT_3));
            });
        }
        ui.horizontal(|ui| {
            ui.add_space(52.0);
            let mut on = self.settings.viz_particles;
            widgets::switch(ui, &mut on);
            self.settings.viz_particles = on;
            ui.label(egui::RichText::new(tr("Poussières de lumière")).color(theme::TEXT_2))
                .on_hover_text(tr("De fines lumières qui montent et scintillent sur les coups de grosse caisse."));
        });
        ui.ctx().request_repaint();
    }

    /// Galerie des visualiseurs (aperçus animés), ouverte depuis les paramètres.
    pub(super) fn viz_gallery_ui(&mut self, ctx: &egui::Context) {
        let Some(mode) = self.viz_gallery else { return };
        let now = ctx.input(|i| i.time);
        let screen = ctx.screen_rect();
        let width = (screen.width() - 120.0).clamp(420.0, 980.0);
        let height = (screen.height() - 140.0).clamp(320.0, 720.0);
        let mut close = false;
        let pv = self.previews(now);
        let modal = egui::Modal::new(egui::Id::new("viz_gallery"))
            .frame(egui::Frame::popup(&ctx.style()).inner_margin(20.0).corner_radius(14))
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    let (title, sub) = match mode {
                        Gallery::Choose(Slot::NowPlaying) => (tr("Visualiseur d'En cours de lecture"), tr("Un clic pour choisir ; « Automatique » en change au fil de l'écoute.")),
                        Gallery::Choose(Slot::Banner) => (tr("Visualiseur du bandeau de l'accueil"), tr("Un clic pour choisir ; « Automatique » en change au fil de l'écoute.")),
                        Gallery::Pool => (tr("Tirage du mode automatique"), tr("Coche ceux parmi lesquels le mode automatique choisit au hasard.")),
                    };
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(title).font(theme::semibold(20.0)));
                        ui.label(egui::RichText::new(sub).color(theme::TEXT_2));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if widgets::icon_button(ui, Icon::Close, tr("Fermer"), 32.0, 16.0).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(12.0);
                let cols = ((width + 14.0) / (190.0 + 14.0)).floor().max(2.0);
                let tile_w = (width - 14.0 * (cols - 1.0)) / cols;
                let tile = vec2(tile_w, tile_w * 9.0 / 16.0);
                let pool = pv.pool.clone();
                let scroll_to_current = std::mem::take(&mut self.viz_strip_scroll);
                let mut chosen = None;
                let mut toggle = None;
                egui::ScrollArea::vertical()
                    .max_height(height - 140.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(14.0, 14.0);
                            let styles: Box<dyn Iterator<Item = usize>> = match mode {
                                Gallery::Choose(_) => Box::new(std::iter::once(AUTO).chain(0..AUTO)),
                                Gallery::Pool => Box::new(0..AUTO),
                            };
                            for style in styles {
                                let (selected, check) = match mode {
                                    Gallery::Choose(slot) => (self.slot_setting(slot) == style, None),
                                    Gallery::Pool => (false, Some(pool.contains(&style))),
                                };
                                let resp = viz_tile(ui, &pv, tile, style, selected, check);
                                if selected && scroll_to_current {
                                    resp.scroll_to_me(Some(egui::Align::Center));
                                }
                                if resp.clicked() {
                                    match mode {
                                        Gallery::Choose(_) => chosen = Some(style),
                                        Gallery::Pool => toggle = Some(style),
                                    }
                                }
                            }
                        });
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if mode == Gallery::Pool {
                        if widgets::secondary_button(ui, None, tr("Tout cocher")).clicked() {
                            self.settings.auto_excluded.clear();
                        }
                        ui.label(egui::RichText::new(trf!("{0} sur {1}", pool.len(), AUTO)).color(theme::TEXT_3));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::primary_button(ui, None, tr("Terminé")).clicked() {
                            close = true;
                        }
                    });
                });
                if let (Some(style), Gallery::Choose(slot)) = (chosen, mode) {
                    self.set_slot(slot, style);
                }
                if let Some(style) = toggle {
                    let name = STYLES[style].0.to_owned();
                    let excluded = &mut self.settings.auto_excluded;
                    if excluded.contains(&name) {
                        excluded.retain(|n| *n != name);
                    } else if pool.len() > 1 {
                        // Au moins un visualiseur reste dans le tirage.
                        excluded.push(name);
                    }
                }
            });
        if modal.should_close() || close {
            self.viz_gallery = None;
        }
        ctx.request_repaint();
    }

    /// Raccourcis du visualiseur (page « En cours de lecture ») : V, Maj+V, Échap.
    pub(super) fn viz_keys(&mut self, ctx: &egui::Context) {
        if self.view != View::NowPlaying || !self.settings.visualizer {
            self.viz_strip = false;
            self.viz_strip_tune = false;
            return;
        }
        if (self.viz_strip || self.viz_strip_tune)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.viz_strip = false;
            self.viz_strip_tune = false;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::V)) {
            self.step_viz(-1);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::V)) {
            self.step_viz(1);
        }
    }
}

/// Voir [`SterMusic::previews`].
struct Previews {
    waves: crate::shader::Waves,
    /// Réglages propres à chaque visualiseur.
    tunes: Vec<crate::viz::Tune>,
    pool: Vec<usize>,
    intensity: f32,
    now: f64,
}

/// Un aperçu animé de `style` dans `rect` (coins arrondis).
fn paint_preview(pv: &Previews, painter: &egui::Painter, rect: Rect, style: usize, alpha: f32) {
    let mut w = pv.waves;
    let fill = STYLES[style].1;
    w.mode = style as i32;
    w.radius = 10.0;
    w.intensity = if fill { 0.75 } else { 0.95 } * alpha * pv.intensity;
    if fill {
        w.fade_top = 0.0;
        w.fade_bottom = 0.0;
        w.scale = 0.9;
    } else {
        w.base = if style == 3 || style == 34 { 0.86 } else { 0.78 };
        w.amp = if style == 0 { 0.5 } else { 0.9 };
        w.fade_top = 0.25;
        w.fade_bottom = 0.0;
    }
    // Une « pochette » imaginaire au centre, pour ceux qui tournent autour d'elle.
    w.focus = [rect.width() / 2.0, rect.height() / 2.0, rect.height() * 0.27];
    pv.tunes[style].apply(&mut w, style, rect.size());
    painter.rect_filled(rect, CornerRadius::same(10), Color32::from_black_alpha((90.0 * alpha) as u8));
    crate::shader::paint(painter, rect, w);
}

/// Une vignette : l'aperçu, son nom dessous ; `selected` l'entoure de l'accent, `check` ajoute
/// une case (tirage automatique).
fn viz_tile(
    ui: &mut egui::Ui,
    pv: &Previews,
    size: egui::Vec2,
    style: usize,
    selected: bool,
    check: Option<bool>,
) -> egui::Response {
    let now = pv.now;
    let (rect, resp) = ui.allocate_exact_size(size + vec2(0.0, 24.0), Sense::click());
    let preview = Rect::from_min_size(rect.min, size);
    let hover = resp.hovered();
    let painter = ui.painter_at(rect.expand(4.0));
    let grow = crate::anim::value(
        ui.ctx(),
        ui.id().with(("viz_tile", style)),
        if hover { 1.0 } else { 0.0 },
        0.15,
        crate::anim::Easing::Out,
    );
    let shown = preview.expand(2.0 * grow);
    if style >= AUTO {
        // « Automatique » : les visualiseurs du tirage défilent en fondu.
        let pool = &pv.pool;
        let k = (now / 1.6) as usize;
        let a = pool[k % pool.len()];
        let b = pool[(k + 1) % pool.len()];
        let f = ((now / 1.6).fract() as f32 * 2.0 - 1.0).clamp(0.0, 1.0);
        paint_preview(pv, &painter, shown, a, 1.0 - f);
        if f > 0.0 {
            paint_preview(pv, &painter, shown, b, f);
        }
        icons::paint_centered(&painter, Icon::Shuffle, shown.center(), 26.0, theme::white(0xE0));
    } else {
        paint_preview(pv, &painter, shown, style, if check == Some(false) { 0.35 } else { 1.0 });
    }
    let (stroke, width) = if selected {
        (theme::accent_light(), 2.0_f32)
    } else if hover {
        (theme::white(0x50), 1.0)
    } else {
        (theme::white(0x18), 1.0)
    };
    painter.rect_stroke(shown, CornerRadius::same(10), Stroke::new(width, stroke), StrokeKind::Outside);
    if let Some(on) = check {
        let c = pos2(shown.right() - 14.0, shown.top() + 14.0);
        if on {
            painter.circle_filled(c, 9.0, theme::accent());
            icons::paint_centered(&painter, Icon::Check, c, 12.0, Color32::WHITE);
        } else {
            painter.circle_stroke(c, 8.0, Stroke::new(1.5_f32, theme::white(0xB0)));
        }
    }
    let name = if style >= AUTO { tr("Automatique") } else { tr(STYLES[style].0) };
    painter.text(
        pos2(preview.center().x, preview.bottom() + 13.0),
        Align2::CENTER_CENTER,
        name,
        FontId::proportional(13.0),
        if selected {
            theme::accent_light()
        } else if hover {
            theme::TEXT
        } else {
            theme::TEXT_2
        },
    );
    resp
}

/// Curseurs des réglages propres au visualiseur `style` ; renvoie les nouveaux réglages s'ils
/// ont changé.
fn tune_sliders(ui: &mut egui::Ui, style: usize, tune: crate::viz::Tune) -> Option<crate::viz::Tune> {
    let percent: fn(f32) -> String = |v| crate::i18n::percent(v * 100.0);
    let degrees: fn(f32) -> String = |v| format!("{:+.0}°", v * 360.0);
    // (nom, minimum, maximum, valeur d'origine, texte affiché), dans l'ordre de `values`.
    type Row<'a> = (&'a str, f32, f32, f32, fn(f32) -> String);
    let mut rows: Vec<Row> = vec![
        (tr("Luminosité"), 0.3, 2.0, 1.0, percent),
        (tr("Vitesse"), 0.25, 3.0, 1.0, percent),
        (tr("Réactivité"), 0.0, 2.0, 1.0, percent),
        (tr("Zoom"), 0.5, 2.5, 1.0, percent),
        (tr("Teinte"), -0.5, 0.5, 0.0, degrees),
    ];
    if let Some(label) = crate::shader::DETAILS[style] {
        rows.push((tr(label), 0.0, 1.0, 0.5, percent));
    }
    let mut values = [tune.intensity, tune.speed, tune.react, tune.zoom, tune.hue, tune.detail];
    let mut reset = false;
    for ((label, min, max, origin, show), value) in rows.into_iter().zip(values.iter_mut()) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add_sized([158.0, 24.0], egui::Label::new(egui::RichText::new(label).color(theme::TEXT_2)).truncate());
            // Le curseur est linéaire de part et d'autre de la valeur d'origine, qui tombe au milieu.
            let to_ratio = |v: f32| {
                if v < origin {
                    0.5 * (v - min) / (origin - min)
                } else {
                    0.5 + 0.5 * (v - origin) / (max - origin)
                }
            };
            let from_ratio = |r: f32| {
                if r < 0.5 {
                    min + (origin - min) * r * 2.0
                } else {
                    origin + (max - origin) * (r - 0.5) * 2.0
                }
            };
            let seek = widgets::seek_bar(ui, label, to_ratio(*value), 118.0, true);
            if let Some(r) = seek.preview.or(seek.commit) {
                // Cran magnétique sur la valeur d'origine.
                *value = if (r - 0.5).abs() < 0.03 { origin } else { from_ratio(r) };
            }
            let changed = (*value - origin).abs() > 0.001;
            ui.label(
                egui::RichText::new(show(*value))
                    .small()
                    .color(if changed { theme::accent_light() } else { theme::TEXT_3 }),
            );
        });
    }
    ui.add_space(6.0);
    ui.add_enabled_ui(tune != crate::viz::Tune::default(), |ui| {
        if widgets::secondary_button(ui, Some(Icon::Undo), tr("Réglages d'origine")).clicked() {
            reset = true;
        }
    });
    if reset {
        return Some(crate::viz::Tune::default());
    }
    let t = crate::viz::Tune {
        intensity: values[0],
        speed: values[1],
        react: values[2],
        zoom: values[3],
        hue: values[4],
        detail: values[5],
    };
    (t != tune).then_some(t)
}
