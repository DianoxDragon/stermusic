//! "Web radio" page: my stations, the directory, and live listening.

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

/// Genres offered as shortcuts (displayed name, directory tag).
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

/// Station being listened to.
pub struct Live {
    pub station: Station,
    pub info: LiveInfo,
    /// Stopped by the user (a live stream doesn't pause: it restarts).
    pub paused: bool,
    /// Reconnections attempted since the last drop.
    retries: u8,
    /// Last title on air seen by the interface (notification when it changes).
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
    /// Casting: looking up the stream address, which the device will play itself.
    pub cast_rx: Option<(Station, Receiver<Result<String, String>>)>,
    query: String,
    /// Directory filters (name, genre, country), and the search shown.
    filter: Query,
    results: Vec<Station>,
    shown_query: Option<Query>,
    searching: Option<Receiver<Result<Vec<Station>, String>>>,
    error: Option<String>,
    logos: HashMap<String, Logo>,
    logo_tx: Sender<(String, Option<egui::ColorImage>)>,
    logo_rx: Receiver<(String, Option<egui::ColorImage>)>,
    /// Adding a station by its address: name and address typed in.
    pub add: Option<(String, String)>,
    /// Directory countries (loaded on the first visit), and the filter of the country picker.
    countries: Vec<radio::Country>,
    countries_rx: Option<Receiver<Result<Vec<radio::Country>, String>>>,
    country_search: String,
    /// Globe of the world's stations.
    pub globe: super::radio_globe::GlobeState,
}

impl Default for RadioState {
    fn default() -> Self {
        let (logo_tx, logo_rx) = channel();
        Self {
            live: None,
            connecting: None,
            cast_rx: None,
            query: String::new(),
            filter: Query {
                country: radio::desktop_country(),
                ..Default::default()
            },
            results: Vec::new(),
            shown_query: None,
            searching: None,
            error: None,
            logos: HashMap::new(),
            logo_tx,
            logo_rx,
            add: None,
            countries: Vec::new(),
            countries_rx: None,
            country_search: String::new(),
            globe: Default::default(),
        }
    }
}

impl RadioState {
    /// Station playing (or connecting): its name.
    pub fn connecting_to(&self) -> Option<&Station> {
        self.connecting
            .as_ref()
            .map(|(s, _)| s)
            .or(self.cast_rx.as_ref().map(|(s, _)| s))
    }

    /// Current station (when cast: without the title on air).
    pub fn set_live(&mut self, station: Station, info: LiveInfo) {
        if let Some(old) = self.live.take() {
            old.info.stop.store(true, Ordering::Relaxed);
        }
        self.live = Some(Live {
            station,
            info,
            paused: false,
            retries: 0,
            last_title: String::new(),
        });
    }

    /// Logo of a station (downloaded in the background on first request).
    pub fn logo(&mut self, ctx: &egui::Context, station: &Station) -> Option<egui::TextureHandle> {
        self.image(ctx, &station.favicon)
    }

    /// Online image (station logo, podcast image), downloaded on first request.
    pub fn image(&mut self, ctx: &egui::Context, url: &str) -> Option<egui::TextureHandle> {
        if url.is_empty() {
            return None;
        }
        match self.logos.get(url) {
            Some(Logo::Ready(t)) => return Some(t.clone()),
            Some(_) => return None,
            None => {}
        }
        self.logos.insert(url.to_owned(), Logo::Loading);
        let (url, tx, ctx) = (url.to_owned(), self.logo_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let image = radio::fetch_image(&url).and_then(|bytes| decode_logo(&bytes));
            let _ = tx.send((url, image));
            ctx.request_repaint();
        });
        None
    }

    /// Name of a country from its code ("FR" → "France"), once the list is loaded.
    pub fn country_label(&self, code: &str) -> String {
        self.countries
            .iter()
            .find(|c| c.code == code)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| code.to_owned())
    }

    /// Country of the directory filter (the desktop's if there is none).
    pub fn filter_country(&self) -> String {
        if self.filter.country.is_empty() {
            radio::desktop_country()
        } else {
            self.filter.country.clone()
        }
    }

    fn load_countries(&mut self, ctx: &egui::Context) {
        if !self.countries.is_empty() || self.countries_rx.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(radio::countries());
            ctx.request_repaint();
        });
        self.countries_rx = Some(rx);
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

