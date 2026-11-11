//! Éléments d'interface réutilisables et animés, dans l'esprit de SterCode et de Fluent.

use std::sync::Arc;

use eframe::egui::{
    self, pos2, vec2, Color32, CornerRadius, FontId, Galley, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
    TextureHandle, Ui,
};

use crate::anim;
use crate::icons::{self, Icon};
use crate::theme;

/// Progression (0 → 1) d'une animation de survol.
pub fn hover_t(ui: &Ui, id: egui::Id, hovered: bool) -> f32 {
    anim::toggle(ui.ctx(), id.with("hover"), hovered, anim::FAST)
}

/// Fond d'un élément interactif : survol en fondu, léger assombrissement à l'appui.
pub fn paint_interaction(ui: &Ui, rect: Rect, radius: u8, response: &Response) {
    let t = hover_t(ui, response.id, response.hovered());
    if t > 0.0 {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(radius),
            theme::hover().gamma_multiply(t * 1.6),
        );
    }
    if response.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius), theme::white(0x08));
    }
}

/// Texte sur une ligne, coupé par « … » s'il dépasse `max_width`.
pub fn galley(ui: &Ui, text: &str, font: FontId, color: Color32, max_width: f32) -> Arc<Galley> {
    let mut job =
        egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping {
        max_width: max_width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.fonts(|f| f.layout_job(job))
}

/// Bouton-icône carré.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str, size: f32, icon_size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), Sense::click());
    if ui.is_rect_visible(rect) {
        paint_interaction(ui, rect, 6, &response);
        let t = hover_t(ui, response.id, response.hovered());
        let color = if ui.is_enabled() {
            theme::mix(theme::icon_idle(), theme::TEXT, t)
        } else {
            theme::TEXT_3
        };
        // Petit enfoncement au clic.
        let press = anim::toggle(
            ui.ctx(),
            response.id.with("press"),
            response.is_pointer_button_down_on(),
            anim::FAST,
        );
        icons::paint_centered(
            ui.painter(),
            icon,
            rect.center(),
            icon_size * (1.0 - 0.1 * press),
            color,
        );
    }
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// Bouton-icône à bascule : icône dans l'accent et petit point dessous quand il est actif
/// (aléatoire, répétition), comme dans le lecteur de Windows 11.
pub fn toggle_icon(ui: &mut Ui, on: bool, icon: Icon, tooltip: &str, size: f32, icon_size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), Sense::click());
    paint_interaction(ui, rect, 6, &response);
    let a = anim::toggle(ui.ctx(), response.id.with("on"), on, anim::NORMAL);
    let t = hover_t(ui, response.id, response.hovered());
    let color = theme::mix(
        theme::mix(theme::icon_idle(), theme::TEXT, t),
        theme::accent_light(),
        a,
    );
    // Petit rebond quand on l'active (favori, aléatoire, répétition).
    let pop = pop(ui.ctx(), response.id, on);
    icons::paint_centered(
        ui.painter(),
        icon,
        rect.center() - vec2(0.0, 2.0 * a),
        icon_size * pop,
        color,
    );
    if a > 0.0 {
        let dot = pos2(rect.center().x, rect.center().y + icon_size / 2.0 + 3.0);
        ui.painter()
            .circle_filled(dot, 1.8 * a, theme::accent_light().gamma_multiply(a));
    }
    // Sans texte, pas de bulle (sinon une bulle vide apparaît au survol).
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// Gros bouton rond Lecture / Pause, dans le dégradé de marque.
pub fn play_button(ui: &mut Ui, playing: bool, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), Sense::click());
    let t = hover_t(ui, response.id, response.hovered());
    let press = anim::toggle(
        ui.ctx(),
        response.id.with("press"),
        response.is_pointer_button_down_on(),
        anim::FAST,
    );
    let scale = 1.0 + 0.05 * t - 0.09 * press;
    let r = Rect::from_center_size(rect.center(), rect.size() * scale);
    let painter = ui.painter();
    let radius = CornerRadius::same((r.width() / 2.0) as u8);
    let shadow = egui::Shadow {
        offset: [0, 3],
        blur: (8.0 + 10.0 * t) as u8,
        spread: 0,
        color: theme::accent().gamma_multiply(0.30 + 0.30 * t),
    };
    painter.add(shadow.as_shape(r, radius));
    theme::brand_fill(painter, r, radius, 1.0);
    // Lecture ↔ pause : les deux icônes se croisent en tournant légèrement et en changeant d'échelle.
    let p = anim::value(
        ui.ctx(),
        response.id.with("morph"),
        if playing { 1.0 } else { 0.0 },
        anim::NORMAL,
        anim::Easing::OutBack,
    );
    let p = p.clamp(0.0, 1.0);
    let icon_size = r.width() * 0.42;
    if p < 1.0 {
        // Le triangle « lecture » paraît décentré : on le pousse un peu à droite.
        let s = icon_size * (1.0 - 0.5 * p);
        icons::paint_centered(
            painter,
            Icon::Play,
            r.center() + vec2(size * 0.04, 0.0),
            s,
            Color32::WHITE.gamma_multiply(1.0 - p),
        );
    }
    if p > 0.0 {
        let s = icon_size * (0.5 + 0.5 * p);
        icons::paint_centered(
            painter,
            Icon::Pause,
            r.center(),
            s,
            Color32::WHITE.gamma_multiply(p),
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(crate::i18n::tr(if playing {
            crate::i18n::n_("Pause (Espace)")
        } else {
            crate::i18n::n_("Lecture (Espace)")
        }))
}

fn button_content(ui: &Ui, icon: Option<Icon>, text: &str, color: Color32) -> (Arc<Galley>, f32) {
    let galley = ui.fonts(|f| f.layout_no_wrap(text.to_owned(), theme::semibold(14.0), color));
    let width = galley.size().x + if icon.is_some() { 26.0 } else { 0.0 } + 32.0;
    (galley, width)
}

fn paint_button_content(ui: &Ui, rect: Rect, icon: Option<Icon>, galley: Arc<Galley>, color: Color32) {
    let content_w = galley.size().x + if icon.is_some() { 26.0 } else { 0.0 };
    let mut x = rect.center().x - content_w / 2.0;
    if let Some(icon) = icon {
        icons::paint_centered(ui.painter(), icon, pos2(x + 8.0, rect.center().y), 16.0, color);
        x += 26.0;
    }
    ui.painter()
        .galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
}

/// Bouton principal, dans le dégradé de marque.
pub fn primary_button(ui: &mut Ui, icon: Option<Icon>, text: &str) -> Response {
    let (galley, w) = button_content(ui, icon, text, Color32::WHITE);
    let (rect, response) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
    let t = hover_t(ui, response.id, response.hovered());
    // Enfoncement au clic, puis petit rebond au relâchement.
    let down = response.is_pointer_button_down_on();
    let press = anim::value(
        ui.ctx(),
        response.id.with("press"),
        if down { 1.0 } else { 0.0 },
        anim::NORMAL,
        anim::Easing::OutBack,
    );
    let rect = Rect::from_center_size(rect.center(), rect.size() * (1.0 - 0.05 * press));
    let painter = ui.painter();
    let shadow = egui::Shadow {
        offset: [0, 4],
        blur: (6.0 + 10.0 * t) as u8,
        spread: 0,
        color: theme::accent().gamma_multiply(0.22 + 0.25 * t),
    };
    painter.add(shadow.as_shape(rect, CornerRadius::same(6)));
    theme::brand_fill(painter, rect, 6, 1.0);
    let overlay = if down {
        Color32::from_black_alpha(0x26)
    } else {
        theme::white((0x1A as f32 * t) as u8)
    };
    painter.rect_filled(rect, CornerRadius::same(6), overlay);
    paint_button_content(ui, rect, icon, galley, Color32::WHITE);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Petit rebond (0,3 → 1, avec dépassement) quand `on` devient vrai : cœur des favoris…
pub fn pop(ctx: &egui::Context, id: egui::Id, on: bool) -> f32 {
    let pop_id = id.with("pop");
    let was = ctx.data(|d| d.get_temp::<bool>(pop_id.with("was")));
    let fresh = on && was == Some(false);
    if fresh {
        anim::reset(ctx, pop_id);
    }
    ctx.data_mut(|d| d.insert_temp(pop_id.with("was"), on));
    anim::value_from(
        ctx,
        pop_id,
        if fresh { 0.3 } else { 1.0 },
        1.0,
        anim::NORMAL,
        anim::Easing::OutBack,
    )
}

/// Égaliseur animé (trois barres qui dansent) : le morceau en cours. À l'arrêt, les barres se posent.
pub fn equalizer(painter: &egui::Painter, center: Pos2, size: f32, color: Color32, playing: bool) {
    let ctx = painter.ctx();
    let time = ctx.input(|i| i.time) as f32;
    // Transition douce entre lecture et pause.
    let id = egui::Id::new(("equalizer", (center.x as i32, center.y as i32)));
    let k = anim::toggle(ctx, id, playing, anim::NORMAL);
    let bar_w = (size * 0.2).max(2.0);
    let gap = size * 0.13;
    let bottom = center.y + size / 2.0;
    for (i, (speed, phase)) in [(7.3_f32, 0.0_f32), (9.1, 1.9), (6.2, 3.4)]
        .into_iter()
        .enumerate()
    {
        let wave =
            ((time * speed + phase).sin() * 0.5 + 0.5) * ((time * speed * 0.37 + phase).cos() * 0.25 + 0.75);
        let h = size * (0.28 + k * 0.72 * wave.clamp(0.0, 1.0)).max(0.22);
        let x = center.x + (i as f32 - 1.0) * (bar_w + gap);
        let bar = Rect::from_min_max(pos2(x - bar_w / 2.0, bottom - h), pos2(x + bar_w / 2.0, bottom));
        painter.rect_filled(bar, CornerRadius::same((bar_w / 2.0) as u8), color);
    }
    if playing {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// Champ de recherche (fond arrondi, loupe, soulignement à l'accent quand il a le focus).
pub fn search_field(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, width: f32) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
    let bg = ui.painter().add(egui::Shape::Noop);
    let response = ui.put(
        rect,
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(hint)
            .frame(false)
            .margin(vec2(32.0, 7.0))
            .vertical_align(egui::Align::Center),
    );
    let t = anim::toggle(ui.ctx(), id.with("focus"), response.has_focus(), anim::FAST);
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
        pos2(rect.left() + 16.0, rect.center().y),
        15.0,
        theme::TEXT_3,
    );
    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        text.clear();
        response.surrender_focus();
    }
    response
}

/// Bouton secondaire « verre ».
pub fn secondary_button(ui: &mut Ui, icon: Option<Icon>, text: &str) -> Response {
    let (galley, w) = button_content(ui, icon, text, theme::TEXT);
    let (rect, response) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
    let t = hover_t(ui, response.id, response.hovered());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(6),
        theme::white(0x10 + (0x0A as f32 * t) as u8),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0_f32, theme::mix(theme::stroke(), theme::stroke_strong(), t)),
        StrokeKind::Inside,
    );
    if response.is_pointer_button_down_on() {
        painter.rect_filled(rect, CornerRadius::same(6), Color32::from_black_alpha(0x18));
    }
    paint_button_content(ui, rect, icon, galley, theme::TEXT);
    response
}

