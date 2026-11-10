//! Barre de lecture (en bas, comme le lecteur multimédia de Windows 11) et mini lecteur.

use std::time::Duration;

use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, UiBuilder};

use super::{Action, Repeat, SterMusic, View};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::library::fmt_duration;
use crate::trf;
use crate::{anim, theme, widgets};

pub const NOWBAR_HEIGHT: f32 = 96.0;

struct NowState {
    track: Option<usize>,
    /// Webradio en cours (ou en connexion) : la station, le titre diffusé, l'état.
    radio: Option<RadioNow>,
    pos: f32,
    total: f32,
    playing: bool,
    has_track: bool,
}

#[derive(Clone)]
struct RadioNow {
    station: crate::radio::Station,
    title: String,
    status: String,
}

impl SterMusic {
    fn radio_now(&self) -> Option<RadioNow> {
        if let Some(s) = self.radio.connecting_to() {
            return Some(RadioNow {
                station: s.clone(),
                title: s.name.clone(),
                status: tr("Connexion…").into(),
            });
        }
        let live = self.radio.live.as_ref()?;
        let title = live.info.title();
        let status = if live.paused {
            tr("Arrêtée").to_owned()
        } else {
            tr("En direct").to_owned()
        };
        Some(if title.is_empty() {
            RadioNow {
                station: live.station.clone(),
                title: live.station.name.clone(),
                status,
            }
        } else {
            RadioNow {
                station: live.station.clone(),
                title,
                status: format!("{} · {}", live.station.name, status),
            }
        })
    }

    fn now_state(&self) -> NowState {
        let track = self.current();
        let (pos, playing, has_track) = match &self.player {
            Some(p) => (p.position().as_secs_f32(), p.is_playing(), p.has_track()),
            None => (0.0, false, false),
        };
        let total = track
            .map(|i| self.lib.tracks[i].duration().as_secs_f32())
            .unwrap_or(0.0);
        NowState {
            track,
            radio: self.radio_now(),
            pos: pos.min(total.max(pos)),
            total,
            playing,
            has_track,
        }
    }

