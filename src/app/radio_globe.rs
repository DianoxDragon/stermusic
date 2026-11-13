//! Web radio globe: stations from all over the world, in their place on Earth. The globe is
//! spun with the mouse and zoomed with the wheel, and clicking a point opens the list of the
//! stations of that place.

use std::sync::mpsc::{channel, Receiver};

use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke};

use super::SterMusic;
use crate::globe::{self, View};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::radio::{self, Place, Station};
use crate::trf;
use crate::{theme, widgets};

/// Width of the panel listing a place's stations.
const PANEL_W: f32 = 330.0;
/// Minimum and maximum zoom (1: the globe fits in the area).
const ZOOM: (f32, f32) = (0.8, 60.0);

pub struct GlobeState {
    pub world: Vec<Station>,
    places: Vec<Place>,
    loading: Option<Receiver<Result<Vec<Station>, String>>>,
    error: Option<String>,
    earth: Option<egui::TextureHandle>,
    earth_rx: Option<Receiver<Option<egui::ColorImage>>>,
    /// Orientation (radians) and zoom.
    lon: f32,
    lat: f32,
    zoom: f32,
    /// Momentum after a drag (points per second).
    velocity: egui::Vec2,
    /// True as soon as the user has touched the globe (it then stops spinning by itself).
    touched: bool,
    /// Flight to a point (latitude, longitude in radians, target zoom).
    fly_to: Option<(f32, f32, f32)>,
    selected: Option<usize>,
    /// Last place opened: the panel keeps it shown while sliding off the map.
    shown: Option<usize>,
    /// Country the globe turned to (it follows the directory's country choice).
    centered_on: Option<String>,
    /// Detailed images of the displayed region: the new one, and the old one staying below during
    /// the cross-fade; the block of tiles being assembled; the time of the last movement (we wait
    /// for the map to stop before requesting tiles).
    detail: Option<DetailTex>,
    detail_prev: Option<DetailTex>,
    detail_rx: Option<Receiver<Option<(globe::TileRange, egui::ColorImage)>>>,
    last_motion: f64,
    /// Download failed: try again a little later.
    detail_retry_at: f64,
}

struct DetailTex {
    texture: egui::TextureHandle,
    range: globe::TileRange,
    since: f64,
}

impl DetailTex {
    fn frame(&self, now: f64) -> globe::Detail {
        globe::Detail {
            texture: self.texture.id(),
            bounds: self.range.bounds(),
            alpha: ((now - self.since) / 0.3).clamp(0.0, 1.0) as f32,
        }
    }
}

impl Default for GlobeState {
    fn default() -> Self {
        Self {
            world: Vec::new(),
            places: Vec::new(),
            loading: None,
            error: None,
            earth: None,
            earth_rx: None,
            lon: 10f32.to_radians(),
            lat: 35f32.to_radians(),
            zoom: 1.0,
            velocity: egui::Vec2::ZERO,
            touched: false,
            fly_to: None,
            selected: None,
            shown: None,
            centered_on: None,
            detail: None,
            detail_prev: None,
            detail_rx: None,
            last_motion: 0.0,
            detail_retry_at: 0.0,
        }
    }
}

impl GlobeState {
    /// Starts loading the stations (cache, otherwise directory) and the Earth image.
    fn ensure_loaded(&mut self, ctx: &egui::Context, force_refresh: bool) {
        if self.earth.is_none() && self.earth_rx.is_none() {
            let (tx, rx) = channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(globe::decode_earth());
                ctx.request_repaint();
            });
            self.earth_rx = Some(rx);
        }
        if (self.world.is_empty() || force_refresh) && self.loading.is_none() {
            let (tx, rx) = channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let cached = radio::load_world();
                let result = match cached {
                    Some((list, true)) if !force_refresh => Ok(list),
                    stale => match radio::fetch_world() {
                        Ok(list) => Ok(list),
                        // No network: the old cache is better than nothing.
                        Err(e) => stale.map(|(list, _)| list).ok_or(e),
                    },
                };
                let _ = tx.send(result);
                ctx.request_repaint();
            });
            self.loading = Some(rx);
            self.error = None;
        }
    }

    fn fly(&mut self, lat_deg: f32, lon_deg: f32, zoom: f32) {
        self.fly_to = Some((
            lat_deg.to_radians().clamp(-globe::MAX_LAT, globe::MAX_LAT),
            lon_deg.to_radians(),
            zoom,
        ));
        self.velocity = egui::Vec2::ZERO;
        self.touched = true;
    }
}