/// Interrupteur animé.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let a = anim::value(
        ui.ctx(),
        response.id.with("knob"),
        if *on { 1.0 } else { 0.0 },
        0.18,
        anim::Easing::Out,
    );
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), theme::white(0x14));
    painter.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0_f32, theme::white(0x40).gamma_multiply(1.0 - a)),
        StrokeKind::Inside,
    );
    if a > 0.0 {
        theme::brand_fill(painter, rect, 10, a);
    }
    let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, a);
    let r = if response.hovered() { 7.0 } else { 6.0 };
    painter.circle_filled(
        pos2(x, rect.center().y),
        r,
        theme::mix(theme::TEXT_2, Color32::WHITE, a),
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub struct Seek {
    /// Valeur affichée pendant que l'on glisse (0 → 1).
    pub preview: Option<f32>,
    /// Valeur choisie (clic ou fin du glissement).
    pub commit: Option<f32>,
    /// Zone occupée par la barre.
    pub rect: Rect,
}

/// Barre de progression / de volume, fine, qui s'épaissit au survol, avec son curseur rond.
pub fn seek_bar(ui: &mut Ui, id_salt: &str, value: f32, width: f32, enabled: bool) -> Seek {
    seek_bar_marked(ui, id_salt, value, width, enabled, None)
}

/// Comme [`seek_bar`], avec un repère à `mark` (0 → 1) : la partie remplie au-delà du repère
/// passe à l'orange (volume amplifié, au-dessus de 100 %).
pub fn seek_bar_marked(
    ui: &mut Ui,
    id_salt: &str,
    value: f32,
    width: f32,
    enabled: bool,
    mark: Option<f32>,
) -> Seek {
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 20.0), sense);
    let id = ui.id().with(id_salt);
    let ratio_at = |x: f32| ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);

    let mut preview = None;
    let mut commit = None;
    if enabled {
        if let Some(p) = response.interact_pointer_pos() {
            if response.dragged() || response.is_pointer_button_down_on() {
                preview = Some(ratio_at(p.x));
                ui.data_mut(|d| d.insert_temp(id, ratio_at(p.x)));
            }
        }
        if response.clicked() || response.drag_stopped() {
            commit = response
                .interact_pointer_pos()
                .map(|p| ratio_at(p.x))
                .or_else(|| ui.data(|d| d.get_temp::<f32>(id)));
        }
    }

    let shown = preview.unwrap_or(value).clamp(0.0, 1.0);
    let active = enabled && (response.hovered() || response.dragged());
    let t = anim::toggle(ui.ctx(), id.with("active"), active, anim::FAST);
    let painter = ui.painter();
    let h = 4.0 + 2.0 * t;
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), h));
    painter.rect_filled(track, CornerRadius::same(3), theme::white(0x2A));
    if shown > 0.0 && enabled {
        let fill = Rect::from_min_max(
            track.min,
            pos2(track.left() + track.width() * shown, track.bottom()),
        );
        if fill.width() >= 4.0 {
            theme::brand_fill(painter, fill, CornerRadius::same(3), 1.0);
        } else {
            painter.rect_filled(fill, CornerRadius::same(3), theme::accent());
        }
    }
    let over = mark.is_some_and(|m| shown > m + 0.001);
    if let Some(m) = mark {
        let x = track.left() + track.width() * m;
        if over && enabled {
            let hot = Rect::from_min_max(
                pos2(x, track.top()),
                pos2(track.left() + track.width() * shown, track.bottom()),
            );
            painter.rect_filled(hot, CornerRadius::same(3), theme::BOOST);
        }
        // Le repère : un petit trait qui dépasse de la barre.
        let tick = Rect::from_center_size(pos2(x, track.center().y), vec2(2.0, h + 6.0));
        painter.rect_filled(tick, CornerRadius::same(1), theme::white(if over { 0xD0 } else { 0x70 }));
    }
    if enabled {
        let c = pos2(track.left() + track.width() * shown, track.center().y);
        painter.circle_filled(c + vec2(0.0, 1.0), 8.0, Color32::from_black_alpha(60));
        painter.circle_filled(c, 7.5, theme::glass());
        painter.circle_stroke(c, 7.5, Stroke::new(1.0_f32, theme::white(0x22)));
        painter.circle_filled(c, 3.5 + 1.5 * t, if over { theme::BOOST } else { theme::accent_light() });
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    Seek { preview, commit, rect }
}

