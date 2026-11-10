//! Serveurs de musique dans les paramètres : ajout, actualisation, retrait.

use std::sync::mpsc::{channel, Receiver};

use eframe::egui::{self, RichText};

use super::views::{card, card_row, separator, subtitle};
use super::{Action, SterMusic};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::servers::{self, Kind, Server};
use crate::trf;
use crate::{theme, widgets};

/// Formulaire « Ajouter un serveur ».
#[derive(Default)]
pub struct ServerForm {
    pub open: bool,
    pub server: Server,
    connecting: Option<Receiver<Result<Server, String>>>,
    pub error: Option<String>,
}

impl SterMusic {
    pub(super) fn sync_server(&mut self, ctx: &egui::Context, id: &str) {
        let Some(server) = self.settings.servers.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        if self.server_rx.iter().any(|(i, _)| i == id) {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(servers::fetch(&server));
            ctx.request_repaint();
        });
        self.server_rx.push((id.to_owned(), rx));
    }

    pub(super) fn poll_servers(&mut self, ctx: &egui::Context) {
        let mut done = Vec::new();
        self.server_rx.retain(|(id, rx)| match rx.try_recv() {
            Ok(r) => {
                done.push((id.clone(), r));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for (id, result) in done {
            match result {
                Ok(tracks) => {
                    servers::save_cache(&id, &tracks);
                    let n = tracks.len();
                    if let Some(s) = self.settings.servers.iter_mut().find(|s| s.id == id) {
                        s.synced = super::now_secs();
                        s.count = n;
                    }
                    self.server_tracks.insert(id.clone(), tracks);
                    let all = self.lib.tracks.clone();
                    self.set_library(all);
                    let name = self.server_name(&id);
                    self.toast(trf!(
                        "{0} : {1}",
                        name,
                        crate::library::plural(n, "morceau", "morceaux")
                    ));
                }
                Err(e) => {
                    let name = self.server_name(&id);
                    self.toast(format!("{name} — {e}"));
                }
            }
        }
        if let Some(rx) = &self.online.server_form.connecting {
            if let Ok(result) = rx.try_recv() {
                self.online.server_form.connecting = None;
                match result {
                    Ok(server) => {
                        let id = server.id.clone();
                        self.settings.servers.push(server);
                        self.online.server_form = ServerForm::default();
                        self.sync_server(ctx, &id);
                    }
                    Err(e) => self.online.server_form.error = Some(e),
                }
            }
        }
        if !self.server_rx.is_empty() || self.online.server_form.connecting.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn server_name(&self, id: &str) -> String {
        self.settings
            .servers
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }

    /// Section « Serveurs de musique » des paramètres.
    pub(super) fn servers_settings(&mut self, ui: &mut egui::Ui) {
        subtitle(ui, tr("Serveurs de musique"));
        card(ui, |ui| {
            card_row(
                ui,
                Icon::Globe,
                tr("Navidrome, Subsonic, Jellyfin"),
                tr("Les morceaux du serveur rejoignent ta bibliothèque et se lisent en direct, sans être copiés."),
                |s, ui| {
                    if !s.online.server_form.open
                        && widgets::secondary_button(ui, Some(Icon::Plus), tr("Ajouter un serveur")).clicked()
                    {
                        s.online.server_form = ServerForm {
                            open: true,
                            ..Default::default()
                        };
                    }
                },
                self,
            );
            for server in self.settings.servers.clone() {
                separator(ui);
                let syncing = self.server_rx.iter().any(|(i, _)| *i == server.id);
                let desc = if syncing {
                    tr("Actualisation…").to_owned()
                } else {
                    format!(
                        "{} · {} · {}",
                        server.kind.label(),
                        server.url,
                        crate::library::plural(server.count, "morceau", "morceaux")
                    )
                };
                let mut enabled = server.enabled;
                card_row(
                    ui,
                    Icon::Music,
                    &server.name,
                    &desc,
                    |s, ui| {
                        if widgets::icon_button(ui, Icon::Trash, tr("Retirer ce serveur"), 30.0, 15.0)
                            .clicked()
                        {
                            s.actions.push(Action::RemoveServer(server.id.clone()));
                        }
                        if syncing {
                            ui.spinner();
                        } else if widgets::icon_button(ui, Icon::Refresh, tr("Actualiser"), 30.0, 15.0)
                            .clicked()
                        {
                            s.actions.push(Action::SyncServer(server.id.clone()));
                        }
                        widgets::switch(ui, &mut enabled);
                    },
                    self,
                );
                if enabled != server.enabled {
                    if let Some(s) = self.settings.servers.iter_mut().find(|s| s.id == server.id) {
                        s.enabled = enabled;
                    }
                    if enabled && !self.server_tracks.contains_key(&server.id) {
                        let tracks = servers::load_cache(&server.id);
                        self.server_tracks.insert(server.id.clone(), tracks);
                    }
                    let all = self.lib.tracks.clone();
                    self.set_library(all);
                }
            }
            if self.online.server_form.open {
                separator(ui);
                self.server_form(ui);
            }
        });
    }

    fn server_form(&mut self, ui: &mut egui::Ui) {
        let form = &mut self.online.server_form;
        ui.horizontal(|ui| {
            ui.add_space(52.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for kind in [Kind::Subsonic, Kind::Jellyfin] {
                        if super::views::segment(ui, kind.label(), form.server.kind == kind).clicked() {
                            form.server.kind = kind;
                        }
                    }
                });
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::singleline(&mut form.server.url)
                        .desired_width(380.0)
                        .hint_text(tr("Adresse (http://nas.local:4533)")),
                );
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut form.server.user)
                            .desired_width(186.0)
                            .hint_text(tr("Utilisateur")),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut form.server.password)
                            .password(true)
                            .desired_width(186.0)
                            .hint_text(tr("Mot de passe")),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let ready = !form.server.url.trim().is_empty() && !form.server.user.trim().is_empty();
                    if form.connecting.is_some() {
                        ui.spinner();
                        ui.label(RichText::new(tr("Connexion…")).color(theme::TEXT_3));
                    } else {
                        let go = ui
                            .add_enabled_ui(ready, |ui| widgets::primary_button(ui, Some(Icon::Link), tr("Se connecter")))
                            .inner
                            .clicked();
                        if go {
                            let server = form.server.clone();
                            let (tx, rx) = channel();
                            let ctx = ui.ctx().clone();
                            std::thread::spawn(move || {
                                let _ = tx.send(servers::connect(server));
                                ctx.request_repaint();
                            });
                            form.connecting = Some(rx);
                            form.error = None;
                        }
                        if widgets::secondary_button(ui, None, tr("Annuler")).clicked() {
                            *form = ServerForm::default();
                        }
                    }
                });
                if let Some(e) = &form.error {
                    ui.label(RichText::new(e).small().color(egui::Color32::from_rgb(0xF0, 0x9A, 0x8A)));
                }
                ui.label(
                    RichText::new(tr(
                        "Le mot de passe d'un serveur Subsonic est gardé dans les réglages de SterMusic ; Jellyfin, lui, donne un jeton.",
                    ))
                    .small()
                    .color(theme::TEXT_3),
                );
            });
        });
    }
}
