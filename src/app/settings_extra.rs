//! Minuterie de sommeil, sortie audio, notifications du bureau et scrobbling : la logique, et
//! leurs réglages dans la page Paramètres.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use super::views::{card, card_row, separator, subtitle};
use super::{Action, SleepKind, SterMusic};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::scrobble;
use crate::trf;
use crate::{theme, widgets};

/// Choix proposés par la minuterie (minutes).
pub const SLEEP_CHOICES: [u32; 6] = [15, 30, 45, 60, 90, 120];

/// Où en est la connexion aux services de scrobbling.
#[derive(Default)]
pub struct OnlineState {
    /// Formulaire d'ajout d'un serveur de musique.
    pub server_form: super::servers_view::ServerForm,
    lb_check: Option<Receiver<Result<String, String>>>,
    lastfm: LastfmStep,
    error: Option<String>,
}

#[derive(Default)]
enum LastfmStep {
    #[default]
    Idle,
    /// Demande d'un jeton à Last.fm.
    Starting(Receiver<Result<(String, String), String>>),
    /// Le navigateur est ouvert sur la page d'autorisation : on attend que l'utilisateur valide.
    Authorizing {
        token: String,
        url: String,
    },
    Finishing(Receiver<Result<(String, String), String>>),
}

/// « 1 h 05 », « 12:34 » : temps restant de la minuterie.
pub fn fmt_remaining(secs: f32) -> String {
    let s = secs.max(0.0).ceil() as u64;
    if s >= 3600 {
        format!("{} h {:02}", s / 3600, (s / 60) % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

impl SterMusic {
    // ---------- Minuterie de sommeil ----------

    pub(super) fn set_sleep(&mut self, kind: Option<SleepKind>) {
        self.sleep = kind.map(|kind| super::Sleep {
            kind,
            end: Instant::now()
                + match kind {
                    SleepKind::Minutes(m) => Duration::from_secs(m as u64 * 60),
                    SleepKind::EndOfTrack => Duration::ZERO,
                },
        });
        self.sleep_gain = 1.0;
        self.apply_volume();
        match kind {
            Some(SleepKind::Minutes(m)) => self.toast(trf!("Arrêt de la lecture dans {0} min", m)),
            Some(SleepKind::EndOfTrack) => self.toast(tr("Arrêt de la lecture à la fin du morceau")),
            None => self.toast(tr("Minuterie de sommeil désactivée")),
        }
    }

    /// Secondes avant l'arrêt, si la minuterie tourne.
    pub(super) fn sleep_remaining(&self) -> Option<f32> {
        let sleep = self.sleep.as_ref()?;
        Some(match sleep.kind {
            SleepKind::Minutes(_) => sleep.end.saturating_duration_since(Instant::now()).as_secs_f32(),
            SleepKind::EndOfTrack => {
                let cur = self.current()?;
                let total = self.lib.tracks[cur].duration().as_secs_f32();
                let pos = self
                    .player
                    .as_ref()
                    .map(|p| p.position().as_secs_f32())
                    .unwrap_or(0.0);
                (total - pos).max(0.0)
            }
        })
    }

    /// Fondu de la fin de minuterie, puis arrêt.
    pub(super) fn sleep_tick(&mut self, ctx: &egui::Context) {
        if self.sleep.is_none() {
            return;
        }
        let end_of_track = self
            .sleep
            .as_ref()
            .is_some_and(|s| s.kind == SleepKind::EndOfTrack);
        let Some(remaining) = self.sleep_remaining() else {
            // « Fin du morceau » sans morceau (webradio, file vide) : rien à attendre.
            if end_of_track && !self.is_playing() {
                return;
            }
            self.sleep_done();
            return;
        };
        let fade = if end_of_track {
            self.settings.sleep_fade_secs.min(10.0)
        } else {
            self.settings.sleep_fade_secs
        }
        .max(1.0);
        let playing = self.is_playing();
        if !end_of_track && remaining <= 0.0 {
            self.sleep_done();
            return;
        }
        // Le volume ne descend que pendant la lecture (une pause en fin de minuterie le garde).
        let gain = if playing {
            (remaining / fade).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if (gain - self.sleep_gain).abs() > 0.002 {
            self.sleep_gain = gain;
            self.apply_volume();
        }
        ctx.request_repaint_after(if remaining < fade {
            Duration::from_millis(80)
        } else {
            Duration::from_secs(1)
        });
    }

    /// La minuterie arrive au bout : la lecture s'arrête, le volume revient pour la prochaine fois.
    pub(super) fn sleep_done(&mut self) {
        self.sleep = None;
        if self.radio.live.as_ref().is_some_and(|l| !l.paused) {
            self.pause_station();
        } else if let Some(p) = &mut self.player {
            if p.finished() {
                p.stop();
            } else if p.is_playing() {
                p.pause();
            }
        }
        self.seeked = true;
        self.sleep_gain = 1.0;
        self.apply_volume();
        self.toast(tr("Minuterie : lecture arrêtée. Bonne nuit !"));
    }

    /// Bulle de la minuterie, ouverte depuis la barre de lecture.
    pub(super) fn sleep_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(230.0);
        ui.label(RichText::new(tr("Minuterie de sommeil")).font(theme::semibold(15.0)));
        if let Some(r) = self.sleep_remaining() {
            ui.label(
                RichText::new(trf!("Arrêt dans {0}", fmt_remaining(r)))
                    .small()
                    .color(theme::accent_light()),
            );
        } else {
            ui.label(
                RichText::new(tr("La musique baisse doucement, puis s'arrête."))
                    .small()
                    .color(theme::TEXT_3),
            );
        }
        ui.add_space(6.0);
        let current = self.sleep.as_ref().map(|s| s.kind);
        for m in SLEEP_CHOICES {
            let label = if m >= 60 && m % 60 == 0 {
                format!("{} h", m / 60)
            } else if m > 60 {
                format!("{} h {:02}", m / 60, m % 60)
            } else {
                format!("{m} min")
            };
            let on = current == Some(SleepKind::Minutes(m));
            if widgets::menu_item(ui, Some(if on { Icon::Timer } else { Icon::Clock }), &label).clicked() {
                self.actions.push(Action::SetSleep(Some(SleepKind::Minutes(m))));
            }
        }
        if self.radio.live.is_none()
            && self.podcasts.playing.is_none()
            && widgets::menu_item(ui, Some(Icon::Music), tr("À la fin du morceau")).clicked()
        {
            self.actions.push(Action::SetSleep(Some(SleepKind::EndOfTrack)));
        }
        if current.is_some() {
            ui.separator();
            if widgets::menu_item(ui, Some(Icon::Close), tr("Désactiver la minuterie")).clicked() {
                self.actions.push(Action::SetSleep(None));
            }
        }
    }

    // ---------- Zone de notification ----------

    /// Place l'icône dans la zone de notification ; sans zone compatible, l'option se décoche.
    pub(super) fn start_tray(&mut self) {
        if self.tray.is_some() {
            return;
        }
        self.tray = crate::tray::Tray::start(&self.egui_ctx, Default::default());
        if self.tray.is_none() {
            self.settings.tray = false;
            self.toast(tr("Pas de zone de notification compatible sur ce bureau"));
        }
    }

    pub(super) fn stop_tray(&mut self) {
        self.tray = None;
        if self.hidden {
            self.show_window();
        }
    }

    /// Vrai si fermer la fenêtre la cache dans la zone de notification (il faut KWin pour
    /// cacher une fenêtre sous Wayland).
    pub(super) fn closes_to_tray(&self) -> bool {
        self.tray.is_some() && self.kwin.active() && !self.quitting
    }

    pub(super) fn hide_window(&mut self) {
        if self.kwin.active() {
            self.hidden = true;
        } else {
            self.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    }

    pub(super) fn show_window(&mut self) {
        self.hidden = false;
        self.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        self.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    /// Commandes venues de l'icône, et fermeture de la fenêtre interceptée.
    pub(super) fn poll_tray(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.closes_to_tray() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window();
        }
        let Some(tray) = &self.tray else { return };
        for c in tray.poll() {
            use crate::tray::Command;
            match c {
                Command::Toggle if self.hidden => self.show_window(),
                Command::Toggle => self.hide_window(),
                Command::Show => self.show_window(),
                Command::PlayPause => self.actions.push(Action::TogglePlay),
                Command::Next => self.actions.push(Action::Next),
                Command::Previous => self.actions.push(Action::Previous),
                Command::Quit => {
                    self.quitting = true;
                    self.hidden = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    // ---------- Sortie audio ----------

    pub(super) fn refresh_outputs(&mut self) {
        self.outputs = crate::outputs::list();
    }

    pub(super) fn set_output(&mut self, name: String) {
        if name == self.settings.audio_output {
            return;
        }
        self.settings.audio_output = name;
        crate::outputs::route_to(&self.settings.audio_output);
        let live = self
            .radio
            .live
            .as_ref()
            .filter(|l| !l.paused)
            .map(|l| l.station.clone());
        if let Some(p) = &mut self.player {
            if let Err(e) = p.reopen() {
                self.toast(e);
            }
        }
        if let Some(station) = live {
            let ctx = self.egui_ctx.clone();
            self.play_station(&ctx, station);
        }
        self.seeked = true;
        let label = self
            .outputs
            .iter()
            .find(|o| o.name == self.settings.audio_output)
            .map(|o| o.description.clone())
            .unwrap_or_else(|| tr("Sortie du système").into());
        self.toast(trf!("Sortie audio : {0}", label));
    }

    /// Menu des sorties audio (clic droit sur le volume, paramètres).
    pub(super) fn output_menu(&mut self, ui: &mut egui::Ui) {
        if self.outputs.is_empty() {
            self.refresh_outputs();
        }
        let current = self.settings.audio_output.clone();
        let system = current.is_empty();
        if widgets::menu_item(
            ui,
            Some(if system { Icon::Speaker } else { Icon::Volume2 }),
            tr("Sortie du système"),
        )
        .clicked()
        {
            self.actions.push(Action::SetOutput(String::new()));
        }
        for o in self.outputs.clone() {
            let on = o.name == current;
            if widgets::menu_item(
                ui,
                Some(if on { Icon::Speaker } else { Icon::Volume2 }),
                &o.description,
            )
            .clicked()
            {
                self.actions.push(Action::SetOutput(o.name));
            }
        }
    }

    // ---------- Notifications ----------

    /// Notifie un changement (morceau, titre d'une webradio) si la fenêtre n'est pas au premier plan.
    pub(super) fn notify_change(&mut self, title: String, body: String, image: Option<PathBuf>) {
        if !self.settings.notify_tracks || self.focused || self.mini {
            return;
        }
        self.notifier.send(crate::notify::Note { title, body, image });
    }

    /// Le morceau vient de changer : on attend sa pochette (au plus une seconde et demie).
    pub(super) fn send_pending_note(&mut self, ctx: &egui::Context) {
        let Some((path, deadline)) = self.pending_note.clone() else {
            return;
        };
        let Some(idx) = self.current().filter(|&i| self.lib.tracks[i].path == path) else {
            self.pending_note = None;
            return;
        };
        if !self.settings.notify_tracks || self.focused {
            self.pending_note = None;
            return;
        }
        let now = ctx.input(|i| i.time);
        let cover = self.covers.large(&path).map(|l| l.file.clone());
        if cover.is_none() && now < deadline {
            ctx.request_repaint_after(Duration::from_millis(150));
            return;
        }
        self.pending_note = None;
        let t = self.lib.tracks[idx].clone();
        let body = if t.album.is_empty() {
            t.artist_or_unknown().to_owned()
        } else {
            format!("{} — {}", t.artist_or_unknown(), t.album)
        };
        self.notify_change(t.title.clone(), body, cover.flatten());
    }

    // ---------- Scrobbling ----------

    pub(super) fn poll_online(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        if let Some(rx) = &self.online.lb_check {
            if let Ok(result) = rx.try_recv() {
                self.online.lb_check = None;
                match result {
                    Ok(user) => {
                        self.settings.scrobble.lb_user = user;
                        self.settings.scrobble.listenbrainz = true;
                        self.online.error = None;
                        changed = true;
                    }
                    Err(e) => self.online.error = Some(format!("ListenBrainz : {e}")),
                }
            }
        }
        let step = std::mem::take(&mut self.online.lastfm);
        self.online.lastfm = match step {
            LastfmStep::Starting(rx) => match rx.try_recv() {
                Ok(Ok((token, url))) => {
                    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                    LastfmStep::Authorizing { token, url }
                }
                Ok(Err(e)) => {
                    self.online.error = Some(format!("Last.fm : {e}"));
                    LastfmStep::Idle
                }
                Err(_) => LastfmStep::Starting(rx),
            },
            LastfmStep::Finishing(rx) => match rx.try_recv() {
                Ok(Ok((session, user))) => {
                    self.settings.scrobble.lastfm_session = session;
                    self.settings.scrobble.lastfm_user = user;
                    self.settings.scrobble.lastfm = true;
                    self.online.error = None;
                    changed = true;
                    LastfmStep::Idle
                }
                Ok(Err(e)) => {
                    self.online.error = Some(format!("Last.fm : {e}"));
                    LastfmStep::Idle
                }
                Err(_) => LastfmStep::Finishing(rx),
            },
            other => other,
        };
        if changed {
            self.actions.push(Action::ScrobbleConfig);
        }
        if self.online.lb_check.is_some()
            || matches!(
                self.online.lastfm,
                LastfmStep::Starting(_) | LastfmStep::Finishing(_)
            )
        {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    // ---------- Paramètres ----------

    /// Ligne « Sortie audio » de la carte Son.
    pub(super) fn output_setting(&mut self, ui: &mut egui::Ui) {
        if self.outputs.is_empty() {
            self.refresh_outputs();
        }
        let current = self
            .outputs
            .iter()
            .find(|o| o.name == self.settings.audio_output)
            .map(|o| o.description.clone())
            .unwrap_or_else(|| {
                if self.settings.audio_output.is_empty() {
                    tr("Sortie du système").into()
                } else {
                    trf!("{0} (débranchée ?)", self.settings.audio_output)
                }
            });
        let desc = if self.outputs.is_empty() {
            tr("Liste des sorties indisponible (pactl introuvable) : le son va vers la sortie du système.")
        } else {
            tr("Haut-parleurs, casque, HDMI… « Sortie du système » suit le choix de Plasma.")
        };
        card_row(
            ui,
            Icon::Speaker,
            tr("Sortie audio"),
            desc,
            |s, ui| {
                if widgets::icon_button(ui, Icon::Refresh, tr("Actualiser la liste"), 30.0, 15.0).clicked() {
                    s.refresh_outputs();
                }
                egui::ComboBox::from_id_salt("audio_output")
                    .selected_text(current)
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        let mut chosen = s.settings.audio_output.clone();
                        ui.selectable_value(&mut chosen, String::new(), tr("Sortie du système"));
                        for o in &s.outputs {
                            ui.selectable_value(&mut chosen, o.name.clone(), &o.description);
                        }
                        if chosen != s.settings.audio_output {
                            s.actions.push(Action::SetOutput(chosen));
                        }
                    });
            },
            self,
        );
    }

    /// Lignes « Notifications » et « Minuterie » de la carte Lecture.
    pub(super) fn playback_extra_settings(&mut self, ui: &mut egui::Ui) {
        separator(ui);
        let mut notify = self.settings.notify_tracks;
        card_row(ui, Icon::Bell, tr("Notification au changement de morceau"), tr("Titre, artiste et pochette dans une bulle de Plasma, quand la fenêtre de SterMusic n'est pas au premier plan."), |_, ui| {
            widgets::switch(ui, &mut notify);
        }, self);
        self.settings.notify_tracks = notify;
        separator(ui);
        card_row(ui, Icon::Timer, tr("Minuterie de sommeil"), tr("Depuis le bouton lune de la barre de lecture. Le volume baisse pendant les dernières secondes."), |_, _| {}, self);
        ui.horizontal(|ui| {
            ui.add_space(52.0);
            ui.add_sized(
                [80.0, 20.0],
                egui::Label::new(RichText::new(tr("Fondu")).color(theme::TEXT_2)),
            );
            let value = (self.settings.sleep_fade_secs - 5.0) / 115.0;
            let seek = widgets::seek_bar(ui, "sleep_fade", value, 220.0, true);
            if let Some(v) = seek.preview.or(seek.commit) {
                self.settings.sleep_fade_secs = (5.0 + v * 115.0).round();
            }
            ui.label(
                RichText::new(format!("{:.0} s", self.settings.sleep_fade_secs))
                    .small()
                    .color(theme::TEXT_3),
            );
        });
    }

    /// Section « Scrobbling ».
    pub(super) fn scrobble_settings(&mut self, ui: &mut egui::Ui) {
        subtitle(ui, tr("Scrobbling"));
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(52.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(tr("Tes écoutes sont envoyées à ton compte : un morceau compte une fois écouté à moitié (ou 4 minutes). Sans réseau, elles attendent et partent plus tard."))
                            .small()
                            .color(theme::TEXT_2),
                    )
                    .wrap(),
                );
            });
            separator(ui);

            // ListenBrainz : un jeton suffit.
            let lb_on = self.settings.scrobble.listenbrainz && !self.settings.scrobble.lb_user.is_empty();
            let desc = if lb_on {
                trf!("Connecté : {0}", self.settings.scrobble.lb_user)
            } else {
                tr("Libre et ouvert. Colle le jeton de ton compte (page Paramètres de listenbrainz.org).")
                    .into()
            };
            let mut enabled = self.settings.scrobble.listenbrainz;
            card_row(
                ui,
                Icon::RadioTower,
                "ListenBrainz",
                &desc,
                |s, ui| {
                    if !s.settings.scrobble.lb_user.is_empty() {
                        widgets::switch(ui, &mut enabled);
                    }
                },
                self,
            );
            if enabled != self.settings.scrobble.listenbrainz {
                self.settings.scrobble.listenbrainz = enabled;
                self.actions.push(Action::ScrobbleConfig);
            }
            ui.horizontal(|ui| {
                ui.add_space(52.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.scrobble.lb_token)
                        .password(true)
                        .desired_width(280.0)
                        .hint_text(tr("Jeton ListenBrainz")),
                );
                let checking = self.online.lb_check.is_some();
                if checking {
                    ui.spinner();
                } else if widgets::secondary_button(ui, Some(Icon::Link), tr("Se connecter")).clicked() {
                    let token = self.settings.scrobble.lb_token.trim().to_owned();
                    if !token.is_empty() {
                        let (tx, rx) = channel();
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(scrobble::lb_validate(&token));
                            ctx.request_repaint();
                        });
                        self.online.lb_check = Some(rx);
                    }
                }
                if widgets::link(ui, tr("Obtenir un jeton")).clicked() {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(scrobble::LB_TOKEN_PAGE)
                        .spawn();
                }
            });
            separator(ui);

            // Last.fm : clé d'API personnelle, puis autorisation dans le navigateur.
            let fm_on = !self.settings.scrobble.lastfm_session.is_empty();
            let desc = if fm_on {
                trf!("Connecté : {0}", self.settings.scrobble.lastfm_user)
            } else {
                tr("Crée une clé d'API gratuite sur last.fm (nom au choix), puis colle sa clé et son secret.").into()
            };
            let mut enabled = self.settings.scrobble.lastfm;
            card_row(
                ui,
                Icon::RadioTower,
                "Last.fm",
                &desc,
                |s, ui| {
                    if fm_on {
                        if widgets::secondary_button(ui, None, tr("Se déconnecter")).clicked() {
                            s.settings.scrobble.lastfm_session.clear();
                            s.settings.scrobble.lastfm_user.clear();
                            s.settings.scrobble.lastfm = false;
                            s.actions.push(Action::ScrobbleConfig);
                        }
                        widgets::switch(ui, &mut enabled);
                    }
                },
                self,
            );
            if fm_on && enabled != self.settings.scrobble.lastfm {
                self.settings.scrobble.lastfm = enabled;
                self.actions.push(Action::ScrobbleConfig);
            }
            if !fm_on {
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.scrobble.lastfm_key)
                            .desired_width(220.0)
                            .hint_text(tr("Clé d'API")),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.scrobble.lastfm_secret)
                            .password(true)
                            .desired_width(220.0)
                            .hint_text(tr("Secret partagé")),
                    );
                    if widgets::link(ui, tr("Créer une clé")).clicked() {
                        let _ = std::process::Command::new("xdg-open")
                            .arg(scrobble::LASTFM_NEW_KEY)
                            .spawn();
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    let ready = !self.settings.scrobble.lastfm_key.trim().is_empty()
                        && !self.settings.scrobble.lastfm_secret.trim().is_empty();
                    match &self.online.lastfm {
                        LastfmStep::Idle => {
                            let clicked = ui
                                .add_enabled_ui(ready, |ui| {
                                    widgets::secondary_button(ui, Some(Icon::Link), tr("Autoriser SterMusic sur Last.fm"))
                                })
                                .inner
                                .clicked();
                            if clicked {
                                let (key, secret) = (
                                    self.settings.scrobble.lastfm_key.clone(),
                                    self.settings.scrobble.lastfm_secret.clone(),
                                );
                                let (tx, rx) = channel();
                                let ctx = ui.ctx().clone();
                                std::thread::spawn(move || {
                                    let _ = tx.send(scrobble::lastfm_begin(&key, &secret));
                                    ctx.request_repaint();
                                });
                                self.online.lastfm = LastfmStep::Starting(rx);
                            }
                        }
                        LastfmStep::Starting(_) | LastfmStep::Finishing(_) => {
                            ui.spinner();
                        }
                        LastfmStep::Authorizing { token, url } => {
                            let (token, url) = (token.clone(), url.clone());
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(tr("Autorise SterMusic dans la page Last.fm ouverte dans ton navigateur, puis reviens ici."))
                                        .small()
                                        .color(theme::accent_light()),
                                );
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    if widgets::primary_button(ui, None, tr("C'est fait")).clicked() {
                                        let (key, secret) = (
                                            self.settings.scrobble.lastfm_key.clone(),
                                            self.settings.scrobble.lastfm_secret.clone(),
                                        );
                                        let (tx, rx) = channel();
                                        let ctx = ui.ctx().clone();
                                        std::thread::spawn(move || {
                                            let _ = tx.send(scrobble::lastfm_finish(&key, &secret, &token));
                                            ctx.request_repaint();
                                        });
                                        self.online.lastfm = LastfmStep::Finishing(rx);
                                    }
                                    if widgets::link(ui, tr("Rouvrir la page")).clicked() {
                                        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                                    }
                                    if widgets::link(ui, tr("Annuler")).clicked() {
                                        self.online.lastfm = LastfmStep::Idle;
                                    }
                                });
                            });
                        }
                    }
                });
            }

            let status = self.scrobbler.status();
            let error = self.online.error.clone().or(status.error.clone());
            if error.is_some() || status.pending > 0 || status.sent > 0 {
                separator(ui);
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    let mut parts = Vec::new();
                    if status.sent > 0 {
                        parts.push(crate::i18n::plural(
                            status.sent,
                            "écoute envoyée",
                            "écoutes envoyées",
                        ));
                    }
                    if status.pending > 0 {
                        parts.push(crate::i18n::plural(
                            status.pending,
                            "écoute en attente",
                            "écoutes en attente",
                        ));
                    }
                    if !parts.is_empty() {
                        ui.label(RichText::new(parts.join(" · ")).small().color(theme::TEXT_3));
                    }
                    if let Some(e) = error {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(e)
                                .small()
                                .color(egui::Color32::from_rgb(0xF0, 0x9A, 0x8A)),
                        );
                    }
                });
            }
            ui.add_space(2.0);
        });
    }
}
