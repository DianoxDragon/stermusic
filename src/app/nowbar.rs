//! Barre de lecture (en bas, comme le lecteur multimédia de Windows 11) et mini lecteur.

use std::time::Duration;

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, UiBuilder,
};

use super::{Action, Repeat, SterMusic, View};
use crate::icons::Icon;
use crate::library::fmt_duration;
use crate::{theme, widgets};

pub const NOWBAR_HEIGHT: f32 = 96.0;

struct NowState {
    track: Option<usize>,
    pos: f32,
    total: f32,
    playing: bool,
    has_track: bool,
}

impl SterMusic {
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
                painter.rect_filled(rect, 0.0, theme::alpha(theme::bg(), 0x90));
                painter.hline(
                    rect.x_range(),
                    rect.top(),
                    Stroke::new(1.0_f32, theme::stroke()),
                );

                let st = self.now_state();

                // Progression, sur toute la largeur, avec les temps aux deux bouts.
                let bar_y = rect.top() + 16.0;
                let time_w = 52.0;
                let enabled = st.has_track && st.total > 0.0;
                let bar_rect = Rect::from_min_max(
                    pos2(rect.left() + 20.0 + time_w, bar_y - 10.0),
                    pos2(rect.right() - 20.0 - time_w, bar_y + 10.0),
                );
                let value = if st.total > 0.0 {
                    st.pos / st.total
                } else {
                    0.0
                };
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
                painter.text(
                    pos2(rect.right() - 20.0, bar_y),
                    Align2::RIGHT_CENTER,
                    if st.total > 0.0 {
                        fmt_duration(Duration::from_secs_f32(st.total))
                    } else {
                        "0:00".into()
                    },
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
                let cover_rect =
                    Rect::from_center_size(pos2(row.left() + 26.0, cy), vec2(52.0, 52.0));
                match st.track {
                    Some(i) => {
                        let t = &self.lib.tracks[i];
                        let tex = self.covers.thumb(&t.path).cloned();
                        let resp =
                            ui.interact(cover_rect, ui.id().with("nowbar_cover"), Sense::click());
                        let lift = widgets::hover_t(ui, resp.id, resp.hovered());
                        let r = cover_rect.expand(1.5 * lift);
                        widgets::cover(ui.painter(), r, tex.as_ref(), 6, &t.album);
                        if resp.clicked() {
                            self.actions.push(if self.view == View::NowPlaying {
                                Action::Back
                            } else {
                                Action::Go(View::NowPlaying)
                            });
                        }
                        resp.on_hover_text("En cours de lecture");
                        let tx = cover_rect.right() + 12.0;
                        let max_w = info_w - 70.0;
                        let title = widgets::galley(
                            ui,
                            &t.title,
                            theme::semibold(14.0),
                            theme::TEXT,
                            max_w,
                        );
                        let title_rect = Rect::from_min_size(pos2(tx, cy - 19.0), title.size());
                        ui.painter().galley(title_rect.min, title, theme::TEXT);
                        let artist = widgets::galley(
                            ui,
                            t.artist_or_unknown(),
                            FontId::proportional(12.5),
                            theme::TEXT_2,
                            max_w,
                        );
                        let artist_rect = Rect::from_min_size(pos2(tx, cy + 3.0), artist.size());
                        let ar =
                            ui.interact(artist_rect, ui.id().with("nowbar_artist"), Sense::click());
                        let col = if ar.hovered() {
                            theme::accent_light()
                        } else {
                            theme::TEXT_2
                        };
                        ui.painter()
                            .galley_with_override_text_color(artist_rect.min, artist, col);
                        if ar.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            let album = self.lib.track_album[i];
                            self.actions
                                .push(Action::Go(View::Artist(self.lib.album_artist[album])));
                        }
                    }
                    None => {
                        widgets::cover(ui.painter(), cover_rect, None, 6, "");
                        ui.painter().text(
                            pos2(cover_rect.right() + 12.0, cy),
                            Align2::LEFT_CENTER,
                            "Rien en lecture",
                            FontId::proportional(13.0),
                            theme::TEXT_3,
                        );
                    }
                }

