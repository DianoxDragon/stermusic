//! Diffusion vers un appareil du réseau : le bouton de la barre de lecture, la liste des
//! appareils, et ce que l'appareil affiche (titre, artiste, pochette).

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, RichText};

use super::{Action, SterMusic};
use crate::cast::{Device, Meta};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::trf;
use crate::{theme, widgets};

#[derive(Default)]
pub struct CastState {
    pub devices: Vec<Device>,
    scanning: Option<Receiver<Vec<Device>>>,
    scanned_at: Option<f64>,
    last_error: Option<String>,
}

impl SterMusic {
    /// Ce que l'appareil affiche pour un morceau de la bibliothèque.
    pub(super) fn cast_meta(&mut self, idx: usize) -> Meta {
        let t = self.lib.tracks[idx].clone();
        let cover_url = t
            .remote
            .as_ref()
            .map(|r| r.cover.clone())
            .filter(|c| !c.is_empty());
        let cover_file = if cover_url.is_none() && self.player.as_ref().is_some_and(|p| p.casting().is_some())
        {
            self.covers.large(&t.path).and_then(|l| l.file.clone())
        } else {
            None
        };
        Meta {
            title: t.title.clone(),
            artist: t.artist_or_unknown().to_owned(),
            album: t.album_or_unknown().to_owned(),
            cover_file,
            cover_url,
            live: false,
        }
    }

    fn scan_devices(&mut self, ctx: &egui::Context) {
        if self.cast.scanning.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::cast::discover());
            ctx.request_repaint();
        });
        self.cast.scanning = Some(rx);
        self.cast.scanned_at = Some(self.egui_ctx.input(|i| i.time));
    }

    pub(super) fn poll_cast(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.cast.scanning {
            if let Ok(list) = rx.try_recv() {
                self.cast.scanning = None;
                self.cast.devices = list;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
        let error = self.player.as_ref().and_then(|p| p.cast_error());
        if error.is_some() && error != self.cast.last_error {
            self.cast.last_error = error.clone();
            if let Some(e) = error {
                self.toast(e);
            }
        }
        if self.player.as_ref().is_some_and(|p| p.casting().is_some()) {
            // L'état de l'appareil change sans que l'interface le sache : relevé régulier.
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    pub(super) fn start_cast(&mut self, ctx: &egui::Context, k: usize) {
        let Some(device) = self.cast.devices.get(k).cloned() else {
            return;
        };
        if let Some(i) = self.current() {
            let meta = self.cast_meta(i);
            if let Some(p) = &mut self.player {
                p.set_meta(meta);
            }
        }
        let Some(player) = &mut self.player else { return };
        let name = device.name.clone();
        match player.start_cast(device) {
            Ok(()) => {
                self.cast.last_error = None;
                self.toast(trf!("Diffusion sur {0}", name));
                // Une webradio en cours se relance sur l'appareil.
                if let Some(live) = self.radio.live.as_ref().filter(|l| !l.paused) {
                    let station = live.station.clone();
                    self.play_station(ctx, station);
                }
            }
            Err(e) => self.toast(e),
        }
        self.seeked = true;
    }

    pub(super) fn stop_cast(&mut self, ctx: &egui::Context) {
        let radio = self
            .radio
            .live
            .as_ref()
            .filter(|l| !l.paused)
            .map(|l| l.station.clone());
        if let Some(p) = &mut self.player {
            p.stop_cast();
        }
        if let Some(station) = radio {
            self.play_station(ctx, station);
        }
        self.toast(tr("Lecture sur cet ordinateur"));
        self.seeked = true;
    }

    /// Webradio diffusée : on cherche seulement l'adresse du flux ; l'appareil la lit.
    pub(super) fn play_station_cast(&mut self, ctx: &egui::Context, station: crate::radio::Station) {
        let (tx, rx) = channel();
        let (s, ctx) = (station.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(crate::radio::resolve(&s.url));
            ctx.request_repaint();
        });
        self.radio.cast_rx = Some((station, rx));
    }

    /// Adresse du flux trouvée : l'appareil la lit, la station devient la station en cours.
    pub(super) fn cast_station_ready(&mut self, station: crate::radio::Station, url: String) {
        self.finish_listen();
        self.leave_episode();
        if let Some(p) = &mut self.player {
            p.set_meta(Meta {
                title: station.name.clone(),
                artist: tr("Webradio").into(),
                cover_url: Some(station.favicon.clone()).filter(|f| !f.is_empty()),
                live: true,
                ..Default::default()
            });
            p.cast_live(&url);
        }
        let info = crate::radio::LiveInfo {
            title: Arc::new(Mutex::new(String::new())),
            server_name: String::new(),
            stop: Arc::new(AtomicBool::new(false)),
        };
        self.radio.set_live(station, info);
        self.seeked = true;
    }

    /// Bouton « Diffuser » et sa liste d'appareils.
    pub(super) fn cast_button(&mut self, ui: &mut egui::Ui) {
        let casting = self.player.as_ref().and_then(|p| p.casting()).cloned();
        let tip = match &casting {
            Some(d) => trf!("Diffusion sur {0}", d.name),
            None => tr("Diffuser sur un appareil (TV, enceinte, Chromecast)").to_owned(),
        };
        let resp = widgets::toggle_icon(ui, casting.is_some(), Icon::Cast, &tip, 36.0, 17.0);
        let popup_id = ui.make_persistent_id("cast_popup");
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup_id));
            // La liste est refaite à l'ouverture si elle date de plus de trente secondes.
            let now = ui.input(|i| i.time);
            if self.cast.scanned_at.is_none_or(|t| now - t > 30.0) {
                let ctx = ui.ctx().clone();
                self.scan_devices(&ctx);
            }
        }
        egui::popup::popup_above_or_below_widget(
            ui,
            popup_id,
            &resp,
            egui::AboveOrBelow::Above,
            egui::PopupCloseBehavior::CloseOnClick,
            |ui| {
                ui.set_min_width(280.0);
                ui.label(RichText::new(tr("Diffuser sur…")).font(theme::semibold(15.0)));
                ui.add_space(4.0);
                if widgets::menu_item(
                    ui,
                    Some(if casting.is_none() {
                        Icon::Speaker
                    } else {
                        Icon::Volume2
                    }),
                    tr("Cet ordinateur"),
                )
                .clicked()
                    && casting.is_some()
                {
                    self.actions.push(Action::StopCast);
                }
                for (k, d) in self.cast.devices.clone().iter().enumerate() {
                    let on = casting.as_ref() == Some(d);
                    let icon = if on {
                        Icon::Cast
                    } else if d.is_chromecast() {
                        Icon::Radio
                    } else {
                        Icon::Speaker
                    };
                    let label = if d.model.is_empty() {
                        d.name.clone()
                    } else {
                        format!("{} · {}", d.name, d.model)
                    };
                    if widgets::menu_item(ui, Some(icon), &label).clicked() && !on {
                        self.actions.push(Action::StartCast(k));
                    }
                }
                if self.cast.scanning.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new(tr("Recherche des appareils…"))
                                .small()
                                .color(theme::TEXT_3),
                        );
                    });
                } else if self.cast.devices.is_empty() {
                    ui.label(
                        RichText::new(tr("Aucun appareil trouvé sur le réseau."))
                            .small()
                            .color(theme::TEXT_3),
                    );
                }
            },
        );
    }
}