impl SterMusic {
    pub(super) fn poll_globe(&mut self, ctx: &egui::Context) {
        let g = &mut self.radio.globe;
        if let Some(rx) = &g.earth_rx {
            if let Ok(img) = rx.try_recv() {
                g.earth_rx = None;
                if let Some(img) = img {
                    let options = egui::TextureOptions {
                        magnification: egui::TextureFilter::Linear,
                        minification: egui::TextureFilter::Linear,
                        wrap_mode: egui::TextureWrapMode::Repeat,
                        mipmap_mode: Some(egui::TextureFilter::Linear),
                    };
                    g.earth = Some(ctx.load_texture("earth", img, options));
                }
            }
        }
        if let Some(rx) = &g.detail_rx {
            if let Ok(result) = rx.try_recv() {
                g.detail_rx = None;
                let now = ctx.input(|i| i.time);
                match result {
                    Some((range, img)) => {
                        let options = egui::TextureOptions {
                            magnification: egui::TextureFilter::Linear,
                            minification: egui::TextureFilter::Linear,
                            wrap_mode: egui::TextureWrapMode::ClampToEdge,
                            mipmap_mode: Some(egui::TextureFilter::Linear),
                        };
                        // The previous image stays below while the new one appears.
                        g.detail_prev = g.detail.take();
                        g.detail = Some(DetailTex {
                            texture: ctx.load_texture("earth_detail", img, options),
                            range,
                            since: now,
                        });
                    }
                    None => g.detail_retry_at = now + 5.0,
                }
            }
        }
        if let Some(rx) = &g.loading {
            if let Ok(result) = rx.try_recv() {
                g.loading = None;
                match result {
                    Ok(world) => {
                        g.places = radio::places(&world);
                        g.world = world;
                        g.selected = None;
                        g.shown = None;
                    }
                    Err(e) => g.error = Some(trf!("Annuaire injoignable : {e}", e)),
                }
            }
        }
    }

    /// "Globe" view of the Web radio page: the Earth fills the whole content area, the header, the
    /// controls and the panel of a place float on top.
    pub(super) fn globe_view(&mut self, ui: &mut egui::Ui, area: Rect) {
        let ctx = ui.ctx().clone();
        self.radio.globe.ensure_loaded(&ctx, false);
        let panel_open = self.radio.globe.selected.is_some();
        // What floats on the map: header, control bar, place panel.
        let header = Rect::from_min_size(area.min + vec2(32.0, 24.0), vec2(area.width() - 48.0, 40.0));
        let toolbar = Rect::from_min_max(
            pos2(area.left() + 28.0, header.bottom() + 10.0),
            pos2(area.right() - 16.0, header.bottom() + 44.0),
        );
        let panel = Rect::from_min_max(
            pos2(area.right() - 16.0 - PANEL_W, toolbar.bottom() + 10.0),
            area.max - vec2(16.0, 16.0),
        );
        ui.allocate_rect(area, Sense::hover());
        let resp = ui.interact(area, ui.id().with("globe"), Sense::click_and_drag());
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        // Pointer on the map, outside what floats over it.
        let over_ui = |p: egui::Pos2| p.y < toolbar.bottom() || (panel_open && panel.contains(p));
        let pointer = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| area.contains(*p) && !over_ui(*p));

        // The globe turns to the country chosen in the directory (at first, the desktop's).
        let country = self.radio.filter_country();
        if self.radio.globe.centered_on.as_ref() != Some(&country) && !self.radio.globe.world.is_empty() {
            let first = self.radio.globe.centered_on.is_none();
            self.radio.globe.centered_on = Some(country.clone());
            if let Some((lat, lon)) = radio::country_center(&self.radio.globe.world, &country) {
                let g = &mut self.radio.globe;
                let zoom = if first { g.zoom } else { g.zoom.max(3.0) };
                g.fly_to = Some((lat.to_radians(), lon.to_radians(), zoom));
            }
        }