                // Centre : commandes.
                let can_play =
                    self.player.is_some() && (!self.lib.is_empty() || st.track.is_some());
                let controls =
                    Rect::from_center_size(pos2(row.center().x, cy), vec2(5.0 * 44.0 + 12.0, 52.0));
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
                let right = Rect::from_min_max(pos2(row.right() - 300.0, row.top()), row.max);
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(right)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let full = self.view == View::NowPlaying;
                        let (icon, tip) = if full {
                            (Icon::Minimize2, "Quitter « En cours de lecture »")
                        } else {
                            (Icon::Maximize, "En cours de lecture")
                        };
                        if widgets::icon_button(ui, icon, tip, 36.0, 17.0).clicked() {
                            self.actions.push(if full {
                                Action::Back
                            } else {
                                Action::Go(View::NowPlaying)
                            });
                        }
                        ui.add_space(6.0);
                        self.volume(ui, 110.0);
                        ui.add_space(6.0);
                        if let Some(i) = st.track {
                            let fav = self.settings.favorites.contains(&self.lib.tracks[i].path);
                            let (icon, tip) = if fav {
                                (Icon::HeartFilled, "Retirer des favoris")
                            } else {
                                (Icon::Heart, "Ajouter aux favoris")
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
                "Aléatoire : activé"
            } else {
                "Aléatoire : désactivé"
            },
            size,
            17.0,
        )
        .clicked()
        {
            self.actions.push(Action::ToggleShuffle);
        }
        if widgets::icon_button(ui, Icon::SkipBack, "Précédent (Ctrl+←)", size, 19.0).clicked()
        {
            self.actions.push(Action::Previous);
        }
        if widgets::play_button(ui, playing, play_size).clicked() {
            self.actions.push(Action::TogglePlay);
        }
        if widgets::icon_button(ui, Icon::SkipForward, "Suivant (Ctrl+→)", size, 19.0).clicked() {
            self.actions.push(Action::Next);
        }
        let (icon, tip) = match self.settings.repeat {
            Repeat::Off => (Icon::Repeat, "Répétition : désactivée"),
            Repeat::All => (Icon::Repeat, "Répétition : toute la file"),
            Repeat::One => (Icon::Repeat1, "Répétition : ce morceau"),
        };
        if widgets::toggle_icon(
            ui,
            self.settings.repeat != Repeat::Off,
            icon,
            tip,
            size,
            17.0,
        )
        .clicked()
        {
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
        let tip = format!("Volume : {:.0} % (clic : couper le son)", vol * 100.0);
        if widgets::icon_button(ui, icon, &tip, 34.0, 17.0).clicked() {
            self.actions.push(Action::ToggleMute);
        }
    }

    /// Mini lecteur : une petite fenêtre toujours au premier plan.
    pub(super) fn mini_player(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                super::chrome::drag_area(ui, rect, "mini_drag");
                let st = self.now_state();
                let pad = 12.0;
                // Pochette bornée : la mise en page tient même si la fenêtre n'a pas encore rétréci.
                let side = (rect.height() - 2.0 * pad).min(124.0);
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
                        offset: [0, 4],
                        blur: 14,
                        spread: 0,
                        color: Color32::from_black_alpha(90),
                    }
                    .as_shape(cover, CornerRadius::same(8)),
                );
                widgets::cover(ui.painter(), cover, tex.as_ref(), 8, &seed);

                let x = cover.right() + 14.0;
                let text_w = rect.right() - x - 44.0;
                let (title, artist) = match st.track {
                    Some(i) => (
                        self.lib.tracks[i].title.clone(),
                        self.lib.tracks[i].artist_or_unknown().to_owned(),
                    ),
                    None => ("Rien en lecture".into(), String::new()),
                };
                let g = widgets::galley(ui, &title, theme::semibold(15.0), theme::TEXT, text_w);
                ui.painter()
                    .galley(pos2(x, rect.top() + pad + 2.0), g, theme::TEXT);
                let g = widgets::galley(
                    ui,
                    &artist,
                    FontId::proportional(12.5),
                    theme::TEXT_2,
                    text_w,
                );
                ui.painter()
                    .galley(pos2(x, rect.top() + pad + 24.0), g, theme::TEXT_2);

                // Revenir à la fenêtre complète.
                let back = Rect::from_min_size(
                    pos2(rect.right() - 40.0, rect.top() + 6.0),
                    vec2(34.0, 34.0),
                );
                if ui
                    .put(back, |ui: &mut egui::Ui| {
                        widgets::icon_button(ui, Icon::Maximize, "Agrandir (Ctrl+M)", 34.0, 16.0)
                    })
                    .clicked()
                {
                    self.actions.push(Action::ToggleMini);
                }

                let controls = Rect::from_min_max(
                    pos2(x - 6.0, rect.top() + pad + 46.0),
                    pos2(rect.right() - pad, rect.top() + pad + 90.0),
                );
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(controls)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if widgets::icon_button(ui, Icon::SkipBack, "Précédent", 34.0, 17.0)
                            .clicked()
                        {
                            self.actions.push(Action::Previous);
                        }
                        if widgets::play_button(ui, st.playing, 40.0).clicked() {
                            self.actions.push(Action::TogglePlay);
                        }
                        if widgets::icon_button(ui, Icon::SkipForward, "Suivant", 34.0, 17.0)
                            .clicked()
                        {
                            self.actions.push(Action::Next);
                        }
                    },
                );

                let bar = Rect::from_min_max(
                    pos2(x, cover.bottom() - 18.0),
                    pos2(rect.right() - pad, cover.bottom() + 2.0),
                );
                let value = if st.total > 0.0 {
                    st.pos / st.total
                } else {
                    0.0
                };
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
            });
    }
}