/// Pochette dans un rectangle arrondi ; sans image, un dégradé avec une note de musique.
pub fn cover(painter: &egui::Painter, rect: Rect, texture: Option<&TextureHandle>, radius: u8, seed: &str) {
    match texture {
        Some(tex) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.add(
                egui::epaint::RectShape::filled(rect, CornerRadius::same(radius), Color32::WHITE)
                    .with_texture(tex.id(), uv),
            );
        }
        None => {
            let (a, b) = theme::seeded_colors(seed);
            painter.rect_filled(rect, CornerRadius::same(radius), a);
            // Lueur douce vers le haut à droite, pour donner du relief.
            let center = pos2(
                rect.right() - rect.width() * 0.25,
                rect.top() + rect.height() * 0.25,
            );
            soft_glow(painter, rect, center, rect.width() * 0.9, theme::alpha(b, 0x80));
            let size = (rect.width() * 0.34).clamp(12.0, 64.0);
            icons::paint_centered(painter, Icon::Music, rect.center(), size, theme::white(0x70));
        }
    }
    painter.rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0_f32, theme::white(0x10)),
        StrokeKind::Inside,
    );
}

/// Halo radial (`color` au centre, transparent au bord), contenu dans `bounds`.
pub fn soft_glow(painter: &egui::Painter, bounds: Rect, center: Pos2, radius: f32, color: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    let steps = 48u32;
    for i in 0..=steps {
        let a = i as f32 / steps as f32 * std::f32::consts::TAU;
        let p = center + vec2(a.cos(), a.sin()) * radius;
        // Ramené dans le rectangle : le bord, transparent, ne déborde pas des coins arrondis.
        let p = pos2(
            p.x.clamp(bounds.left(), bounds.right()),
            p.y.clamp(bounds.top(), bounds.bottom()),
        );
        mesh.colored_vertex(p, Color32::TRANSPARENT);
        if i > 0 {
            mesh.add_triangle(0, i, i + 1);
        }
    }
    painter.add(mesh);
}

