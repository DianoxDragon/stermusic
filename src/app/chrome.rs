//! Window chrome: built-in title bar, sidebar, dialogs, notifications.

use eframe::egui::viewport::ResizeDirection;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, ViewportCommand,
};

use super::{Action, Dialog, Page, Playlist, PlaylistId, SterMusic, View};
use crate::anim::{self, Easing};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::trf;
use crate::{theme, widgets};

pub const TITLE_HEIGHT: f32 = 40.0;
const RESIZE_MARGIN: f32 = 5.0;
const SIDEBAR_W: f32 = 264.0;
const SIDEBAR_MIN: f32 = 60.0;

/// Window edge under the mouse (borderless window: resizing is handled here).
pub fn resize_direction(ctx: &egui::Context) -> Option<ResizeDirection> {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)) {
        return None;
    }
    let pos = ctx.input(|i| i.pointer.hover_pos())?;
    let r = ctx.screen_rect();
    let (left, right) = (
        pos.x < r.left() + RESIZE_MARGIN,
        pos.x > r.right() - RESIZE_MARGIN,
    );
    let (top, bottom) = (
        pos.y < r.top() + RESIZE_MARGIN,
        pos.y > r.bottom() - RESIZE_MARGIN,
    );
    Some(match (left, right, top, bottom) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return None,
    })
}

pub fn apply_resize(ctx: &egui::Context, direction: Option<ResizeDirection>) {
    let Some(direction) = direction else { return };
    let cursor = match direction {
        ResizeDirection::North | ResizeDirection::South => egui::CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => egui::CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => egui::CursorIcon::ResizeNwSe,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => egui::CursorIcon::ResizeNeSw,
    };
    ctx.set_cursor_icon(cursor);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}

/// Area that moves the window (drag) and maximizes it (double click).
pub fn drag_area(ui: &mut egui::Ui, rect: Rect, id: &str) {
    let response = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    if response.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if response.double_clicked() {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum WinBtn {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Window button, Breeze (KDE) style: thin glyphs in a circle that fills on hover, Breeze red
/// for close.
fn window_button(ui: &mut egui::Ui, rect: Rect, kind: WinBtn, tooltip: &str) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(("winbtn", kind as u8)), Sense::click());
    // Hover computed from the pointer position: it reacts even when the window doesn't have the
    // focus (egui may then not consider the button hovered).
    let hovered = ui
        .ctx()
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| rect.contains(p));
    let t = widgets::hover_t(ui, response.id, hovered);
    // Inactive window: dimmed glyphs, like KWin's decorations.
    let focused = ui.ctx().input(|i| i.focused);
    let idle = if focused { theme::TEXT_2 } else { theme::TEXT_3 };
    let close = kind == WinBtn::Close;
    let painter = ui.painter();
    {
        const BREEZE_RED: Color32 = Color32::from_rgb(0xDA, 0x44, 0x53);
        let c = rect.center();
        let r = 9.0;
        if t > 0.0 {
            let fill = if close { BREEZE_RED } else { theme::white(0x30) };
            painter.circle_filled(c, r, fill.gamma_multiply(t));
        }
        let pressed = response.is_pointer_button_down_on();
        if pressed {
            painter.circle_filled(c, r, Color32::from_black_alpha(40));
        }
        let color = if close {
            theme::mix(idle, Color32::WHITE, t)
        } else {
            theme::mix(idle, theme::TEXT, t)
        };
        let stroke = Stroke::new(1.5_f32, color);
        let k = 3.6;
        match kind {
            WinBtn::Minimize => painter.add(egui::Shape::line(
                vec![
                    c + vec2(-k, -k * 0.5),
                    c + vec2(0.0, k * 0.5),
                    c + vec2(k, -k * 0.5),
                ],
                stroke,
            )),
            WinBtn::Maximize => painter.add(egui::Shape::line(
                vec![
                    c + vec2(-k, k * 0.5),
                    c + vec2(0.0, -k * 0.5),
                    c + vec2(k, k * 0.5),
                ],
                stroke,
            )),
            WinBtn::Restore => painter.add(egui::Shape::closed_line(
                vec![
                    c + vec2(0.0, -k),
                    c + vec2(k, 0.0),
                    c + vec2(0.0, k),
                    c + vec2(-k, 0.0),
                ],
                stroke,
            )),
            _ => {
                painter.line_segment([c + vec2(-k, -k), c + vec2(k, k)], stroke);
                painter.line_segment([c + vec2(k, -k), c + vec2(-k, k)], stroke)
            }
        };
    }
    response.on_hover_text(tooltip)
}