/// Logo as a small square texture (the format is guessed; an SVG or a .ico icon fails).
fn decode_logo(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = img
        .resize(320, 320, image::imageops::FilterType::Triangle)
        .to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [img.width() as usize, img.height() as usize],
        img.as_raw(),
    ))
}

impl SterMusic {
    /// Starts listening to a station (the connection happens in the background; the music playing
    /// goes on until the station answers).
    pub(super) fn play_station(&mut self, ctx: &egui::Context, station: Station) {
        if self.player.as_ref().is_some_and(|p| p.casting().is_some()) {
            self.play_station_cast(ctx, station);
            return;
        }
        let (tx, rx) = channel();
        let s = station.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(radio::connect(&s));
            ctx.request_repaint();
        });
        self.radio.connecting = Some((station, rx));
    }

    /// Stops the current station (it stays shown, ready to be restarted).
    pub(super) fn pause_station(&mut self) {
        if let Some(p) = &mut self.player {
            p.stop();
        }
        if let Some(live) = &mut self.radio.live {
            live.info.stop.store(true, Ordering::Relaxed);
            live.paused = true;
        }
    }

    /// Leaves the radio (a library track is about to play).
    pub(super) fn leave_radio(&mut self) {
        if let Some(live) = self.radio.live.take() {
            live.info.stop.store(true, Ordering::Relaxed);
        }
        self.radio.connecting = None;
        self.radio.cast_rx = None;
    }

    /// Next (or previous) station among "My stations".
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
        if let Some(rx) = &self.radio.countries_rx {
            if let Ok(result) = rx.try_recv() {
                self.radio.countries_rx = None;
                if let Ok(list) = result {
                    self.radio.countries = list;
                }
            }
        }
        self.poll_globe(ctx);
        if let Some(rx) = &self.radio.searching {
            if let Ok(result) = rx.try_recv() {
                self.radio.searching = None;
                match result {
                    Ok(list) => {
                        self.radio.results = list;
                        // New results arrive in a cascade, as when the page opens.
                        super::lists::restart_cascade(ctx);
                    }
                    Err(e) => {
                        self.radio.results.clear();
                        self.radio.error = Some(trf!("Annuaire injoignable : {e}", e));
                    }
                }
            }
        }
        let cast_ready = match &self.radio.cast_rx {
            Some((_, rx)) => rx.try_recv().ok(),
            None => None,
        };
        if let Some(result) = cast_ready {
            let (station, _) = self.radio.cast_rx.take().unwrap();
            match result {
                Ok(url) => self.cast_station_ready(station, url),
                Err(e) => self.toast(format!("{} — {e}", station.name)),
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
        // Stream dropped: retry three times, then stop.
        let ended = self.radio.live.as_ref().is_some_and(|l| !l.paused)
            && self.radio.connecting.is_none()
            && self.radio.cast_rx.is_none()
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
        // Title on air: it changes while listening.
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
        // Station added by its address, without a name: the one given by the server.
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
        self.leave_episode();
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

    /// Adds (or removes) a station from "My stations".
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

    /// Page header: title, Directory / Globe switch, add an address.
    pub(super) fn radio_header(&mut self, ui: &mut egui::Ui) {
        self.radio.load_countries(&ui.ctx().clone());
        ui.horizontal(|ui| {
            heading(ui, tr("Webradios"));
            ui.add_space(18.0);
            ui.spacing_mut().item_spacing.x = 6.0;
            if super::views::segment(ui, tr("Annuaire"), !self.settings.radio_globe).clicked() {
                self.settings.radio_globe = false;
            }
            if super::views::segment(ui, tr("Globe"), self.settings.radio_globe).clicked() {
                self.settings.radio_globe = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if widgets::secondary_button(ui, Some(Icon::Link), tr("Ajouter une adresse…")).clicked() {
                    self.radio.add = Some((String::new(), String::new()));
                }
            });
        });
    }

    /// Web radio page, Directory view (the Globe view takes the whole area: see `globe_view`).
    pub(super) fn radio_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.radio_header(ui);
        ui.add_space(14.0);
        if self.radio.shown_query.is_none() {
            let q = self.radio.filter.clone();
            self.radio.search(&ctx, q);
        }
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
                let mut launch = false;
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 360.0).clamp(160.0, 420.0);
                    let field = widgets::search_field(
                        ui,
                        egui::Id::new("radio_search"),
                        &mut self.radio.query,
                        tr("Nom d'une station"),
                        w,
                    );
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let go = widgets::secondary_button(ui, Some(Icon::Search), tr("Chercher")).clicked();
                    // Field cleared: the search by name stops too.
                    let name = self.radio.query.trim().to_owned();
                    if enter || go || (name.is_empty() && !self.radio.filter.name.is_empty()) {
                        self.radio.filter.name = name;
                        launch = true;
                    }
                    ui.add_space(8.0);
                    if self.country_picker(ui) {
                        launch = true;
                    }
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    if super::views::segment(ui, tr("Tous les genres"), self.radio.filter.tag.is_empty())
                        .clicked()
                    {
                        self.radio.filter.tag.clear();
                        launch = true;
                    }
                    for (label, tag) in GENRES {
                        if super::views::segment(ui, tr(label), self.radio.filter.tag == *tag).clicked() {
                            self.radio.filter.tag = if self.radio.filter.tag == *tag {
                                String::new()
                            } else {
                                (*tag).to_owned()
                            };
                            launch = true;
                        }
                    }
                });
                if launch && self.radio.shown_query.as_ref() != Some(&self.radio.filter) {
                    let q = self.radio.filter.clone();
                    self.radio.search(&ctx, q);
                }
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

    /// Country button, with a list to filter; true if the choice changed.
    fn country_picker(&mut self, ui: &mut egui::Ui) -> bool {
        let code = self.radio.filter.country.clone();
        let label = if code.is_empty() {
            tr("Tous les pays").to_owned()
        } else {
            self.radio.country_label(&code)
        };
        let resp = widgets::secondary_button(ui, Some(Icon::Globe), &label);
        let popup_id = ui.make_persistent_id("country_popup");
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup_id));
        }
        let mut changed = false;
        egui::popup::popup_below_widget(
            ui,
            popup_id,
            &resp,
            egui::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                ui.set_width(280.0);
                let field = widgets::search_field(
                    ui,
                    egui::Id::new("country_search"),
                    &mut self.radio.country_search,
                    tr("Chercher un pays"),
                    270.0,
                );
                if !field.has_focus() && ui.memory(|m| m.focused().is_none()) {
                    field.request_focus();
                }
                ui.add_space(4.0);
                let q = crate::library::fold(self.radio.country_search.trim());
                egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                    let mut pick = None;
                    if q.is_empty()
                        && widgets::menu_item(ui, Some(Icon::Globe), tr("Tous les pays")).clicked()
                    {
                        pick = Some(String::new());
                    }
                    if self.radio.countries.is_empty() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(tr("Chargement des pays…")).color(theme::TEXT_3));
                        });
                    }
                    for c in &self.radio.countries {
                        if !q.is_empty()
                            && !crate::library::fold(&c.name).contains(&q)
                            && c.code.to_ascii_lowercase() != q
                        {
                            continue;
                        }
                        let text = format!("{} · {}", c.name, c.stations);
                        let icon = (c.code == code).then_some(Icon::Radio);
                        if widgets::menu_item(ui, icon, &text).clicked() {
                            pick = Some(c.code.clone());
                        }
                    }
                    if let Some(p) = pick {
                        self.radio.filter.country = p;
                        self.radio.country_search.clear();
                        changed = true;
                        ui.memory_mut(|m| m.close_popup());
                    }
                });
            },
        );
        changed
    }

    /// Station thumbnail (logo or color, name): a click starts listening.
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

    /// Row of a directory result.
    pub(super) fn station_row(&mut self, ui: &mut egui::Ui, s: &Station, i: usize) {
        let w = ui.available_width();
        let (slot, resp) = ui.allocate_exact_size(vec2(w, 56.0), Sense::click());
        if !ui.is_rect_visible(slot) {
            return;
        }
        // Cascade arrival: each row slides in a little from the right as it appears.
        let a = super::lists::stagger(ui.ctx(), i);
        let rect = slot.translate(vec2(14.0 * (1.0 - a), 0.0));
        ui.scope(|ui| {
            ui.multiply_opacity(a);
            self.station_row_inner(ui, s, rect, resp);
        });
    }

    fn station_row_inner(&mut self, ui: &mut egui::Ui, s: &Station, rect: Rect, resp: egui::Response) {
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
        let g = widgets::galley(ui, &s.name, theme::semibold(14.0), theme::TEXT, tw);
        ui.painter()
            .galley_with_override_text_color(pos2(tx, rect.top() + 10.0), g, theme::TEXT);
        let sub = on_air.unwrap_or_else(|| s.details());
        let g = widgets::galley(ui, &sub, FontId::proportional(12.5), theme::TEXT_2, tw);
        ui.painter().galley_with_override_text_color(
            pos2(tx, rect.top() + 30.0),
            g,
            theme::TEXT_2,
        );
        let (icon, tip) = if saved {
            (Icon::HeartFilled, tr("Retirer de mes stations"))
        } else {
            (Icon::Heart, tr("Ajouter à mes stations"))
        };
        if widgets::toggle_icon(
            &mut ui.new_child(egui::UiBuilder::new().max_rect(heart)),
            saved,
            icon,
            tip,
            34.0,
            16.0,
        )
        .clicked()
        {
            self.actions.push(Action::ToggleStation(s.clone()));
        } else if resp.clicked() {
            self.actions.push(Action::PlayStation(s.clone()));
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr("Écouter"));
    }

    /// "Live: title", "Connecting…" if the station is the current one.
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

    /// Logo of the station, otherwise a colored tile with the radio icon.
    pub(super) fn paint_logo(&mut self, ui: &egui::Ui, rect: Rect, s: &Station) {
        let radius = CornerRadius::same((rect.width() * 0.18) as u8);
        match self.radio.logo(ui.ctx(), s) {
            Some(tex) => {
                // Logo just downloaded: it fades in.
                let t = widgets::texture_in(ui.ctx(), &tex);
                let mut p = ui.painter().clone();
                p.multiply_opacity(t);
                if t < 1.0 {
                    let (c1, _) = theme::seeded_colors(&s.name);
                    ui.painter().rect_filled(rect, radius, c1.gamma_multiply(1.0 - t));
                }
                p.rect_filled(rect, radius, Color32::WHITE);
                let inner = rect.shrink(rect.width() * 0.06);
                p.add(
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

    /// "Add a station by its address" window.
    pub(super) fn radio_dialog(&mut self, ctx: &egui::Context) {
        let Some((name, url)) = &mut self.radio.add else {
            return;
        };
        let mut close = false;
        let mut add = None;
        let modal = crate::widgets::modal(ctx, egui::Id::new("radio_add"))
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

/// Columns and width of the "My stations" thumbnails.
fn station_grid(width: f32) -> (usize, f32) {
    let cols = ((width + 12.0) / (240.0 + 12.0)).floor().max(1.0) as usize;
    (cols, (width - 12.0 * (cols - 1) as f32) / cols as f32)
}