    pub(super) fn now_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("now_bar")
            .exact_height(NOWBAR_HEIGHT)
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let painter = ui.painter();
                // Les coins du bas touchent ceux de la fenêtre.
                let r = theme::window_radius();
                painter.rect_filled(
                    rect,
                    CornerRadius {
                        sw: r,
                        se: r,
                        ..Default::default()
                    },
                    theme::alpha(theme::bg(), 0x90),
                );
                painter.hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, theme::stroke()));

                let st = self.now_state();

                // Progression, sur toute la largeur, avec les temps aux deux bouts.
                let bar_y = rect.top() + 16.0;
                let time_w = 52.0;
                let enabled = st.has_track && st.total > 0.0;
                let bar_rect = Rect::from_min_max(
                    pos2(rect.left() + 20.0 + time_w, bar_y - 10.0),
                    pos2(rect.right() - 20.0 - time_w, bar_y + 10.0),
                );
                let value = if st.total > 0.0 { st.pos / st.total } else { 0.0 };
                let seek = ui
                    .scope_builder(UiBuilder::new().max_rect(bar_rect), |ui| {
                        widgets::seek_bar(ui, "seek", value, bar_rect.width(), enabled)
                    })
                    .inner;
                if let Some(v) = seek.commit {
                    self.actions.push(Action::SeekTo(v * st.total));
                }
                let shown = seek.preview.map(|v| v * st.total).unwrap_or(st.pos);
                let time_font = FontId::proportional(12.0);
                let painter = ui.painter();
                if st.radio.is_some() {
                    // Direct : pastille rouge et « EN DIRECT » à la place du temps écoulé.
                    let dot = pos2(rect.left() + 24.0, bar_y);
                    let pulse = if st.playing {
                        0.6 + 0.4 * (ui.input(|i| i.time) as f32 * 3.0).sin().abs()
                    } else {
                        0.4
                    };
                    painter.circle_filled(
                        dot,
                        4.0,
                        Color32::from_rgb(0xE5, 0x48, 0x4D).gamma_multiply(pulse),
                    );
                    painter.text(
                        pos2(dot.x + 9.0, bar_y),
                        Align2::LEFT_CENTER,
                        tr("DIRECT"),
                        FontId::proportional(10.5),
                        theme::TEXT_2,
                    );
                } else {
                    painter.text(
                        pos2(rect.left() + 20.0, bar_y),
                        Align2::LEFT_CENTER,
                        if st.has_track {
                            fmt_duration(Duration::from_secs_f32(shown))
                        } else {
                            "0:00".into()
                        },
                        time_font.clone(),
                        theme::TEXT_2,
                    );
                }
                let total_text = if st.radio.is_some() {
                    String::new()
                } else if st.total > 0.0 {
                    fmt_duration(Duration::from_secs_f32(st.total))
                } else {
                    "0:00".into()
                };
                painter.text(
                    pos2(rect.right() - 20.0, bar_y),
                    Align2::RIGHT_CENTER,
                    total_text,
                    time_font,
                    theme::TEXT_2,
                );

                let row = Rect::from_min_max(
                    pos2(rect.left() + 16.0, bar_y + 12.0),
                    pos2(rect.right() - 16.0, rect.bottom() - 6.0),
                );
                let cy = row.center().y;

                // Gauche : pochette, titre, artiste.
                let info_w = (row.width() * 0.30).min(360.0);
                let cover_rect = Rect::from_center_size(pos2(row.left() + 26.0, cy), vec2(52.0, 52.0));
                if let Some(radio) = &st.radio {
                    self.radio_info(ui, cover_rect, info_w, radio);
                }
                match st.track.filter(|_| st.radio.is_none()) {
                    Some(i) => {
                        let t = self.lib.tracks[i].clone();
                        let tex = self.covers.thumb(&t.path).cloned();
                        let tx = cover_rect.right() + 12.0;
                        let max_w = info_w - 70.0;
                        let title = widgets::galley(ui, &t.title, theme::semibold(14.0), theme::TEXT, max_w);
                        let artist = widgets::galley(
                            ui,
                            t.artist_or_unknown(),
                            FontId::proportional(12.5),
                            theme::TEXT_2,
                            max_w,
                        );

                        // Pochette, titre et artiste : un seul bouton vers « En cours de lecture ».
                        let text_w = title.size().x.max(artist.size().x);
                        let zone = Rect::from_min_max(
                            pos2(cover_rect.left() - 6.0, cover_rect.top() - 5.0),
                            pos2(tx + text_w + 12.0, cover_rect.bottom() + 5.0),
                        );
                        let resp = ui.interact(zone, ui.id().with("nowbar_info"), Sense::click());
                        let hover = widgets::hover_t(ui, resp.id, resp.hovered());
                        if hover > 0.0 {
                            ui.painter().rect_filled(
                                zone,
                                CornerRadius::same(8),
                                theme::white((0x10 as f32 * hover) as u8),
                            );
                        }
                        if resp.is_pointer_button_down_on() {
                            ui.painter()
                                .rect_filled(zone, CornerRadius::same(8), theme::white(0x08));
                        }
                        let full = self.view == View::NowPlaying;
                        if resp.clicked() {
                            self.actions.push(if full {
                                Action::Back
                            } else {
                                Action::Go(View::NowPlaying)
                            });
                        }
                        let resp = resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(if full {
                                tr("Quitter « En cours de lecture »")
                            } else {
                                tr("En cours de lecture")
                            });
                        let _ = resp;

                        // Changement de morceau : la pochette et les textes arrivent en fondu, en glissant.
                        let appear = anim::value_from(
                            ui.ctx(),
                            egui::Id::new(("nowbar_track", i)),
                            0.0,
                            1.0,
                            anim::SLOW,
                            anim::Easing::OutExpo,
                        );
                        let lift = 1.5 * hover;
                        let r = cover_rect.expand(lift);
                        widgets::cover(ui.painter(), r, tex.as_ref(), 6, &t.album);
                        let slide = vec2(10.0 * (1.0 - appear), 0.0);
                        let title_pos = pos2(tx, cy - 19.0) + slide;
                        ui.painter().galley_with_override_text_color(
                            title_pos,
                            title,
                            theme::TEXT.gamma_multiply(appear),
                        );

                        // Le nom de l'artiste reste un lien vers sa page (il passe devant la zone).
                        let artist_rect = Rect::from_min_size(pos2(tx, cy + 3.0) + slide, artist.size());
                        let ar = ui.interact(artist_rect, ui.id().with("nowbar_artist"), Sense::click());
                        let col = if ar.hovered() {
                            theme::accent_light()
                        } else {
                            theme::TEXT_2
                        };
                        ui.painter().galley_with_override_text_color(
                            artist_rect.min,
                            artist,
                            col.gamma_multiply(appear),
                        );
                        if ar.hovered() {
                            let y = artist_rect.bottom();
                            ui.painter()
                                .hline(artist_rect.x_range(), y, Stroke::new(1.0_f32, col));
                        }
                        if ar
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(tr("Voir l'artiste"))
                            .clicked()
                        {
                            let album = self.lib.track_album[i];
                            self.actions
                                .push(Action::Go(View::Artist(self.lib.album_artist[album])));
                        }
                    }
                    None if st.radio.is_some() => {}
                    None => {
                        widgets::cover(ui.painter(), cover_rect, None, 6, "");
                        ui.painter().text(
                            pos2(cover_rect.right() + 12.0, cy),
                            Align2::LEFT_CENTER,
                            tr("Rien en lecture"),
                            FontId::proportional(13.0),
                            theme::TEXT_3,
                        );
                    }
                }

                // Centre : commandes.
                let can_play = self.player.is_some()
                    && (!self.lib.is_empty() || st.track.is_some() || st.radio.is_some());
                let controls =
                    Rect::from_center_size(pos2(row.center().x, cy), vec2(6.0 * 44.0 + 12.0, 52.0));
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(controls)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add_enabled_ui(can_play, |ui| self.controls(ui, st.playing, 40.0, 46.0));
                    },
                );

                // Droite : favori, volume, en cours de lecture.
                let right = Rect::from_min_max(pos2(row.right() - 340.0, row.top()), row.max);
                // Fenêtre étroite : la barre de volume raccourcit pour ne pas toucher les commandes.
                let vol_w = if row.width() < 980.0 { 64.0 } else { 110.0 };
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(right)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.add_space(4.0);
                        // Égaliseur, dans une bulle au-dessus du bouton.
                        let eq = widgets::toggle_icon(
                            ui,
                            self.settings.eq_on,
                            Icon::Equalizer,
                            tr("Égaliseur"),
                            36.0,
                            17.0,
                        );
                        let popup_id = ui.make_persistent_id("eq_popup");
                        if eq.clicked() {
                            ui.memory_mut(|m| m.toggle_popup(popup_id));
                        }
                        egui::popup::popup_above_or_below_widget(
                            ui,
                            popup_id,
                            &eq,
                            egui::AboveOrBelow::Above,
                            egui::PopupCloseBehavior::CloseOnClickOutside,
                            |ui| {
                                ui.set_width(470.0);
                                ui.add_space(6.0);
                                self.equalizer_panel(ui);
                                ui.add_space(6.0);
                            },
                        );
                        ui.add_space(2.0);
                        self.sleep_button(ui);
                        ui.add_space(6.0);
                        self.volume(ui, vol_w);
                        ui.add_space(6.0);
                        if let Some(radio) = &st.radio {
                            let saved = self.settings.radios.iter().any(|r| r.url == radio.station.url);
                            let (icon, tip) = if saved {
                                (Icon::HeartFilled, tr("Retirer de mes stations"))
                            } else {
                                (Icon::Heart, tr("Ajouter à mes stations"))
                            };
                            if widgets::toggle_icon(ui, saved, icon, tip, 36.0, 17.0).clicked() {
                                self.actions.push(Action::ToggleStation(radio.station.clone()));
                            }
                        } else if let Some(i) = st.track {
                            let fav = self.settings.favorites.contains(&self.lib.tracks[i].path);
                            let (icon, tip) = if fav {
                                (Icon::HeartFilled, tr("Retirer des favoris"))
                            } else {
                                (Icon::Heart, tr("Ajouter aux favoris"))
                            };
                            if widgets::toggle_icon(ui, fav, icon, tip, 36.0, 17.0).clicked() {
                                self.actions.push(Action::ToggleFavorite(i));
                            }
                        }
                    },
                );
            });
    }

    /// Aléatoire, précédent, lecture, suivant, répétition.
    fn controls(&mut self, ui: &mut egui::Ui, playing: bool, size: f32, play_size: f32) {
        if widgets::toggle_icon(
            ui,
            self.settings.shuffle,
            Icon::Shuffle,
            if self.settings.shuffle {
                tr("Aléatoire : activé")
            } else {
                tr("Aléatoire : désactivé")
            },
            size,
            17.0,
        )
        .clicked()
        {
            self.actions.push(Action::ToggleShuffle);
        }
        if widgets::icon_button(ui, Icon::SkipBack, tr("Précédent (Ctrl+←)"), size, 19.0).clicked() {
            self.actions.push(Action::Previous);
        }
        if widgets::play_button(ui, playing, play_size).clicked() {
            self.actions.push(Action::TogglePlay);
        }
        if widgets::icon_button(ui, Icon::SkipForward, tr("Suivant (Ctrl+→)"), size, 19.0).clicked() {
            self.actions.push(Action::Next);
        }
        // Arrêter : plus rien en lecture, la file d'attente reste.
        let active = self.player.as_ref().is_some_and(|p| p.has_track())
            || self.radio.live.is_some()
            || self.radio.connecting_to().is_some();
        let stop = ui.add_enabled_ui(active, |ui| {
            widgets::icon_button(
                ui,
                Icon::MediaStop,
                tr("Arrêter (la file d'attente est gardée)"),
                size,
                16.0,
            )
        });
        if stop.inner.clicked() {
            self.actions.push(Action::Stop);
        }
        let (icon, tip) = match self.settings.repeat {
            Repeat::Off => (Icon::Repeat, tr("Répétition : désactivée")),
            Repeat::All => (Icon::Repeat, tr("Répétition : toute la file")),
            Repeat::One => (Icon::Repeat1, tr("Répétition : ce morceau")),
        };
        if widgets::toggle_icon(ui, self.settings.repeat != Repeat::Off, icon, tip, size, 17.0).clicked() {
            self.actions.push(Action::CycleRepeat);
        }
    }

    /// Bouton muet + barre de volume (mise en page de droite à gauche).
    fn volume(&mut self, ui: &mut egui::Ui, width: f32) {
        let vol = if self.settings.muted {
            0.0
        } else {
            self.settings.volume
        };
        let seek = widgets::seek_bar(ui, "volume", vol, width, true);
        if let Some(v) = seek.preview.or(seek.commit) {
            if (v - self.settings.volume).abs() > f32::EPSILON || self.settings.muted {
                self.actions.push(Action::SetVolume(v));
            }
        }
        let icon = match vol {
            v if v <= 0.0 => Icon::VolumeX,
            v if v < 0.34 => Icon::Volume0,
            v if v < 0.67 => Icon::Volume1,
            _ => Icon::Volume2,
        };
        let tip = trf!(
            "Volume : {0} % (clic : couper le son)",
            format!("{:.0}", vol * 100.0)
        );
        let resp = widgets::icon_button(ui, icon, &tip, 34.0, 17.0);
        if resp.clicked() {
            self.actions.push(Action::ToggleMute);
        }
        // Clic droit : choisir la sortie audio.
        resp.context_menu(|ui| {
            ui.label(
                egui::RichText::new(tr("Sortie audio"))
                    .small()
                    .color(theme::TEXT_3),
            );
            self.output_menu(ui);
        });
    }

    /// Bouton de la minuterie de sommeil, avec le temps restant quand elle tourne.
    fn sleep_button(&mut self, ui: &mut egui::Ui) {
        let remaining = self.sleep_remaining();
        let tip = match remaining {
            Some(r) => trf!(
                "Minuterie de sommeil : arrêt dans {0}",
                super::settings_extra::fmt_remaining(r)
            ),
            None => tr("Minuterie de sommeil").to_owned(),
        };
        let on = self.sleep.is_some();
        let resp = widgets::toggle_icon(ui, on, Icon::Moon, &tip, 36.0, 17.0);
        let popup_id = ui.make_persistent_id("sleep_popup");
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup_id));
        }
        egui::popup::popup_above_or_below_widget(
            ui,
            popup_id,
            &resp,
            egui::AboveOrBelow::Above,
            egui::PopupCloseBehavior::CloseOnClick,
            |ui| self.sleep_menu(ui),
        );
        if let Some(r) = remaining.filter(|_| ui.available_width() > 60.0) {
            ui.label(
                egui::RichText::new(super::settings_extra::fmt_remaining(r))
                    .small()
                    .color(theme::accent_light()),
            );
        }
    }

    /// Station en cours dans la barre de lecture : logo, titre diffusé, nom de la station.
    fn radio_info(&mut self, ui: &mut egui::Ui, cover_rect: Rect, info_w: f32, radio: &RadioNow) {
        let tx = cover_rect.right() + 12.0;
        let max_w = info_w - 70.0;
        let cy = cover_rect.center().y;
        let title = widgets::galley(ui, &radio.title, theme::semibold(14.0), theme::TEXT, max_w);
        let sub = widgets::galley(
            ui,
            &radio.status,
            FontId::proportional(12.5),
            theme::TEXT_2,
            max_w,
        );
        let text_w = title.size().x.max(sub.size().x);
        let zone = Rect::from_min_max(
            pos2(cover_rect.left() - 6.0, cover_rect.top() - 5.0),
            pos2(tx + text_w + 12.0, cover_rect.bottom() + 5.0),
        );
        let resp = ui.interact(zone, ui.id().with("nowbar_radio"), Sense::click());
        let hover = widgets::hover_t(ui, resp.id, resp.hovered());
        if hover > 0.0 {
            ui.painter().rect_filled(
                zone,
                CornerRadius::same(8),
                theme::white((0x10 as f32 * hover) as u8),
            );
        }
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr("Webradios"))
            .clicked()
        {
            self.actions.push(Action::Go(View::Page(super::Page::Radio)));
        }
        self.paint_logo(ui, cover_rect, &radio.station);
        let appear = anim::value_from(
            ui.ctx(),
            egui::Id::new(("nowbar_radio_title", &radio.title)),
            0.0,
            1.0,
            anim::SLOW,
            anim::Easing::OutExpo,
        );
        let slide = vec2(10.0 * (1.0 - appear), 0.0);
        ui.painter().galley_with_override_text_color(
            pos2(tx, cy - 19.0) + slide,
            title,
            theme::TEXT.gamma_multiply(appear),
        );
        ui.painter()
            .galley_with_override_text_color(pos2(tx, cy + 3.0), sub, theme::TEXT_2);
    }

    /// Mini lecteur compact : pochette, titre, artiste, commandes et progression sur une ligne.
    /// Il reste au premier plan (script KWin sous Plasma).
    pub(super) fn mini_player(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                super::chrome::drag_area(ui, rect, "mini_drag");
                let st = self.now_state();
                let pad = 10.0;
                // Pochette carrée, bornée : la mise en page tient même si la fenêtre n'a pas encore rétréci.
                let side = (rect.height() - 2.0 * pad).clamp(48.0, 110.0);
                let cover = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(side, side));
                let tex = st
                    .track
                    .and_then(|i| self.covers.thumb(&self.lib.tracks[i].path).cloned());
                let seed = st
                    .track
                    .map(|i| self.lib.tracks[i].album.clone())
                    .unwrap_or_default();
                ui.painter().add(
                    egui::Shadow {
                        offset: [0, 3],
                        blur: 12,
                        spread: 0,
                        color: Color32::from_black_alpha(90),
                    }
                    .as_shape(cover, CornerRadius::same(8)),
                );
                widgets::cover(ui.painter(), cover, tex.as_ref(), 8, &seed);

                let x = cover.right() + 12.0;
                let right = rect.right().min(rect.left() + 560.0) - pad;
                let text_w = right - x - 34.0;
                let (title, artist) = match (st.track, &st.radio) {
                    (_, Some(r)) => (r.title.clone(), r.status.clone()),
                    (Some(i), None) => (
                        self.lib.tracks[i].title.clone(),
                        self.lib.tracks[i].artist_or_unknown().to_owned(),
                    ),
                    (None, None) => (tr("Rien en lecture").into(), String::new()),
                };
                let g = widgets::galley(ui, &title, theme::semibold(14.0), theme::TEXT, text_w);
                ui.painter().galley(pos2(x, cover.top() + 2.0), g, theme::TEXT);
                let g = widgets::galley(ui, &artist, FontId::proportional(12.5), theme::TEXT_2, text_w);
                ui.painter().galley(pos2(x, cover.top() + 22.0), g, theme::TEXT_2);

                // Revenir à la fenêtre complète.
                let back = Rect::from_min_size(pos2(right - 30.0, cover.top() - 4.0), vec2(30.0, 30.0));
                if ui
                    .put(back, |ui: &mut egui::Ui| {
                        widgets::icon_button(ui, Icon::Maximize, tr("Agrandir (Ctrl+M)"), 30.0, 15.0)
                    })
                    .clicked()
                {
                    self.actions.push(Action::ToggleMini);
                }

                // Ligne du bas : précédent, lecture, suivant, puis la progression.
                let row_h = 38.0;
                let row =
                    Rect::from_min_max(pos2(x - 4.0, cover.bottom() - row_h), pos2(right, cover.bottom()));
                let controls_w = 30.0 + 36.0 + 30.0 + 8.0;
                let controls = Rect::from_min_size(row.min, vec2(controls_w, row_h));
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(controls)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if widgets::icon_button(ui, Icon::SkipBack, tr("Précédent"), 30.0, 15.0).clicked() {
                            self.actions.push(Action::Previous);
                        }
                        if widgets::play_button(ui, st.playing, 36.0).clicked() {
                            self.actions.push(Action::TogglePlay);
                        }
                        if widgets::icon_button(ui, Icon::SkipForward, tr("Suivant"), 30.0, 15.0).clicked() {
                            self.actions.push(Action::Next);
                        }
                    },
                );
                let bar = Rect::from_min_max(
                    pos2(controls.right() + 8.0, row.center().y - 10.0),
                    pos2(row.right(), row.center().y + 10.0),
                );
                if bar.width() > 40.0 {
                    let value = if st.total > 0.0 { st.pos / st.total } else { 0.0 };
                    let seek = ui
                        .scope_builder(UiBuilder::new().max_rect(bar), |ui| {
                            widgets::seek_bar(
                                ui,
                                "mini_seek",
                                value,
                                bar.width(),
                                st.has_track && st.total > 0.0,
                            )
                        })
                        .inner;
                    if let Some(v) = seek.commit {
                        self.actions.push(Action::SeekTo(v * st.total));
                    }
                }
            });
    }
}