        // Usable area under the header; the globe slides left when the panel opens.
        let shift = crate::anim::toggle(
            &ctx,
            egui::Id::new("globe_panel_shift"),
            panel_open,
            crate::anim::NORMAL,
        );
        let usable = Rect::from_min_max(pos2(area.left(), toolbar.bottom()), area.max);
        let center = usable.center() - vec2((PANEL_W + 16.0) * 0.5 * shift, 0.0);
        let base = (usable.width() - (PANEL_W + 16.0) * shift).min(usable.height()) * 0.46;
        let g = &mut self.radio.globe;
        let mut view = View {
            lon: g.lon,
            lat: g.lat,
            center,
            radius: base * g.zoom,
        };

        // Drag: the globe follows the mouse; released, it keeps a bit of momentum.
        if resp.dragged() {
            let d = resp.drag_delta();
            view.drag(d);
            g.velocity = d / dt.max(1.0 / 240.0);
            g.touched = true;
            g.fly_to = None;
        } else if g.velocity.length() > 5.0 {
            view.drag(g.velocity * dt);
            g.velocity *= (-dt * 4.0).exp();
        } else {
            g.velocity = egui::Vec2::ZERO;
        }
        // Wheel: zoom towards the point under the pointer.
        if pointer.is_some() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let before = pointer.and_then(|p| view.unproject(p));
                g.zoom = (g.zoom * (scroll * 0.004).exp()).clamp(ZOOM.0, ZOOM.1);
                view.radius = base * g.zoom;
                if let (Some(p), Some((lat0, lon0))) = (pointer, before) {
                    if let Some((lat1, lon1)) = view.unproject(p) {
                        view.lat =
                            (view.lat + (lat0 - lat1).to_radians()).clamp(-globe::MAX_LAT, globe::MAX_LAT);
                        view.lon = globe::wrap_rad(view.lon + (lon0 - lon1).to_radians());
                    }
                }
                g.touched = true;
                g.fly_to = None;
            }
        }
        // Smooth flight to a place.
        if let Some((lat, lon, zoom)) = g.fly_to {
            let k = 1.0 - (-dt * 5.0).exp();
            let dlon = globe::wrap_rad(lon - view.lon);
            view.lon = globe::wrap_rad(view.lon + dlon * k);
            view.lat += (lat - view.lat) * k;
            g.zoom += (zoom - g.zoom) * k;
            view.radius = base * g.zoom;
            if dlon.abs() < 0.001 && (lat - view.lat).abs() < 0.001 && (zoom - g.zoom).abs() < 0.01 {
                g.fly_to = None;
            }
        } else if !g.touched && self.focused {
            // As long as nobody touches it, the Earth spins gently.
            view.lon = globe::wrap_rad(view.lon + dt * 0.08);
        }
        g.lon = view.lon;
        g.lat = view.lat;
        let moving = g.fly_to.is_some() || g.velocity != egui::Vec2::ZERO || (!g.touched && self.focused);
        let now = ui.input(|i| i.time);
        let scrolled = ui.input(|i| i.smooth_scroll_delta.y != 0.0);
        if moving || resp.dragged() || scrolled {
            g.last_motion = now;
        }
        let fresh = g.detail.as_ref().map(|d| d.frame(now));
        if fresh.is_some_and(|d| d.alpha >= 1.0) {
            g.detail_prev = None;
        } else if fresh.is_some() {
            ctx.request_repaint();
        }
        let details = [
            g.detail_prev.as_ref().map(|d| globe::Detail {
                alpha: 1.0,
                ..d.frame(now)
            }),
            fresh,
        ];
        if moving || resp.dragged() || shift > 0.0 && shift < 1.0 {
            ctx.request_repaint();
        }

        self.request_detail(&ctx, &view, area, now);
        let g = &mut self.radio.globe;
        let painter = ui.painter_at(area);
        let glow = theme::accent_light();
        match &g.earth {
            Some(tex) if globe::available() => globe::paint(
                &painter,
                area,
                globe::Frame {
                    view,
                    earth: tex.id(),
                    details,
                    glow,
                    corner: 10.0,
                },
            ),
            _ => {
                painter.circle_filled(view.center, view.radius, Color32::from_rgb(0x0C, 0x16, 0x3A));
                painter.circle_stroke(
                    view.center,
                    view.radius,
                    Stroke::new(1.5_f32, theme::alpha(glow, 0x80)),
                );
            }
        }

        // Stations: one dot per place, bigger when there are many stations.
        let live_url = self
            .radio
            .live
            .as_ref()
            .map(|l| l.station.url.clone())
            .or_else(|| self.radio.connecting_to().map(|s| s.url.clone()));
        let g = &self.radio.globe;
        let mut nearest: Option<(usize, f32)> = None;
        let scale = (0.85 + g.zoom.sqrt() * 0.15).min(1.8);
        let time = ui.input(|i| i.time) as f32;
        let mut live_place = None;
        // Dots in the theme colors, lightened to stand out on the map.
        let dot = theme::accent_light().lerp_to_gamma(Color32::WHITE, 0.25);
        for (k, place) in g.places.iter().enumerate() {
            let Some((p, z)) = view.project(place.lat, place.lon) else {
                continue;
            };
            // Nothing under the panel: only blotches of it would show through its background.
            if !area.expand(10.0).contains(p) || (panel_open && panel.expand(4.0).contains(p)) {
                continue;
            }
            // Dots fade out as they approach the edge of the globe.
            let fade = ((z - 0.05) / 0.35).clamp(0.0, 1.0);
            let fade = fade * fade * (3.0 - 2.0 * fade);
            if fade <= 0.01 {
                continue;
            }
            let n = place.stations.len() as f32;
            let r = (1.2 + n.sqrt() * 0.6).min(5.0) * scale;
            if n >= 12.0 {
                painter.circle_filled(p, r * 1.9, theme::alpha(glow, (14.0 * fade) as u8));
            }
            painter.circle_filled(p, r + 0.8, Color32::from_black_alpha((90.0 * fade) as u8));
            painter.circle_filled(p, r, dot.gamma_multiply(fade));
            if n >= 12.0 {
                painter.circle_filled(p, r * 0.45, Color32::WHITE.gamma_multiply(0.8 * fade));
            }
            if let Some(ptr) = pointer {
                let d = ptr.distance(p);
                if d < (r + 8.0) && nearest.is_none_or(|(_, best)| d < best) {
                    nearest = Some((k, d));
                }
            }
            if live_url
                .as_ref()
                .is_some_and(|u| place.stations.iter().any(|&i| g.world[i].url == *u))
            {
                live_place = Some(p);
            }
        }
        // Chosen place: the ring tightens on the dot, with a ripple escaping from it.
        let ring = crate::anim::pulse(
            &ctx,
            egui::Id::new("globe_ring"),
            g.selected.map_or(u64::MAX, |k| k as u64),
            crate::anim::SLOW * 2.0,
        );
        if let Some(sel) = g.selected.and_then(|k| g.places.get(k)) {
            if let Some((p, _)) = view.project(sel.lat, sel.lon) {
                painter.circle_stroke(
                    p,
                    11.0 + 8.0 * ring,
                    Stroke::new(2.0_f32, Color32::WHITE.gamma_multiply(1.0 - 0.6 * ring)),
                );
                if ring > 0.0 {
                    painter.circle_stroke(
                        p,
                        11.0 + 26.0 * (1.0 - ring),
                        Stroke::new(1.5_f32, Color32::WHITE.gamma_multiply(0.6 * ring)),
                    );
                }
            }
        }
        if let Some(p) = live_place {
            // Place of the current station: a widening ripple.
            let t = (time * 0.8).fract();
            painter.circle_stroke(
                p,
                6.0 + 18.0 * t,
                Stroke::new(2.0_f32, Color32::WHITE.gamma_multiply(1.0 - t)),
            );
            ctx.request_repaint();
        }

        // Hover and click.
        if let Some((k, _)) = nearest {
            let place = &g.places[k];
            let label = format!(
                "{} · {}",
                place.full_name(),
                crate::i18n::plural(place.stations.len(), "station", "stations")
            );
            if let Some(ptr) = pointer {
                // The tooltip of the hovered place fades in, moving up a little.
                let t = crate::anim::appear_each(
                    &ctx,
                    egui::Id::new(("globe_tip", k)),
                    crate::anim::FAST * 1.5,
                    crate::anim::Easing::Out,
                );
                let galley = widgets::galley(ui, &label, FontId::proportional(13.0), theme::TEXT, 320.0);
                let r = Rect::from_min_size(ptr + vec2(14.0, -30.0 + 6.0 * (1.0 - t)), galley.size() + vec2(16.0, 10.0));
                painter.rect_filled(r, CornerRadius::same(6), theme::alpha(theme::bg(), 0xE8).gamma_multiply(t));
                painter.galley(r.min + vec2(8.0, 5.0), galley, theme::TEXT.gamma_multiply(t));
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() && pointer.is_some() {
            self.radio.globe.selected = nearest.map(|(k, _)| k);
        }
        if resp.double_clicked() {
            if let Some((lat, lon)) = pointer.and_then(|p| view.unproject(p)) {
                let zoom = (self.radio.globe.zoom * 2.5).min(ZOOM.1);
                self.radio.globe.fly(lat, lon, zoom);
            }
        }

        // Gradient veils at the top and bottom: the header and the captions stay readable.
        let bg = theme::bg();
        // The top veil follows the rounded corner of the area (otherwise it draws a square one).
        vertical_fade(
            &painter,
            Rect::from_min_max(area.min, pos2(area.right(), toolbar.bottom() + 36.0)),
            theme::alpha(bg, 0xE0),
            theme::alpha(bg, 0x00),
            10.0,
        );
        vertical_fade(
            &painter,
            Rect::from_min_max(pos2(area.left(), area.bottom() - 56.0), area.max),
            theme::alpha(bg, 0x00),
            theme::alpha(bg, 0xB0),
            0.0,
        );

        widgets::place(ui, header, |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                self.radio_header(ui)
            });
        });
        self.globe_overlay(ui, area, toolbar, panel_open);
        // The panel slides in from the right when opening, and slides back out when closing.
        if let Some(k) = self.radio.globe.selected {
            // Opening the panel: its stations arrive in a cascade (a change of place also restarts the
            // cascade, in `place_panel`).
            if self.radio.globe.shown.is_none() {
                super::lists::restart_cascade(&ctx);
            }
            self.radio.globe.shown = Some(k);
        }
        if shift > 0.01 {
            if let Some(k) = self.radio.globe.shown {
                let slide = panel.translate(vec2((PANEL_W * 0.35) * (1.0 - shift), 0.0));
                self.place_panel(ui, slide, k, shift);
            }
        } else if !panel_open {
            self.radio.globe.shown = None;
        }
        // Border of the content area, on top of the map.
        ui.painter().rect_stroke(
            area,
            CornerRadius {
                nw: 10,
                ..Default::default()
            },
            Stroke::new(1.0_f32, theme::white(0x0C)),
            egui::StrokeKind::Inside,
        );
    }

    /// Requests the detailed tiles of the visible area, once the globe is still.
    fn request_detail(&mut self, ctx: &egui::Context, view: &View, area: Rect, now: f64) {
        let g = &mut self.radio.globe;
        if g.zoom < 1.4 || g.detail_rx.is_some() || now < g.detail_retry_at || !globe::available() {
            return;
        }
        let still = now - g.last_motion;
        if still < 0.25 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(0.3 - still));
            return;
        }
        let Some((bounds, need)) = visible_area(view, area, ctx.pixels_per_point()) else {
            return;
        };
        // The coarsest level that is enough for the screen; coarser still if the area needs too many
        // tiles.
        let mut level = (globe::MIN_LEVEL..=globe::MAX_LEVEL)
            .find(|&l| globe::level_px_per_deg(l) >= need * 0.85)
            .unwrap_or(globe::MAX_LEVEL);
        let mut range = globe::TileRange::covering(bounds, level);
        while range.count() > globe::MAX_TILES && level > globe::MIN_LEVEL {
            level -= 1;
            range = globe::TileRange::covering(bounds, level);
        }
        if range.count() > globe::MAX_TILES {
            return;
        }
        // The image already there covers the area finely enough: nothing to do.
        if g.detail.as_ref().is_some_and(|d| d.range.contains(&range)) {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(globe::mosaic(range).map(|img| (range, img)));
            ctx.request_repaint();
        });
        g.detail_rx = Some(rx);
    }

    /// Buttons and caption laid over the globe.
    fn globe_overlay(&mut self, ui: &mut egui::Ui, area: Rect, bar: Rect, panel_open: bool) {
        let g = &self.radio.globe;
        let status = if g.loading.is_some() && g.world.is_empty() {
            tr("Chargement des stations du monde…").to_owned()
        } else if let Some(e) = &g.error {
            e.clone()
        } else {
            trf!(
                "{0} dans {1}",
                crate::i18n::plural(g.world.len(), "station", "stations"),
                crate::i18n::plural(g.places.len(), "lieu", "lieux")
            )
        };
        let loading = g.loading.is_some();
        let shade = theme::alpha(theme::bg(), 0xB0);
        let status_w = widgets::galley(ui, &status, FontId::proportional(14.0), theme::TEXT, 600.0)
            .size()
            .x;
        let left_w = status_w + if loading { 40.0 } else { 16.0 };
        ui.painter().rect_filled(
            Rect::from_min_size(bar.min - vec2(4.0, 0.0), vec2(left_w, 32.0)),
            CornerRadius::same(8),
            shade,
        );
        ui.painter().rect_filled(
            Rect::from_min_max(
                pos2(bar.right() - 150.0, bar.top()),
                pos2(bar.right() + 4.0, bar.top() + 32.0),
            ),
            CornerRadius::same(8),
            shade,
        );
        widgets::place(ui, bar, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if loading {
                    ui.spinner();
                }
                ui.label(RichText::new(status).color(theme::TEXT_2));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, Icon::Plus, tr("Zoomer"), 32.0, 16.0).clicked() {
                        let g = &mut self.radio.globe;
                        let (lat, lon, z) =
                            (g.lat.to_degrees(), g.lon.to_degrees(), (g.zoom * 2.0).min(ZOOM.1));
                        g.fly(lat, lon, z);
                    }
                    if widgets::icon_button(ui, Icon::WinMinimize, tr("Dézoomer"), 32.0, 16.0).clicked() {
                        let g = &mut self.radio.globe;
                        let (lat, lon, z) =
                            (g.lat.to_degrees(), g.lon.to_degrees(), (g.zoom / 2.0).max(ZOOM.0));
                        g.fly(lat, lon, z);
                    }
                    if widgets::icon_button(ui, Icon::Home, tr("Revenir sur mon pays"), 32.0, 16.0).clicked()
                    {
                        let country = self.radio.filter_country();
                        if let Some((lat, lon)) = radio::country_center(&self.radio.globe.world, &country) {
                            self.radio.globe.fly(lat, lon, 4.0);
                        }
                    }
                    if !loading
                        && widgets::icon_button(
                            ui,
                            Icon::Refresh,
                            tr("Actualiser les stations du monde"),
                            32.0,
                            16.0,
                        )
                        .clicked()
                    {
                        self.radio.globe.ensure_loaded(&ui.ctx().clone(), true);
                    }
                });
            });
        });
        for (text, size, align, x) in [
            (
                tr("Glisse pour tourner, molette pour zoomer, clique un point pour voir ses stations."),
                11.5,
                Align2::LEFT_BOTTOM,
                area.left() + 32.0,
            ),
            (
                tr("Image : NASA · Villes : GeoNames · Stations : radio-browser.info"),
                10.5,
                Align2::RIGHT_BOTTOM,
                if panel_open {
                    area.right() - PANEL_W - 40.0
                } else {
                    area.right() - 24.0
                },
            ),
        ] {
            // Limited room (or panel open): only the credits remain.
            if align == Align2::LEFT_BOTTOM && (area.width() < 760.0 || panel_open) {
                continue;
            }
            let g = ui
                .painter()
                .layout_no_wrap(text.to_owned(), FontId::proportional(size), theme::TEXT_2);
            let r = align.anchor_size(pos2(x, area.bottom() - 12.0), g.size());
            ui.painter()
                .rect_filled(r.expand2(vec2(6.0, 3.0)), CornerRadius::same(5), shade);
            ui.painter().galley(r.min, g, theme::TEXT_2);
        }
    }

    /// Panel of the stations of the chosen place.
    fn place_panel(&mut self, ui: &mut egui::Ui, rect: Rect, k: usize, opacity: f32) {
        let Some(place) = self.radio.globe.places.get(k).cloned() else {
            return;
        };
        // Another place: the stations cascade in again.
        if crate::anim::key_changed(ui.ctx(), egui::Id::new("globe_place"), k as u64) {
            super::lists::restart_cascade(ui.ctx());
        }
        let mut scoped = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
        scoped.multiply_opacity(opacity);
        let ui = &mut scoped;
        // Panel laid over the map: soft shadow, almost opaque background.
        ui.painter().add(
            egui::Shadow {
                offset: [0, 6],
                blur: 22,
                spread: 0,
                color: Color32::from_black_alpha(110),
            }
            .as_shape(rect, CornerRadius::same(12)),
        );
        ui.painter()
            .rect_filled(rect, CornerRadius::same(12), theme::alpha(theme::bg(), 0xF0));
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(12),
            Stroke::new(1.0_f32, theme::white(0x14)),
            egui::StrokeKind::Inside,
        );
        let inner = rect.shrink2(vec2(10.0, 12.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        {
            let ui = &mut child;
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(inner.width() - 40.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(&place.name)
                                .font(theme::semibold(18.0))
                                .color(theme::TEXT),
                        )
                        .wrap(),
                    );
                    let count = crate::i18n::plural(place.stations.len(), "station", "stations");
                    let sub = if place.region.is_empty() || place.region == place.name {
                        count
                    } else {
                        format!("{} · {count}", place.region)
                    };
                    ui.add(egui::Label::new(RichText::new(sub).small().color(theme::TEXT_3)).wrap());
                });
                if widgets::icon_button(ui, Icon::Close, tr("Fermer"), 30.0, 15.0).clicked() {
                    self.radio.globe.selected = None;
                }
            });
            ui.add_space(8.0);
            let stations: Vec<Station> = place
                .stations
                .iter()
                .map(|&i| self.radio.globe.world[i].clone())
                .collect();
            egui::ScrollArea::vertical()
                .id_salt(("place", &place.name))
                .auto_shrink(false)
                .show(ui, |ui| {
                    for (i, s) in stations.iter().enumerate() {
                        self.station_row(ui, s, i);
                    }
                });
        }
    }
}