/// Image ronde (artistes) ; sans image, l'initiale sur un dégradé.
pub fn circle_cover(ui: &Ui, center: Pos2, radius: f32, texture: Option<&TextureHandle>, name: &str) {
    let painter = ui.painter();
    match texture {
        Some(tex) => {
            // Rectangle aux coins entièrement arrondis : egui en lisse le bord (anti-crénelage),
            // ce qu'il ne fait pas pour un maillage en éventail.
            let rect = Rect::from_center_size(center, egui::Vec2::splat(radius * 2.0));
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            let r = radius.min(255.0) as u8;
            painter.add(
                egui::epaint::RectShape::filled(rect, CornerRadius::same(r), Color32::WHITE)
                    .with_texture(tex.id(), uv),
            );
        }
        None => {
            let (a, b) = theme::seeded_colors(name);
            painter.circle_filled(center, radius, theme::mix(a, b, 0.35));
            let initial: String = name
                .chars()
                .next()
                .map(|c| c.to_uppercase().collect())
                .unwrap_or_default();
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                initial,
                theme::semibold(radius * 0.8),
                theme::white(0xD0),
            );
        }
    }
    painter.circle_stroke(center, radius, Stroke::new(1.0_f32, theme::white(0x14)));
}

/// Élément de menu contextuel avec icône (style Fluent).
pub fn menu_item(ui: &mut Ui, icon: Option<Icon>, text: &str) -> Response {
    let width = ui.available_width().max(200.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    paint_interaction(ui, rect, 5, &response);
    if let Some(icon) = icon {
        icons::paint_centered(
            ui.painter(),
            icon,
            pos2(rect.left() + 16.0, rect.center().y),
            16.0,
            theme::TEXT_2,
        );
    }
    ui.painter().text(
        pos2(rect.left() + 36.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(14.0),
        theme::TEXT,
    );
    if response.clicked() {
        ui.close_menu();
    }
    response
}

/// Lien texte discret (« Tout afficher »).
pub fn link(ui: &mut Ui, text: &str) -> Response {
    let galley = ui.fonts(|f| f.layout_no_wrap(text.to_owned(), FontId::proportional(13.0), theme::TEXT_2));
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(18.0, 6.0), Sense::click());
    let t = hover_t(ui, response.id, response.hovered());
    let color = theme::mix(theme::TEXT_2, theme::accent_light(), t);
    ui.painter()
        .galley_with_override_text_color(rect.left_top() + vec2(0.0, 3.0), galley, color);
    icons::paint_centered(
        ui.painter(),
        Icon::ChevronRight,
        pos2(rect.right() - 7.0, rect.center().y),
        14.0,
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Grand état vide (bibliothèque vide, file d'attente vide…).
pub fn empty_state(ui: &mut Ui, icon: Icon, title: &str, subtitle: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(48.0);
        let (rect, _) = ui.allocate_exact_size(vec2(88.0, 88.0), Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 44.0, theme::accent_soft());
        icons::paint_centered(ui.painter(), icon, rect.center(), 40.0, theme::accent_light());
        ui.add_space(14.0);
        ui.label(
            egui::RichText::new(title)
                .font(theme::semibold(20.0))
                .color(theme::TEXT),
        );
        ui.add_space(2.0);
        ui.label(egui::RichText::new(subtitle).color(theme::TEXT_2));
        ui.add_space(14.0);
    });
}

/// Curseur vertical d'égaliseur (−max → +max dB) : remplissage depuis 0, curseur rond,
/// double-clic pour revenir à 0. Renvoie vrai si la valeur a changé.
pub fn vslider(ui: &mut Ui, id_salt: impl std::hash::Hash, value: &mut f32, max: f32, height: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, height), Sense::click_and_drag());
    let id = ui.id().with(id_salt);
    let track = Rect::from_center_size(rect.center(), vec2(4.0, height - 16.0));
    let y_of = |v: f32| egui::remap_clamp(v, -max..=max, track.bottom()..=track.top());
    let mut changed = false;
    if response.double_clicked() {
        *value = 0.0;
        changed = true;
    } else if let Some(p) = response
        .interact_pointer_pos()
        .filter(|_| response.dragged() || response.clicked())
    {
        let v = egui::remap_clamp(p.y, track.bottom()..=track.top(), -max..=max);
        // Crans d'un demi-décibel.
        let v = (v * 2.0).round() / 2.0;
        if (v - *value).abs() > f32::EPSILON {
            *value = v;
            changed = true;
        }
    }
    let active = response.hovered() || response.dragged();
    let t = anim::toggle(ui.ctx(), id.with("active"), active, anim::FAST);
    let painter = ui.painter();
    painter.rect_filled(track, CornerRadius::same(2), theme::white(0x22));
    // Repère du 0 dB.
    painter.hline(
        track.left() - 5.0..=track.right() + 5.0,
        y_of(0.0),
        Stroke::new(1.0_f32, theme::white(0x30)),
    );
    let (y0, y1) = (y_of(0.0), y_of(*value));
    let fill = Rect::from_min_max(pos2(track.left(), y0.min(y1)), pos2(track.right(), y0.max(y1)));
    if fill.height() > 0.5 {
        theme::brand_fill(painter, fill, CornerRadius::same(2), 1.0);
    }
    let c = pos2(track.center().x, y1);
    painter.circle_filled(c + vec2(0.0, 1.0), 8.0, Color32::from_black_alpha(60));
    painter.circle_filled(c, 7.5, theme::glass());
    painter.circle_stroke(c, 7.5, Stroke::new(1.0_f32, theme::white(0x22)));
    painter.circle_filled(c, 3.5 + 1.5 * t, theme::accent_light());
    response
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(crate::trf!("{0} dB (double-clic : 0)", format!("{:+.1}", value)));
    changed
}