impl SterMusic {
    pub(super) fn title_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("title")
            .exact_height(TITLE_HEIGHT)
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                drag_area(ui, rect, "titlebar");

                let mut x = rect.left() + 8.0;
                let can_back = !self.history.is_empty() || !matches!(self.view, View::Page(_));
                let t = anim::toggle(ui.ctx(), egui::Id::new("back_visible"), can_back, anim::NORMAL);
                if t > 0.01 {
                    let r = Rect::from_center_size(pos2(x + 16.0, rect.center().y), vec2(32.0, 32.0));
                    let resp = ui.put(r, |ui: &mut egui::Ui| {
                        ui.multiply_opacity(t);
                        widgets::icon_button(ui, Icon::ArrowLeft, tr("Retour (Échap)"), 32.0, 16.0)
                    });
                    if resp.clicked() && can_back {
                        self.actions.push(Action::Back);
                    }
                    x += 40.0 * t;
                }

                // Logo: music note on the brand gradient.
                let logo = Rect::from_center_size(pos2(x + 12.0, rect.center().y), vec2(20.0, 20.0));
                theme::brand_fill(ui.painter(), logo, 5, 1.0);
                icons::paint_centered(ui.painter(), Icon::Music, logo.center(), 13.0, Color32::WHITE);
                ui.painter().text(
                    pos2(logo.right() + 10.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    "SterMusic",
                    FontId::proportional(12.5),
                    theme::TEXT_2,
                );

                // Round, tightly packed window buttons, like Breeze's.
                let bw = 30.0;
                let right = rect.right() - 8.0;
                let close = Rect::from_min_size(pos2(right - bw, rect.top()), vec2(bw, TITLE_HEIGHT));
                let max = close.translate(vec2(-bw, 0.0));
                let min = max.translate(vec2(-bw, 0.0));
                if window_button(ui, close, WinBtn::Close, tr("Fermer")).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                let (kind, tip) = if maximized {
                    (WinBtn::Restore, tr("Restaurer"))
                } else {
                    (WinBtn::Maximize, tr("Agrandir"))
                };
                if window_button(ui, max, kind, tip).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                }
                if window_button(ui, min, WinBtn::Minimize, tr("Réduire")).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                }
                let mini = Rect::from_min_size(min.min - vec2(40.0, 0.0), vec2(36.0, TITLE_HEIGHT));
                {
                    let r = Rect::from_center_size(mini.center(), vec2(32.0, 32.0));
                    if ui
                        .put(r, |ui: &mut egui::Ui| {
                            widgets::icon_button(
                                ui,
                                Icon::MiniPlayer,
                                tr("Mini lecteur (Ctrl+M)"),
                                32.0,
                                16.0,
                            )
                        })
                        .clicked()
                    {
                        self.actions.push(Action::ToggleMini);
                    }
                }

                // Window title centered, like windows decorated by KWin.
                {
                    let caption = self.window_title.trim_end_matches(" · SterMusic").to_owned();
                    let caption = if caption == "SterMusic" {
                        String::new()
                    } else {
                        caption
                    };
                    let caption_color = if ctx.input(|i| i.focused) {
                        theme::TEXT_2
                    } else {
                        theme::TEXT_3
                    };
                    if !caption.is_empty() {
                        let max_w = (rect.width() - 2.0 * 220.0).max(80.0);
                        let g =
                            widgets::galley(ui, &caption, FontId::proportional(13.0), caption_color, max_w);
                        ui.painter().galley(
                            pos2(
                                rect.center().x - g.size().x / 2.0,
                                rect.center().y - g.size().y / 2.0,
                            ),
                            g,
                            theme::TEXT_2,
                        );
                    }
                }
            });
    }

    pub(super) fn sidebar(&mut self, ctx: &egui::Context) {
        let collapsed = self.settings.sidebar_collapsed;
        let w = anim::value(
            ctx,
            egui::Id::new("sidebar_w"),
            if collapsed { SIDEBAR_MIN } else { SIDEBAR_W },
            anim::NORMAL,
            Easing::OutExpo,
        );
        egui::SidePanel::left("sidebar")
            .exact_width(w)
            .resizable(false)
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let expanded = w > SIDEBAR_W * 0.75;
                let mut y = rect.top() + 4.0;
                let left = rect.left() + 6.0;
                let inner_w = w - 12.0;

                // "Hamburger" menu.
                let r = Rect::from_min_size(pos2(left, y), vec2(48.0, 36.0));
                if ui
                    .put(r, |ui: &mut egui::Ui| {
                        widgets::icon_button(ui, Icon::Menu, "", 40.0, 18.0)
                    })
                    .clicked()
                {
                    self.actions.push(Action::ToggleSidebar);
                }
                y += 42.0;

                // Search.
                if expanded {
                    let r = Rect::from_min_size(pos2(left + 6.0, y), vec2(inner_w - 12.0, 34.0));
                    self.search_box(ui, r);
                } else {
                    let r = Rect::from_min_size(pos2(left, y), vec2(48.0, 34.0));
                    if ui
                        .put(r, |ui: &mut egui::Ui| {
                            widgets::icon_button(ui, Icon::Search, tr("Rechercher (Ctrl+F)"), 40.0, 17.0)
                        })
                        .clicked()
                    {
                        self.actions.push(Action::FocusSearch);
                    }
                }
                y += 46.0;

                // Main navigation.
                let queue_badge = self.queue.len();
                let items = [
                    (Page::Home, Icon::Home, tr("Accueil")),
                    (Page::Library, Icon::Music, tr("Bibliothèque musicale")),
                    (Page::Queue, Icon::ListMusic, tr("File d'attente")),
                    (Page::Playlists, Icon::Library, tr("Playlists")),
                    (Page::Radio, Icon::Radio, tr("Webradios")),
                    (Page::Podcasts, Icon::Podcast, tr("Podcasts")),
                    (Page::Stats, Icon::Chart, tr("Statistiques")),
                ];
                let active_page = match &self.view {
                    View::Page(p) => Some(*p),
                    View::Album(_) | View::Artist(_) | View::Browse(..) => Some(Page::Library),
                    View::Playlist(_) => Some(Page::Playlists),
                    View::Recap(_) => Some(Page::Stats),
                    View::Podcast(_) => Some(Page::Podcasts),
                    _ => None,
                };
                for (page, icon, label) in items {
                    let r = Rect::from_min_size(pos2(left, y), vec2(inner_w, 40.0));
                    let badge = (page == Page::Queue && queue_badge > 0).then(|| queue_badge.to_string());
                    let selected = active_page == Some(page);
                    if nav_item(ui, r, icon, label, selected, expanded, badge.as_deref()).clicked() {
                        self.actions.push(Action::Go(View::Page(page)));
                    }
                    y += 42.0;
                }

                // Playlists, for quick access.
                let bottom_reserved = 94.0;
                if expanded {
                    y += 6.0;
                    ui.painter().hline(
                        left + 12.0..=left + inner_w - 12.0,
                        y,
                        Stroke::new(1.0_f32, theme::stroke()),
                    );
                    y += 10.0;
                    let current_pl = if let View::Playlist(id) = self.view {
                        Some(id)
                    } else {
                        None
                    };
                    let pinned = self.pinned();
                    ui.painter().text(
                        pos2(left + 12.0, y + 6.0),
                        Align2::LEFT_CENTER,
                        tr("Playlists épinglées"),
                        FontId::proportional(11.5),
                        theme::TEXT_3,
                    );
                    y += 18.0;
                    if pinned.is_empty() {
                        let hint = Rect::from_min_max(
                            pos2(left + 12.0, y),
                            pos2(left + inner_w - 12.0, rect.bottom() - bottom_reserved),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(hint), |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(tr(
                                        "Épingle ici jusqu'à six playlists : clic droit sur une playlist → Épingler.",
                                    ))
                                    .small()
                                    .color(theme::TEXT_3),
                                )
                                .wrap(),
                            );
                        });
                    }
                    // Pinned playlists (six at most), as a row of tiles: their name shows up on hover.
                    let row_w = inner_w - 24.0;
                    let gap = 6.0;
                    let side = ((row_w - gap * 5.0) / 6.0).min(40.0);
                    for (k, id) in pinned.into_iter().take(super::MAX_PINS).enumerate() {
                        let cell = Rect::from_min_size(
                            pos2(left + 12.0 + k as f32 * (side + gap), y + 2.0),
                            vec2(side, side),
                        );
                        let resp = self.pin_tile(ui, cell, id, current_pl == Some(id));
                        if resp.clicked() {
                            self.actions.push(Action::Go(View::Playlist(id)));
                        }
                        resp.context_menu(|ui| {
                            let tracks = self.playlist_tracks(id);
                            self.playlist_menu(ui, id, &tracks);
                        });
                    }
                }

                // Tools (tag editor included), right above the settings, with the number of files whose
                // tags are waiting to be saved.
                let r = Rect::from_min_size(pos2(left, rect.bottom() - 90.0), vec2(inner_w, 40.0));
                let selected = matches!(active_page, Some(Page::Tags | Page::Tools));
                let pending = self.tag_editor.dirty_count();
                let badge = (pending > 0).then(|| pending.to_string());
                let busy = self.tag_editor.busy() || self.lib_tools.busy() || self.tools.busy();
                let nav = nav_item(
                    ui,
                    r,
                    Icon::Wrench,
                    tr("Outils"),
                    selected,
                    expanded,
                    badge.as_deref().filter(|_| !busy),
                );
                // Playback or recording in progress: visible from any page.
                if busy {
                    let center = if expanded {
                        pos2(r.right() - 22.0, r.center().y)
                    } else {
                        pos2(r.left() + 36.0, r.top() + 11.0)
                    };
                    let side = if expanded { 16.0 } else { 11.0 };
                    egui::Spinner::new()
                        .color(theme::accent_light())
                        .paint_at(ui, Rect::from_center_size(center, vec2(side, side)));
                }
                let tip = if self.tag_editor.saving() {
                    tr("Enregistrement des tags en cours…")
                } else if self.tag_editor.loading() {
                    tr("Lecture des tags en cours…")
                } else {
                    self.lib_tools.status().unwrap_or_default()
                };
                let nav = if busy { nav.on_hover_text(tip) } else { nav };
                if nav.clicked() {
                    self.actions.push(Action::Go(View::Page(Page::Tools)));
                }

                // Settings, at the bottom.
                let r = Rect::from_min_size(pos2(left, rect.bottom() - 48.0), vec2(inner_w, 40.0));
                let selected = active_page == Some(Page::Settings);
                if nav_item(ui, r, Icon::Settings, tr("Paramètres"), selected, expanded, None).clicked() {
                    self.actions.push(Action::Go(View::Page(Page::Settings)));
                }

            });
    }

    fn search_box(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let bg = ui.painter().add(egui::Shape::Noop);
        let id = egui::Id::new("search_box");
        let response = ui.put(
            rect,
            egui::TextEdit::singleline(&mut self.search)
                .id(id)
                .hint_text(tr("Rechercher"))
                .frame(false)
                .margin(vec2(34.0, 8.0))
                .vertical_align(egui::Align::Center),
        );
        if std::mem::take(&mut self.focus_search) {
            response.request_focus();
        }
        let focused = response.has_focus();
        let t = anim::toggle(ui.ctx(), id.with("focus"), focused, anim::FAST);
        ui.painter().set(
            bg,
            egui::epaint::RectShape::new(
                rect,
                CornerRadius::same(6),
                theme::mix(theme::white(0x0F), theme::white(0x06), t),
                Stroke::new(1.0_f32, theme::stroke()),
                StrokeKind::Inside,
            ),
        );
        // Accent underline when the search has the focus (Fluent).
        if t > 0.0 {
            let line = Rect::from_min_max(
                pos2(rect.left() + 1.0, rect.bottom() - 2.0),
                pos2(rect.right() - 1.0, rect.bottom()),
            );
            theme::brand_fill(ui.painter(), line, 1, t);
        }
        icons::paint_centered(
            ui.painter(),
            Icon::Search,
            pos2(rect.left() + 17.0, rect.center().y),
            15.0,
            theme::TEXT_3,
        );

        if response.changed() {
            self.refresh_search();
            if self.search.trim().is_empty() {
                if self.view == View::Search {
                    self.actions.push(Action::Back);
                }
            } else if self.view != View::Search {
                self.actions.push(Action::Go(View::Search));
            }
        }
        if focused && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.search.clear();
            if self.view == View::Search {
                self.actions.push(Action::Back);
            }
            response.surrender_focus();
        }
    }

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let mut close = false;
        let mut confirm = false;
        let title = match dialog {
            Dialog::ThemeName { id: None, .. } => tr("Enregistrer le thème"),
            Dialog::ThemeName { .. } => tr("Renommer le thème"),
            Dialog::DeleteTheme(_) => tr("Supprimer le thème ?"),
            Dialog::ClearStats => tr("Effacer les statistiques ?"),
            Dialog::NewPlaylist { .. } => tr("Nouvelle playlist"),
            Dialog::Rename { .. } => tr("Renommer la playlist"),
            Dialog::ConfirmDelete(_) => tr("Supprimer la playlist ?"),
        };
        let modal = crate::widgets::modal(ctx, egui::Id::new("dialog"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .inner_margin(24.0)
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(egui::RichText::new(title).font(theme::semibold(20.0)));
                ui.add_space(12.0);
                match dialog {
                    Dialog::ThemeName { id: None, .. } => {
                        ui.label(
                            egui::RichText::new(
                                tr("Ses couleurs sont gardées telles quelles, même si celles du bureau changent."),
                            )
                            .color(theme::TEXT_2),
                        );
                        ui.add_space(10.0);
                    }
                    Dialog::DeleteTheme(id) => {
                        let name = self
                            .settings
                            .saved_themes
                            .iter()
                            .find(|t| &t.id == id)
                            .map(|t| t.name.as_str())
                            .unwrap_or("");
                        ui.label(
                            egui::RichText::new(trf!("Le thème « {name} » sera retiré de tes favoris.", name))
                                .color(theme::TEXT_2),
                        );
                    }
                    _ => {}
                }
                let hint = if matches!(dialog, Dialog::ThemeName { .. }) {
                    tr("Nom du thème")
                } else {
                    tr("Nom de la playlist")
                };
                match dialog {
                    Dialog::NewPlaylist { name, .. }
                    | Dialog::Rename { name, .. }
                    | Dialog::ThemeName { name, .. } => {
                        let r = ui.add(
                            egui::TextEdit::singleline(name)
                                .desired_width(f32::INFINITY)
                                .margin(vec2(10.0, 8.0))
                                .hint_text(hint),
                        );
                        if !r.has_focus() && ui.memory(|m| m.focused().is_none()) {
                            r.request_focus();
                        }
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            confirm = true;
                        }
                    }
                    Dialog::DeleteTheme(_) => {}
                    Dialog::ClearStats => {
                        ui.label(
                            egui::RichText::new(tr(
                                "Les compteurs d'écoutes et tout l'historique daté seront supprimés définitivement. \
                                 Les playlists « Les plus écoutés » et le récap repartiront de zéro.",
                            ))
                            .color(theme::TEXT_2),
                        );
                    }
                    Dialog::ConfirmDelete(i) => {
                        let name = self
                            .settings
                            .playlists
                            .get(*i)
                            .map(|p| p.name.as_str())
                            .unwrap_or("");
                        ui.label(
                            egui::RichText::new(trf!("« {name} » sera supprimée. Les morceaux restent dans la bibliothèque.", name))
                            .color(theme::TEXT_2),
                        );
                    }
                }
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = match dialog {
                            Dialog::ThemeName { id: None, .. } => tr("Enregistrer"),
                            Dialog::ThemeName { .. } => tr("Renommer"),
                            Dialog::DeleteTheme(_) => tr("Supprimer"),
                            Dialog::ClearStats => tr("Effacer"),
                            Dialog::NewPlaylist { .. } => tr("Créer"),
                            Dialog::Rename { .. } => tr("Renommer"),
                            Dialog::ConfirmDelete(_) => tr("Supprimer"),
                        };
                        if widgets::primary_button(ui, None, label).clicked() {
                            confirm = true;
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
        if confirm {
            match self.dialog.take() {
                Some(Dialog::ThemeName { id, name }) => {
                    let name = match name.trim() {
                        "" => theme::suggested_name(),
                        n => n.to_owned(),
                    };
                    match id {
                        None => {
                            let saved = theme::SavedTheme::from_current(name.clone());
                            let id = saved.id.clone();
                            self.settings.saved_themes.push(saved);
                            theme::set_saved(&self.settings.saved_themes);
                            self.settings.theme = id.clone();
                            theme::set(ctx, &id);
                            self.toast(trf!("Thème « {name} » ajouté à tes favoris", name));
                        }
                        Some(id) => {
                            if let Some(t) = self.settings.saved_themes.iter_mut().find(|t| t.id == id) {
                                t.name = name;
                            }
                            theme::set_saved(&self.settings.saved_themes);
                            theme::set(ctx, &self.settings.theme);
                        }
                    }
                }
                Some(Dialog::ClearStats) => self.clear_stats(),
                Some(Dialog::DeleteTheme(id)) => {
                    self.settings.saved_themes.retain(|t| t.id != id);
                    theme::set_saved(&self.settings.saved_themes);
                    // The deleted theme was the current one: go back to the desktop's, otherwise to the first one.
                    if self.settings.theme == id {
                        self.settings.theme = if theme::system_theme().is_some() {
                            theme::KDE_ID.to_owned()
                        } else {
                            theme::THEMES[0].id.to_owned()
                        };
                    }
                    theme::set(ctx, &self.settings.theme);
                }
                Some(Dialog::NewPlaylist { name, tracks }) => {
                    let name = if name.trim().is_empty() {
                        tr("Sans titre").to_owned()
                    } else {
                        name.trim().to_owned()
                    };
                    let paths = self.paths(&tracks);
                    self.settings.playlists.push(Playlist {
                        pinned: false,
                        name: name.clone(),
                        tracks: paths,
                    });
                    self.toast(trf!("Playlist « {name} » créée", name));
                }
                Some(Dialog::Rename { index, name }) => {
                    if let Some(p) = self.settings.playlists.get_mut(index) {
                        if !name.trim().is_empty() {
                            p.name = name.trim().to_owned();
                        }
                    }
                }
                Some(Dialog::ConfirmDelete(i)) if i < self.settings.playlists.len() => {
                    self.settings.playlists.remove(i);
                    if matches!(self.view, View::Playlist(PlaylistId::User(_))) {
                        self.view = View::Page(Page::Playlists);
                        self.history.clear();
                    }
                }
                _ => {}
            }
        } else if close {
            self.dialog = None;
        }
    }

    pub(super) fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((text, start)) = &mut self.toast else {
            return;
        };
        let now = ctx.input(|i| i.time);
        if *start < 0.0 {
            *start = now;
        }
        let age = (now - *start) as f32;
        const LIFE: f32 = 3.5;
        if age > LIFE {
            self.toast = None;
            return;
        }
        // Arrival: slides in from the right with a slight bounce; departure: fades out moving down.
        let enter = anim::Easing::OutBack.apply((age / 0.35).min(1.0));
        let leave = ((LIFE - age) / 0.4).clamp(0.0, 1.0);
        let fade = (age / 0.2).min(1.0).min(leave).clamp(0.0, 1.0);
        let offset = vec2(28.0 * (1.0 - enter), 8.0 * (1.0 - leave));
        let pop = anim::Easing::OutBack.apply(((age - 0.08) / 0.3).clamp(0.0, 1.0));
        let left = 1.0 - age / LIFE;
        let screen = ctx.screen_rect();
        let text = text.clone();
        egui::Area::new(egui::Id::new("toast"))
            .order(egui::Order::Foreground)
            .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -120.0) + offset)
            .interactable(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(fade);
                let frame = egui::Frame::popup(ui.style())
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_max_width((screen.width() * 0.5).min(420.0));
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                            icons::paint_centered(
                                ui.painter(),
                                Icon::Info,
                                r.center(),
                                16.0 * pop,
                                theme::accent_light(),
                            );
                            ui.label(egui::RichText::new(text).color(theme::TEXT));
                        });
                    });
                // Thin line at the bottom, across the full width: the time left before the notification
                // goes away. The rounded shape of the notification is painted, clipped to a band at the
                // bottom: the line thus follows its corners.
                let r = frame.response.rect;
                let strip = Rect::from_min_max(pos2(r.left(), r.bottom() - 3.0), pos2(r.left() + r.width() * left, r.bottom()));
                let painter = ui.painter().with_clip_rect(strip);
                theme::brand_fill(&painter, r, 10, 0.85);
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
}

