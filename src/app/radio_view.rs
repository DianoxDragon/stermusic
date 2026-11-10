//! Page « Webradios » : mes stations, l'annuaire, et l'écoute en direct.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui::{self, pos2, vec2, Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense};

use super::views::heading;
use super::{Action, SterMusic};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::radio::{self, Connected, LiveInfo, Query, Station};
use crate::trf;
use crate::{theme, widgets};

/// Genres proposés en raccourci (nom affiché, étiquette de l'annuaire).
const GENRES: &[(&str, &str)] = &[
    (crate::i18n::n_("Pop"), "pop"),
    (crate::i18n::n_("Rock"), "rock"),
    (crate::i18n::n_("Jazz"), "jazz"),
    (crate::i18n::n_("Classique"), "classical"),
    (crate::i18n::n_("Électro"), "electronic"),
    (crate::i18n::n_("Hip-hop"), "hiphop"),
    (crate::i18n::n_("Chill"), "chillout"),
    (crate::i18n::n_("Lounge"), "lounge"),
    (crate::i18n::n_("Années 80"), "80s"),
    (crate::i18n::n_("Metal"), "metal"),
    (crate::i18n::n_("Ambient"), "ambient"),
    (crate::i18n::n_("Info"), "news"),
];

/// Station en cours d'écoute.
pub struct Live {
    pub station: Station,
    pub info: LiveInfo,
    /// Arrêtée par l'utilisateur (un direct ne se met pas en pause : il se relance).
    pub paused: bool,
    /// Reconnexions tentées depuis la dernière coupure.
    retries: u8,
    /// Dernier titre diffusé vu par l'interface (notification quand il change).
    pub last_title: String,
}

enum Logo {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

pub struct RadioState {
    pub live: Option<Live>,
    pub connecting: Option<(Station, Receiver<Result<Connected, String>>)>,
    query: String,
    results: Vec<Station>,
    shown_query: Option<Query>,
    searching: Option<Receiver<Result<Vec<Station>, String>>>,
    error: Option<String>,
    logos: HashMap<String, Logo>,
    logo_tx: Sender<(String, Option<egui::ColorImage>)>,
    logo_rx: Receiver<(String, Option<egui::ColorImage>)>,
    /// Ajout d'une station par son adresse : nom et adresse saisis.
    pub add: Option<(String, String)>,
}

impl Default for RadioState {
    fn default() -> Self {
        let (logo_tx, logo_rx) = channel();
        Self {
            live: None,
            connecting: None,
            query: String::new(),
            results: Vec::new(),
            shown_query: None,
            searching: None,
            error: None,
            logos: HashMap::new(),
            logo_tx,
            logo_rx,
            add: None,
        }
    }
}

impl RadioState {
    /// Station qui joue (ou se connecte) : son nom.
    pub fn connecting_to(&self) -> Option<&Station> {
        self.connecting.as_ref().map(|(s, _)| s)
    }

    /// Logo d'une station (téléchargé en arrière-plan à la première demande).
    pub fn logo(&mut self, ctx: &egui::Context, station: &Station) -> Option<egui::TextureHandle> {
        if station.favicon.is_empty() {
            return None;
        }
        match self.logos.get(&station.favicon) {
            Some(Logo::Ready(t)) => return Some(t.clone()),
            Some(_) => return None,
            None => {}
        }
        self.logos.insert(station.favicon.clone(), Logo::Loading);
        let (url, tx, ctx) = (station.favicon.clone(), self.logo_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let image = radio::fetch_image(&url).and_then(|bytes| decode_logo(&bytes));
            let _ = tx.send((url, image));
            ctx.request_repaint();
        });
        None
    }

    fn search(&mut self, ctx: &egui::Context, query: Query) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let q = query.clone();
        std::thread::spawn(move || {
            let _ = tx.send(radio::search(&q));
            ctx.request_repaint();
        });
        self.searching = Some(rx);
        self.shown_query = Some(query);
        self.error = None;
    }
}

/// Logo en petite texture carrée (le format est deviné ; un SVG ou une icône .ico échouent).
fn decode_logo(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = img
        .resize(128, 128, image::imageops::FilterType::Triangle)
        .to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [img.width() as usize, img.height() as usize],
        img.as_raw(),
    ))
}