/// Champ de saisie au style des autres contrôles (verre, soulignement aux couleurs du thème).
pub fn text_field(
    ui: &mut Ui,
    id: egui::Id,
    text: &mut String,
    hint: &str,
    width: f32,
    rows: usize,
) -> Response {
    const ROW: f32 = 19.0;
    const MAX_ROWS: usize = 7;
    // Plusieurs lignes : le champ grandit avec son texte (jusqu'à sept lignes), puis défile.
    let height = if rows > 1 {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let wrapped = ui.fonts(|f| {
            f.layout(text.clone(), font, Color32::WHITE, width - 20.0)
                .rows
                .len()
        });
        14.0 + ROW * wrapped.clamp(rows, MAX_ROWS) as f32
    } else {
        32.0
    };
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let bg = ui.painter().add(egui::Shape::Noop);
    fn style<'t>(edit: egui::TextEdit<'t>, id: egui::Id, hint: &str) -> egui::TextEdit<'t> {
        edit.id(id)
            .hint_text(egui::RichText::new(hint).color(theme::TEXT_3).italics())
            .frame(false)
            .margin(vec2(10.0, 7.0))
    }
    let response = if rows > 1 {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        egui::ScrollArea::vertical()
            .id_salt(id.with("scroll"))
            .max_height(height)
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.add_sized(
                    vec2(width, height),
                    style(
                        egui::TextEdit::multiline(text)
                            .desired_rows(rows)
                            .desired_width(width - 12.0),
                        id,
                        hint,
                    ),
                )
            })
            .inner
    } else {
        ui.put(
            rect,
            style(
                egui::TextEdit::singleline(text).vertical_align(egui::Align::Center),
                id,
                hint,
            ),
        )
    };
    let t = anim::toggle(ui.ctx(), id.with("focus"), response.has_focus(), anim::FAST);
    let h = hover_t(ui, id.with("hover"), response.hovered());
    ui.painter().set(
        bg,
        egui::epaint::RectShape::new(
            rect,
            CornerRadius::same(6),
            theme::mix(
                theme::white(0x0D + (0x05 as f32 * h) as u8),
                theme::white(0x06),
                t,
            ),
            Stroke::new(1.0_f32, theme::stroke()),
            StrokeKind::Inside,
        ),
    );
    if t > 0.0 {
        let line = Rect::from_min_max(
            pos2(rect.left() + 1.0, rect.bottom() - 2.0),
            pos2(rect.right() - 1.0, rect.bottom()),
        );
        theme::brand_fill(ui.painter(), line, 1, t);
    }
    response
}

/// Place un contrôle dans `rect` sans déplacer le curseur de mise en page du parent (ce que fait
/// `Ui::put`) : pour les lignes de liste dessinées à la main, dont la hauteur est déjà réservée.
pub fn place<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
    );
    add(&mut child)
}