/// Navigation item: icon, label, background on hover and when selected.
fn nav_item(
    ui: &mut egui::Ui,
    rect: Rect,
    icon: Icon,
    label: &str,
    selected: bool,
    expanded: bool,
    badge: Option<&str>,
) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(("nav", label)), Sense::click());
    let s = anim::toggle(ui.ctx(), response.id.with("sel"), selected, anim::FAST);
    let painter = ui.painter();
    if s > 0.0 {
        {
            // Breeze selection: background tinted with the accent and an accent border.
            painter.rect_filled(
                rect,
                CornerRadius::same(5),
                theme::alpha(theme::accent(), (0x40 as f32 * s) as u8),
            );
            painter.rect_stroke(
                rect,
                CornerRadius::same(5),
                Stroke::new(
                    1.0_f32,
                    theme::alpha(theme::accent_light(), (0x90 as f32 * s) as u8),
                ),
                StrokeKind::Inside,
            );
        }
    }
    widgets::paint_interaction(ui, rect, 6, &response);
    let color = theme::mix(
        theme::TEXT_2,
        theme::TEXT,
        s.max(widgets::hover_t(ui, response.id, response.hovered())),
    );
    icons::paint_centered(
        ui.painter(),
        icon,
        pos2(rect.left() + 24.0, rect.center().y),
        18.0,
        color,
    );
    if expanded {
        ui.painter().text(
            pos2(rect.left() + 52.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(14.0),
            color,
        );
        if let Some(b) = badge {
            let galley =
                ui.fonts(|f| f.layout_no_wrap(b.to_owned(), FontId::proportional(11.0), theme::TEXT));
            let w = (galley.size().x + 12.0).max(22.0);
            let appear = anim::appear_each(ui.ctx(), response.id.with("badge_in"), anim::SLOW, Easing::OutBack);
            let bump = anim::pulse(ui.ctx(), response.id.with("badge_bump"), egui::Id::new(b).value(), anim::SLOW * 1.5);
            let scale = appear.max(0.0) * (1.0 + 0.22 * bump);
            let r = Rect::from_center_size(
                pos2(rect.right() - 12.0 - w / 2.0, rect.center().y),
                vec2(w, 18.0) * scale,
            );
            ui.painter().rect_filled(
                r,
                CornerRadius::same((9.0 * scale) as u8),
                theme::mix(theme::accent_soft(), theme::accent(), bump),
            );
            ui.painter()
                .galley(r.center() - galley.size() / 2.0, galley, theme::TEXT);
        }
    }
    if expanded {
        response
    } else {
        response.on_hover_text(label)
    }
}