/// Rectangle with a vertical gradient (veil above the map), the top left corner rounded by
/// `corner` points.
fn vertical_fade(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32, corner: f32) {
    let color_at = |y: f32| {
        let t = ((y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        top.lerp_to_gamma(bottom, t)
    };
    // (Convex) outline: the rounded corner, then the other three corners.
    let mut outline = Vec::new();
    if corner > 0.0 {
        let c = rect.left_top() + vec2(corner, corner);
        for k in 0..=8 {
            let a = std::f32::consts::PI + std::f32::consts::FRAC_PI_2 * k as f32 / 8.0;
            outline.push(c + corner * vec2(a.cos(), a.sin()));
        }
    } else {
        outline.push(rect.left_top());
    }
    outline.extend([rect.right_top(), rect.right_bottom(), rect.left_bottom()]);
    let mut mesh = egui::Mesh::default();
    let center = rect.center();
    mesh.colored_vertex(center, color_at(center.y));
    for p in &outline {
        mesh.colored_vertex(*p, color_at(p.y));
    }
    let n = outline.len() as u32;
    for k in 0..n {
        mesh.add_triangle(0, 1 + k, 1 + (k + 1) % n);
    }
    painter.add(mesh);
}

/// The area of the Earth visible in `area` (degrees), and the screen pixels per degree it would
/// need; `None` if it is too large (whole globe).
fn visible_area(view: &View, area: Rect, ppp: f32) -> Option<([f32; 4], f32)> {
    let mut lats = Vec::new();
    let mut lons = Vec::new();
    for i in 0..=12 {
        for j in 0..=12 {
            let p = pos2(
                area.left() + area.width() * i as f32 / 12.0,
                area.top() + area.height() * j as f32 / 12.0,
            );
            if let Some((lat, lon)) = view.unproject(p) {
                lats.push(lat);
                lons.push(lon);
            }
        }
    }
    if lats.len() < 4 {
        return None;
    }
    let min = |v: &[f32]| v.iter().copied().fold(f32::MAX, f32::min);
    let max = |v: &[f32]| v.iter().copied().fold(f32::MIN, f32::max);
    let (lat0, lat1, lon0, lon1) = (min(&lats), max(&lats), min(&lons), max(&lons));
    if lon1 - lon0 > 150.0 || lat1 - lat0 > 100.0 {
        return None;
    }
    let need = view.radius * ppp * std::f32::consts::PI / 180.0;
    Some(([lat0, lon0, lat1, lon1], need))
}