impl SterMusic {
    /// Lance l'écoute d'une station (la connexion se fait en arrière-plan ; la musique en
    /// cours continue jusqu'à ce que la station réponde).
    pub(super) fn play_station(&mut self, ctx: &egui::Context, station: Station) {
        let (tx, rx) = channel();
        let s = station.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(radio::connect(&s));
            ctx.request_repaint();
        });
        self.radio.connecting = Some((station, rx));
    }

    /// Arrête la station en cours (elle reste affichée, prête à être relancée).
    pub(super) fn pause_station(&mut self) {
        if let Some(p) = &mut self.player {
            p.stop();
        }
        if let Some(live) = &mut self.radio.live {
            live.info.stop.store(true, Ordering::Relaxed);
            live.paused = true;
        }
    }

    /// Quitte la radio (un morceau de la bibliothèque va jouer).
    pub(super) fn leave_radio(&mut self) {
        if let Some(live) = self.radio.live.take() {
            live.info.stop.store(true, Ordering::Relaxed);
        }
        self.radio.connecting = None;
    }

    /// Station suivante (ou précédente) parmi « Mes stations ».
    pub(super) fn step_station(&mut self, ctx: &egui::Context, delta: isize) {
        let list = &self.settings.radios;
        if list.is_empty() {
            return;
        }
        let current = self
            .radio
            .live
            .as_ref()
            .and_then(|l| list.iter().position(|s| s.url == l.station.url));
        let next = match current {
            Some(i) => (i as isize + delta).rem_euclid(list.len() as isize) as usize,
            None => 0,
        };
        let station = list[next].clone();
        self.play_station(ctx, station);
    }

    pub(super) fn poll_radio(&mut self, ctx: &egui::Context) {
        while let Ok((url, image)) = self.radio.logo_rx.try_recv() {
            let logo = match image {
                Some(img) => Logo::Ready(ctx.load_texture(format!("logo:{url}"), img, Default::default())),
                None => Logo::Failed,
            };
            self.radio.logos.insert(url, logo);
        }
        if let Some(rx) = &self.radio.searching {
            if let Ok(result) = rx.try_recv() {
                self.radio.searching = None;
                match result {
                    Ok(list) => self.radio.results = list,
                    Err(e) => {
                        self.radio.results.clear();
                        self.radio.error = Some(trf!("Annuaire injoignable : {e}", e));
                    }
                }
            }
        }
        let connected = match &self.radio.connecting {
            Some((_, rx)) => rx.try_recv().ok(),
            None => None,
        };
        if let Some(result) = connected {
            let (station, _) = self.radio.connecting.take().unwrap();
            match result {
                Ok((decoder, info)) => self.start_live(station, decoder, info),
                Err(e) => {
                    self.toast(format!("{} — {e}", station.name));
                    if let Some(live) = &mut self.radio.live {
                        live.paused = true;
                    }
                }
            }
        }
        // Coupure du flux : on retente trois fois, puis on s'arrête.
        let ended = self.radio.live.as_ref().is_some_and(|l| !l.paused)
            && self.radio.connecting.is_none()
            && !self.player.as_ref().is_some_and(|p| p.is_live() && !p.finished());
        if ended {
            let live = self.radio.live.as_mut().unwrap();
            if live.retries < 3 {
                live.retries += 1;
                let station = live.station.clone();
                self.play_station(ctx, station);
            } else {
                live.paused = true;
                self.toast(tr("La station a coupé le flux"));
            }
        }
        // Titre diffusé : il change en cours d'écoute.
        if let Some(live) = &mut self.radio.live {
            if !live.paused {
                let title = live.info.title();
                if title != live.last_title {
                    live.last_title = title.clone();
                    if !title.is_empty() {
                        let station = live.station.clone();
                        self.notify_change(title, station.name.clone(), None);
                    }
                }
                ctx.request_repaint_after(std::time::Duration::from_secs(1));
            }
        }
        if self.radio.connecting.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    fn start_live(
        &mut self,
        mut station: Station,
        decoder: rodio::Decoder<radio::StreamReader>,
        info: LiveInfo,
    ) {
        if self.player.is_none() {
            return;
        }
        // Station ajoutée par son adresse, sans nom : celui que donne le serveur.
        if station.name == station.url && !info.server_name.is_empty() {
            let url = station.url.clone();
            station.name = info.server_name.clone();
            for s in self.settings.radios.iter_mut().filter(|s| s.url == url) {
                s.name = station.name.clone();
            }
        }
        self.finish_listen();
        let retries = match &self.radio.live {
            Some(l) if l.station.url == station.url && !l.paused => l.retries,
            _ => 0,
        };
        if let Some(old) = self.radio.live.take() {
            old.info.stop.store(true, Ordering::Relaxed);
        }
        let Some(player) = &mut self.player else { return };
        if let Err(e) = player.play_stream(decoder) {
            info.stop.store(true, Ordering::Relaxed);
            self.toast(e);
            return;
        }
        self.radio.live = Some(Live {
            station,
            info,
            paused: false,
            retries,
            last_title: String::new(),
        });
        self.seeked = true;
    }

    /// Ajoute (ou retire) une station de « Mes stations ».
    pub(super) fn toggle_saved_station(&mut self, station: &Station) {
        match self.settings.radios.iter().position(|s| s.url == station.url) {
            Some(i) => {
                self.settings.radios.remove(i);
                self.toast(trf!("« {0} » retirée de tes stations", station.name));
            }
            None => {
                self.settings.radios.push(station.clone());
                self.toast(trf!("« {0} » ajoutée à tes stations", station.name));
            }
        }
    }

    // ---------- Page ----------

    pub(super) fn radio_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.radio.shown_query.is_none() {
            let country = radio::desktop_country();
            self.radio.search(&ctx, Query::Top(country));
        }
        ui.horizontal(|ui| {
            heading(ui, tr("Webradios"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if widgets::secondary_button(ui, Some(Icon::Link), tr("Ajouter une adresse…")).clicked() {
                    self.radio.add = Some((String::new(), String::new()));
                }
            });
        });
        ui.add_space(14.0);
        egui::ScrollArea::vertical()
            .id_salt("radio_page")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                if !self.settings.radios.is_empty() {
                    super::lists::section(ui, tr("Mes stations"), None);
                    ui.add_space(4.0);
                    let saved = self.settings.radios.clone();
                    let (cols, w) = station_grid(ui.available_width());
                    for row in saved.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.0;
                            for s in row {
                                self.station_tile(ui, s, w);
                            }
                        });
                        ui.add_space(10.0);
                    }
                    ui.add_space(14.0);
                }

                super::lists::section(ui, tr("Trouver une station"), None);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 130.0).clamp(160.0, 420.0);
                    let field = widgets::search_field(
                        ui,
                        egui::Id::new("radio_search"),
                        &mut self.radio.query,
                        tr("Nom d'une station"),
                        w,
                    );
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let go = widgets::secondary_button(ui, Some(Icon::Search), tr("Chercher")).clicked();
                    let q = self.radio.query.trim().to_owned();
                    if (enter || go) && !q.is_empty() {
                        self.radio.search(&ctx, Query::Name(q));
                    }
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    let country = radio::desktop_country();
                    let top = Query::Top(country.clone());
                    if super::views::segment(
                        ui,
                        tr("Les plus écoutées"),
                        self.radio.shown_query == Some(top.clone()),
                    )
                    .clicked()
                    {
                        self.radio.search(&ctx, top);
                    }
                    for (label, tag) in GENRES {
                        let q = Query::Tag((*tag).into());
                        if super::views::segment(ui, tr(label), self.radio.shown_query.as_ref() == Some(&q))
                            .clicked()
                        {
                            self.radio.search(&ctx, q);
                        }
                    }
                });
                ui.add_space(12.0);
                if self.radio.searching.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(tr("Recherche dans l'annuaire…")).color(theme::TEXT_3));
                    });
                } else if let Some(e) = &self.radio.error {
                    ui.label(RichText::new(e).color(theme::TEXT_3));
                } else if self.radio.results.is_empty() {
                    ui.label(RichText::new(tr("Aucune station trouvée.")).color(theme::TEXT_3));
                }
                let results = self.radio.results.clone();
                for (i, s) in results.iter().enumerate() {
                    self.station_row(ui, s, i);
                }
                ui.add_space(10.0);
                ui.label(
                    RichText::new(tr("Annuaire : radio-browser.info"))
                        .small()
                        .color(theme::TEXT_3),
                );
                ui.add_space(24.0);
            });
    }

    /// Vignette de station (logo ou couleur, nom) : un clic lance l'écoute.
    fn station_tile(&mut self, ui: &mut egui::Ui, s: &Station, w: f32) {
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 64.0), Sense::click());
        let h = widgets::hover_t(ui, resp.id, resp.hovered());
        let on_air = self.station_state(s);
        let fill = if on_air.is_some() {
            theme::alpha(theme::accent(), 0x40)
        } else {
            theme::white((0x0A as f32 + 10.0 * h) as u8)
        };
        ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
        let logo_rect = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(48.0, 48.0));
        self.paint_logo(ui, logo_rect, s);
        let tx = logo_rect.right() + 12.0;
        let tw = rect.right() - tx - 10.0;
        let g = widgets::galley(ui, &s.name, theme::semibold(14.0), theme::TEXT, tw);
        ui.painter().galley(pos2(tx, rect.top() + 13.0), g, theme::TEXT);
        let sub = match on_air {
            Some(text) => text,
            None => s.details(),
        };
        let g = widgets::galley(ui, &sub, FontId::proportional(12.5), theme::TEXT_2, tw);
        ui.painter().galley(pos2(tx, rect.top() + 34.0), g, theme::TEXT_2);
        if resp.clicked() {
            self.actions.push(Action::PlayStation(s.clone()));
        }
        let station = s.clone();
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
            .context_menu(|ui| {
                if widgets::menu_item(ui, Some(Icon::PlayOutline), tr("Écouter")).clicked() {
                    self.actions.push(Action::PlayStation(station.clone()));
                }
                if !station.homepage.is_empty()
                    && widgets::menu_item(ui, Some(Icon::Globe), tr("Site de la station")).clicked()
                {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(&station.homepage)
                        .spawn();
                }
                ui.separator();
                if widgets::menu_item(ui, Some(Icon::Trash), tr("Retirer de mes stations")).clicked() {
                    self.actions.push(Action::ToggleStation(station.clone()));
                }
            });
    }

    /// Ligne d'un résultat de l'annuaire.
    fn station_row(&mut self, ui: &mut egui::Ui, s: &Station, i: usize) {
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 56.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let a = super::lists::stagger(ui.ctx(), i);
        let on_air = self.station_state(s);
        let h = widgets::hover_t(ui, resp.id, resp.hovered());
        if on_air.is_some() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), theme::alpha(theme::accent(), 0x30));
        } else if h > 0.0 {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), theme::white((0x0E as f32 * h) as u8));
        }
        let logo = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(40.0, 40.0));
        self.paint_logo(ui, logo, s);
        let saved = self.settings.radios.iter().any(|r| r.url == s.url);
        let heart = Rect::from_center_size(pos2(rect.right() - 26.0, rect.center().y), vec2(34.0, 34.0));
        let tx = logo.right() + 12.0;
        let tw = heart.left() - tx - 12.0;
        let color = theme::TEXT.gamma_multiply(a);
        let g = widgets::galley(ui, &s.name, theme::semibold(14.0), theme::TEXT, tw);
        ui.painter()
            .galley_with_override_text_color(pos2(tx, rect.top() + 10.0), g, color);
        let sub = on_air.unwrap_or_else(|| s.details());
        let g = widgets::galley(ui, &sub, FontId::proportional(12.5), theme::TEXT_2, tw);
        ui.painter().galley_with_override_text_color(
            pos2(tx, rect.top() + 30.0),
            g,
            theme::TEXT_2.gamma_multiply(a),
        );
        let (icon, tip) = if saved {
            (Icon::HeartFilled, tr("Retirer de mes stations"))
        } else {
            (Icon::Heart, tr("Ajouter à mes stations"))
        };
        if ui
            .put(heart, |ui: &mut egui::Ui| {
                widgets::toggle_icon(ui, saved, icon, tip, 34.0, 16.0)
            })
            .clicked()
        {
            self.actions.push(Action::ToggleStation(s.clone()));
        } else if resp.clicked() {
            self.actions.push(Action::PlayStation(s.clone()));
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr("Écouter"));
    }

    /// « En direct : titre », « Connexion… » si la station est la station en cours.
    fn station_state(&self, s: &Station) -> Option<String> {
        if self.radio.connecting_to().is_some_and(|c| c.url == s.url) {
            return Some(tr("Connexion…").into());
        }
        let live = self.radio.live.as_ref().filter(|l| l.station.url == s.url)?;
        if live.paused {
            return Some(tr("Arrêtée").into());
        }
        let title = live.info.title();
        Some(if title.is_empty() {
            tr("En direct").into()
        } else {
            trf!("En direct : {0}", title)
        })
    }

    /// Logo de la station, sinon une tuile colorée avec l'icône radio.
    pub(super) fn paint_logo(&mut self, ui: &egui::Ui, rect: Rect, s: &Station) {
        let radius = CornerRadius::same((rect.width() * 0.18) as u8);
        match self.radio.logo(ui.ctx(), s) {
            Some(tex) => {
                ui.painter().rect_filled(rect, radius, Color32::WHITE);
                let inner = rect.shrink(rect.width() * 0.06);
                ui.painter().add(
                    egui::epaint::RectShape::filled(inner, radius, Color32::WHITE)
                        .with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                );
            }
            None => {
                let (c1, c2) = theme::seeded_colors(&s.name);
                ui.painter().rect_filled(rect, radius, c1);
                let glow = pos2(
                    rect.right() - rect.width() * 0.25,
                    rect.top() + rect.height() * 0.25,
                );
                widgets::soft_glow(
                    ui.painter(),
                    rect,
                    glow,
                    rect.width() * 0.9,
                    theme::alpha(c2, 0x80),
                );
                icons::paint_centered(
                    ui.painter(),
                    Icon::Radio,
                    rect.center(),
                    rect.width() * 0.45,
                    Color32::WHITE,
                );
            }
        }
    }

    /// Fenêtre « Ajouter une station par son adresse ».
    pub(super) fn radio_dialog(&mut self, ctx: &egui::Context) {
        let Some((name, url)) = &mut self.radio.add else {
            return;
        };
        let mut close = false;
        let mut add = None;
        let modal = egui::Modal::new(egui::Id::new("radio_add"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .inner_margin(24.0)
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(RichText::new(tr("Ajouter une station")).font(theme::semibold(20.0)));
                ui.add_space(12.0);
                ui.add(
                    egui::TextEdit::singleline(name)
                        .desired_width(f32::INFINITY)
                        .margin(vec2(10.0, 8.0))
                        .hint_text(tr("Nom de la station")),
                );
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(url)
                        .desired_width(f32::INFINITY)
                        .margin(vec2(10.0, 8.0))
                        .hint_text(tr("Adresse du flux (http://…, .pls ou .m3u)")),
                );
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let ok = url.trim().starts_with("http");
                        if ui
                            .add_enabled_ui(ok, |ui| widgets::primary_button(ui, None, tr("Ajouter")))
                            .inner
                            .clicked()
                        {
                            let n = match name.trim() {
                                "" => url.trim().to_owned(),
                                n => n.to_owned(),
                            };
                            add = Some(Station {
                                name: n,
                                url: url.trim().to_owned(),
                                ..Default::default()
                            });
                        }
                        if widgets::secondary_button(ui, None, tr("Annuler")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if modal.should_close() {
            close = true;
        }
        if let Some(s) = add {
            self.settings.radios.push(s.clone());
            self.radio.add = None;
            self.actions.push(Action::PlayStation(s));
        } else if close {
            self.radio.add = None;
        }
    }
}

/// Colonnes et largeur des vignettes de « Mes stations ».
fn station_grid(width: f32) -> (usize, f32) {
    let cols = ((width + 12.0) / (240.0 + 12.0)).floor().max(1.0) as usize;
    (cols, (width - 12.0 * (cols - 1) as f32) / cols as f32)
}