impl SterMusic {
    /// Tile of a pinned playlist: mosaic of its album art (heart for favorites, icon for a smart
    /// playlist). It grows in when it appears, lifts on hover and gets an accent ring when the
    /// playlist is open.
    fn pin_tile(&mut self, ui: &mut egui::Ui, cell: Rect, id: PlaylistId, selected: bool) -> egui::Response {
        let key = egui::Id::new(("pin_tile", format!("{id:?}")));
        let resp = ui.interact(cell, key, Sense::click());
        let appear = anim::value_from(ui.ctx(), key.with("in"), 0.0, 1.0, anim::SLOW * 1.4, Easing::OutBack);
        let h = widgets::hover_t(ui, resp.id, resp.hovered());
        let s = anim::toggle(ui.ctx(), key.with("sel"), selected, anim::NORMAL);
        let pop = widgets::pop(ui.ctx(), key, selected);
        let press = anim::toggle(ui.ctx(), key.with("press"), resp.is_pointer_button_down_on(), anim::FAST);
        let scale = appear.max(0.0) * pop * (1.0 + 0.06 * h - 0.06 * press);
        let tile = Rect::from_center_size(cell.center() - vec2(0.0, 2.0 * h), cell.size() * scale);
        let mut scoped = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
        scoped.multiply_opacity(appear.clamp(0.0, 1.0));
        let ui = &mut scoped;
        let radius = 8;
        ui.painter().add(
            egui::Shadow {
                offset: [0, (2.0 + 3.0 * h) as i8],
                blur: (6.0 + 8.0 * h) as u8,
                spread: 0,
                color: Color32::from_black_alpha((60.0 + 50.0 * h) as u8),
            }
            .as_shape(tile, CornerRadius::same(radius)),
        );
        match id {
            PlaylistId::Smart(i) => {
                let icon = self.settings.smart_playlists[i].icon.icon();
                let (c1, c2) = theme::seeded_colors(&self.settings.smart_playlists[i].name);
                ui.painter()
                    .rect_filled(tile, CornerRadius::same(radius), theme::mix(c1, theme::accent(), 0.25));
                // Glow towards the top right, kept inside: it fades out before the edges (centered on the
                // corner, it filled the rounded part).
                let w = tile.width();
                widgets::soft_glow(
                    ui.painter(),
                    tile,
                    tile.center() + vec2(w * 0.15, -w * 0.15),
                    w * 0.36,
                    theme::alpha(c2, 0x80),
                );
                icons::paint_centered(ui.painter(), icon, tile.center(), tile.width() * 0.46, Color32::WHITE);
            }
            PlaylistId::Favorites => self.mosaic(ui, tile, &[], true, radius),
            PlaylistId::User(i) => {
                // The first tracks are enough for the mosaic (four albums at most).
                let tracks: Vec<usize> = self
                    .settings
                    .playlists
                    .get(i)
                    .map(|pl| pl.tracks.iter().take(80).filter_map(|p| self.lib.find(p)).collect())
                    .unwrap_or_default();
                self.mosaic(ui, tile, &tracks, false, radius);
            }
        }
        ui.painter().rect_stroke(
            tile,
            CornerRadius::same(radius),
            Stroke::new(1.0_f32, theme::white(0x18)),
            StrokeKind::Inside,
        );
        if s > 0.0 {
            ui.painter().rect_stroke(
                tile.expand(3.0),
                CornerRadius::same(radius + 3),
                Stroke::new(2.0_f32, theme::accent_light().gamma_multiply(s)),
                StrokeKind::Inside,
            );
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(self.playlist_name(id))
    }
}
