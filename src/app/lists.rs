//! Listes de morceaux, grilles d'albums et d'artistes, menus contextuels.

use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, Ui};

use super::{Action, Ctx, PlaylistId, View};
use crate::icons::{self, Icon};
use crate::library::{fmt_duration, plural};
use crate::{anim, theme, widgets};

pub const ROW_H: f32 = 48.0;

#[derive(Default, Clone, Copy)]
pub struct ListOpts {
    /// Pochette dans la marge (sinon le numéro).
    pub cover: bool,
    pub album: bool,
    /// Numéro de piste (vue d'un album) plutôt que la position dans la liste.
    pub track_no: bool,
    pub playlist: Option<PlaylistId>,
    /// Liste de la file d'attente : la ligne en cours se repère par sa position.
    pub queue_pos: Option<Option<usize>>,
    /// Numéro de position avant la pochette (file d'attente).
    pub numbered: bool,
}

struct Cols {
    gutter: f32,
    title: (f32, f32),
    artist: (f32, f32),
    album: Option<(f32, f32)>,
    year: Option<(f32, f32)>,
    duration: (f32, f32),
    heart: (f32, f32),
}

fn columns(rect: Rect, opts: &ListOpts) -> Cols {
    let gutter = if opts.cover { 60.0 } else { 48.0 } + if opts.numbered { 44.0 } else { 0.0 };
    let heart_w = 40.0;
    let dur_w = 64.0;
    let year_w = if opts.album && rect.width() > 920.0 {
        64.0
    } else {
        0.0
    };
    let rest = rect.width() - gutter - heart_w - dur_w - year_w - 8.0;
    let mut x = rect.left() + gutter;
    let mut col = |w: f32| {
        let c = (x, w - 16.0);
        x += w;
        c
    };
    let (title, artist, album) = if opts.album {
        let t = col(rest * 0.42);
        let a = col(rest * 0.26);
        (t, a, Some(col(rest * 0.32)))
    } else {
        let t = col(rest * 0.6);
        (t, col(rest * 0.4), None)
    };
    let year = (year_w > 0.0).then(|| col(year_w));
    let heart = col(heart_w);
    let duration = col(dur_w);
    Cols {
        gutter,
        title,
        artist,
        album,
        year,
        duration,
        heart,
    }
}

/// En-tête des colonnes.
pub fn header(ui: &mut Ui, opts: &ListOpts) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let c = columns(rect, opts);
    let font = FontId::proportional(12.5);
    let y = rect.center().y;
    let p = ui.painter();
    p.text(
        pos2(rect.left() + c.gutter / 2.0, y),
        Align2::CENTER_CENTER,
        "#",
        font.clone(),
        theme::TEXT_3,
    );
    p.text(
        pos2(c.title.0, y),
        Align2::LEFT_CENTER,
        "Titre",
        font.clone(),
        theme::TEXT_3,
    );
    p.text(
        pos2(c.artist.0, y),
        Align2::LEFT_CENTER,
        "Artiste",
        font.clone(),
        theme::TEXT_3,
    );
    if let Some(a) = c.album {
        p.text(
            pos2(a.0, y),
            Align2::LEFT_CENTER,
            "Album",
            font.clone(),
            theme::TEXT_3,
        );
    }
    if let Some(a) = c.year {
        p.text(
            pos2(a.0, y),
            Align2::LEFT_CENTER,
            "Année",
            font.clone(),
            theme::TEXT_3,
        );
    }
    icons::paint_centered(p, Icon::Clock, pos2(c.duration.0 + 12.0, y), 14.0, theme::TEXT_3);
    p.hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0_f32, theme::stroke()),
    );
}

pub struct RowOut {
    pub rect: Rect,
    /// Un glisser-déposer commence sur cette ligne (file d'attente).
    pub drag_started: bool,
}

/// Une ligne de morceau. `list` est la liste affichée : on la lit à partir de la ligne `i`.
/// Les premières lignes apparaissent en cascade à l'ouverture de la page.
pub fn track_row(ui: &mut Ui, c: &mut Ctx, list: &[usize], i: usize, opts: &ListOpts) -> RowOut {
    let a = stagger(ui.ctx(), i);
    ui.scope(|ui| {
        ui.set_opacity(a);
        track_row_inner(ui, c, list, i, opts)
    })
    .inner
}

fn track_row_inner(ui: &mut Ui, c: &mut Ctx, list: &[usize], i: usize, opts: &ListOpts) -> RowOut {
    let idx = list[i];
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return RowOut {
            rect,
            drag_started: false,
        };
    }
    let row = rect.shrink2(vec2(0.0, 2.0));
    // File d'attente : la ligne entière se glisse pour réorganiser.
    let sense = if opts.queue_pos.is_some() {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let resp = ui.interact(row, ui.id().with(("row", i, idx)), sense);
    let is_current = match opts.queue_pos {
        Some(pos) => pos == Some(i),
        None => c.current == Some(idx),
    };
    let selected = *c.selected == Some(idx) && opts.queue_pos.is_none();
    let cols = columns(rect, opts);
    let t = &c.lib.tracks[idx];
    let fav = c.settings.favorites.contains(&t.path);

    let painter = ui.painter();
    if i % 2 == 1 {
        painter.rect_filled(row, CornerRadius::same(6), theme::white(0x04));
    }
    if selected || is_current {
        painter.rect_filled(
            row,
            CornerRadius::same(6),
            if is_current {
                theme::alpha(theme::accent(), 0x1E)
            } else {
                theme::white(0x0E)
            },
        );
    }
    widgets::paint_interaction(ui, row, 6, &resp);
    let hovered = resp.hovered();

    // Marge : numéro de position (file d'attente), pochette / numéro, et bouton lecture au survol.
    let gutter = Rect::from_min_size(row.min, vec2(cols.gutter, row.height()));
    let gutter = if opts.numbered {
        let slot = Rect::from_min_size(gutter.min, vec2(44.0, gutter.height()));
        if is_current {
            widgets::equalizer(
                ui.painter(),
                slot.center(),
                14.0,
                theme::accent_light(),
                c.playing,
            );
        } else {
            ui.painter().text(
                slot.center(),
                Align2::CENTER_CENTER,
                (i + 1).to_string(),
                FontId::proportional(13.0),
                theme::TEXT_3,
            );
        }
        Rect::from_min_max(pos2(slot.right(), gutter.top()), gutter.max)
    } else {
        gutter
    };
    let gutter_resp = ui.interact(gutter, ui.id().with(("gutter", i, idx)), Sense::click());
    let show_play = hovered || gutter_resp.hovered();
    if opts.cover {
        let cover = Rect::from_center_size(gutter.center(), vec2(38.0, 38.0));
        let tex = c.covers.thumb(&t.path).cloned();
        widgets::cover(ui.painter(), cover, tex.as_ref(), 4, &t.album);
        if show_play || (is_current && !opts.numbered) {
            ui.painter()
                .rect_filled(cover, CornerRadius::same(4), Color32::from_black_alpha(130));
        }
    }
    let center = gutter.center();
    if is_current && !show_play && !opts.numbered {
        widgets::equalizer(ui.painter(), center, 15.0, theme::accent_light(), c.playing);
    } else if show_play {
        let color = if gutter_resp.hovered() {
            Color32::WHITE
        } else {
            theme::TEXT
        };
        // Le morceau en cours propose Pause (ou Lecture s'il est en pause), pas « relancer ».
        let icon = if is_current && c.playing {
            Icon::Pause
        } else {
            Icon::Play
        };
        icons::paint_centered(ui.painter(), icon, center, 15.0, color);
    } else if !opts.cover {
        let n = if opts.track_no {
            t.track_no.map(|n| n.to_string()).unwrap_or_default()
        } else {
            (i + 1).to_string()
        };
        ui.painter().text(
            center,
            Align2::CENTER_CENTER,
            n,
            FontId::proportional(13.0),
            theme::TEXT_3,
        );
    }

    // Colonnes de texte.
    let y = row.center().y;
    let title_color = if is_current {
        theme::accent_light()
    } else {
        theme::TEXT
    };
    let other = if is_current {
        theme::mix(theme::accent_light(), theme::TEXT_2, 0.5)
    } else {
        theme::TEXT_2
    };
    let text = |ui: &Ui, s: &str, col: (f32, f32), font: FontId, color: Color32| {
        let g = widgets::galley(ui, s, font, color, col.1);
        ui.painter().galley(pos2(col.0, y - g.size().y / 2.0), g, color);
    };
    text(ui, &t.title, cols.title, FontId::proportional(14.0), title_color);
    text(
        ui,
        t.artist_or_unknown(),
        cols.artist,
        FontId::proportional(13.5),
        other,
    );
    if let Some(a) = cols.album {
        text(ui, t.album_or_unknown(), a, FontId::proportional(13.5), other);
    }
    if let Some(a) = cols.year {
        text(
            ui,
            &t.year.map(|y| y.to_string()).unwrap_or_default(),
            a,
            FontId::proportional(13.5),
            other,
        );
    }
    text(
        ui,
        &fmt_duration(t.duration()),
        cols.duration,
        FontId::proportional(13.5),
        other,
    );

    // Favori, ou poignée de déplacement dans la file d'attente.
    let heart = Rect::from_center_size(pos2(cols.heart.0 + 12.0, y), vec2(30.0, 30.0));
    let mut drag_started = resp.drag_started();
    if opts.queue_pos.is_some() {
        let h = ui.interact(heart, ui.id().with(("grip", i)), Sense::drag());
        if hovered || h.hovered() || h.dragged() {
            icons::paint_centered(
                ui.painter(),
                Icon::Grip,
                heart.center(),
                16.0,
                if h.hovered() { theme::TEXT } else { theme::TEXT_3 },
            );
        }
        drag_started |= h.drag_started();
        h.on_hover_cursor(egui::CursorIcon::Grab)
            .on_hover_text("Glisser pour déplacer");
    } else {
        let h = ui.interact(heart, ui.id().with(("heart", i, idx)), Sense::click());
        let pop = widgets::pop(ui.ctx(), h.id, fav);
        if fav {
            icons::paint_centered(
                ui.painter(),
                Icon::HeartFilled,
                heart.center(),
                15.0 * pop,
                theme::accent_light(),
            );
        } else if hovered || h.hovered() {
            icons::paint_centered(
                ui.painter(),
                Icon::Heart,
                heart.center(),
                15.0,
                if h.hovered() { theme::TEXT } else { theme::TEXT_3 },
            );
        }
        if h.on_hover_text(if fav {
            "Retirer des favoris"
        } else {
            "Ajouter aux favoris"
        })
        .clicked()
        {
            c.actions.push(Action::ToggleFavorite(idx));
        }
    }

    let play = || match opts.queue_pos {
        // Le morceau en cours : pause / reprise plutôt que de le relancer depuis le début.
        _ if is_current && c.has_track => Action::TogglePlay,
        Some(_) => Action::QueueJump(i),
        None => Action::Play {
            tracks: list.to_vec(),
            start: i,
        },
    };
    if gutter_resp.clicked() || resp.double_clicked() {
        c.actions.push(play());
    } else if resp.clicked() {
        *c.selected = Some(idx);
    }
    resp.context_menu(|ui| track_menu(ui, c, list, i, opts));
    gutter_resp.context_menu(|ui| track_menu(ui, c, list, i, opts));
    RowOut { rect, drag_started }
}

fn track_menu(ui: &mut Ui, c: &mut Ctx, list: &[usize], i: usize, opts: &ListOpts) {
    let idx = list[i];
    let t = &c.lib.tracks[idx];
    if widgets::menu_item(ui, Some(Icon::PlayOutline), "Lire").clicked() {
        c.actions.push(match opts.queue_pos {
            Some(_) => Action::QueueJump(i),
            None => Action::Play {
                tracks: list.to_vec(),
                start: i,
            },
        });
    }
    if opts.queue_pos.is_none() {
        if widgets::menu_item(ui, Some(Icon::ListStart), "Lire ensuite").clicked() {
            c.actions.push(Action::PlayNext(vec![idx]));
        }
        if widgets::menu_item(ui, Some(Icon::ListEnd), "Ajouter à la file d'attente").clicked() {
            c.actions.push(Action::Enqueue(vec![idx]));
        }
    }
    add_to_menu(ui, c, vec![idx]);
    ui.separator();
    let fav = c.settings.favorites.contains(&t.path);
    if widgets::menu_item(
        ui,
        Some(if fav { Icon::HeartFilled } else { Icon::Heart }),
        if fav {
            "Retirer des favoris"
        } else {
            "Ajouter aux favoris"
        },
    )
    .clicked()
    {
        c.actions.push(Action::ToggleFavorite(idx));
    }
    let album = c.lib.track_album[idx];
    if widgets::menu_item(ui, Some(Icon::Disc), "Afficher l'album").clicked() {
        c.go(View::Album(album));
    }
    if widgets::menu_item(ui, Some(Icon::Mic), "Afficher l'artiste").clicked() {
        c.go(View::Artist(c.lib.album_artist[album]));
    }
    if widgets::menu_item(ui, Some(Icon::FolderOpen), "Ouvrir le dossier").clicked() {
        if let Some(dir) = t.path.parent() {
            c.actions.push(Action::OpenFolder(dir.to_path_buf()));
        }
    }
    if let Some(pl) = opts.playlist {
        ui.separator();
        if widgets::menu_item(ui, Some(Icon::Trash), "Retirer de la playlist").clicked() {
            c.actions.push(Action::RemoveFromPlaylist(pl, i));
        }
    }
    if opts.queue_pos.is_some() {
        ui.separator();
        if widgets::menu_item(ui, Some(Icon::Close), "Retirer de la file d'attente").clicked() {
            c.actions.push(Action::QueueRemove(i));
        }
    }
}

/// Sous-menu « Ajouter à » : nouvelle playlist, file d'attente, playlists existantes.
pub fn add_to_menu(ui: &mut Ui, c: &mut Ctx, tracks: Vec<usize>) {
    ui.menu_button("       Ajouter à une playlist", |ui| {
        if widgets::menu_item(ui, Some(Icon::Plus), "Nouvelle playlist…").clicked() {
            c.actions.push(Action::NewPlaylist(tracks.clone()));
        }
        if !c.settings.playlists.is_empty() {
            ui.separator();
        }
        for (k, pl) in c.settings.playlists.iter().enumerate() {
            if widgets::menu_item(ui, Some(Icon::ListMusic), &pl.name).clicked() {
                c.actions.push(Action::AddToPlaylist(k, tracks.clone()));
            }
        }
    });
}

/// Liste virtuelle (seules les lignes visibles sont dessinées), avec son propre défilement.
pub fn track_list(ui: &mut Ui, c: &mut Ctx, id: &str, list: &[usize], opts: &ListOpts) {
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink(false)
        .show_rows(ui, ROW_H, list.len(), |ui, range| {
            for i in range {
                track_row(ui, c, list, i, opts);
            }
            ui.add_space(16.0);
        });
}

/// Liste complète, à placer dans une zone qui défile déjà (album, artiste, recherche).
pub fn track_list_inline(ui: &mut Ui, c: &mut Ctx, list: &[usize], opts: &ListOpts) {
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    for i in 0..list.len() {
        track_row(ui, c, list, i, opts);
    }
    ui.spacing_mut().item_spacing.y = spacing;
}

// ---------- Grilles ----------

pub const CARD_MIN: f32 = 160.0;
pub const GAP: f32 = 20.0;

/// Nombre de colonnes et largeur des cartes pour remplir `width`.
pub fn grid_layout(width: f32) -> (usize, f32) {
    let cols = ((width + GAP) / (CARD_MIN + GAP)).floor().max(1.0) as usize;
    let w = ((width - GAP * (cols - 1) as f32) / cols as f32).min(240.0);
    (cols, w)
}

pub fn card_height(w: f32) -> f32 {
    w + 58.0
}

/// Grille d'éléments (dans une zone qui défile déjà), limitée à `max_rows` lignes.
pub fn grid(
    ui: &mut Ui,
    salt: &str,
    n: usize,
    max_rows: Option<usize>,
    mut card: impl FnMut(&mut Ui, usize, f32),
) {
    let (cols, w) = grid_layout(ui.available_width());
    let rows = n.div_ceil(cols).min(max_rows.unwrap_or(usize::MAX));
    // Un même album peut apparaître dans deux sections : identifiants distincts par section.
    ui.push_id(salt, |ui| {
        for r in 0..rows {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for k in r * cols..((r + 1) * cols).min(n) {
                    let a = stagger(ui.ctx(), k);
                    ui.scope(|ui| {
                        ui.set_opacity(a);
                        card(ui, k, w);
                    });
                }
            });
            ui.add_space(GAP * 0.6);
        }
    });
}

/// Grille virtuelle, avec son propre défilement.
pub fn virtual_grid(ui: &mut Ui, id: &str, n: usize, mut card: impl FnMut(&mut Ui, usize, f32)) {
    let (cols, w) = grid_layout(ui.available_width() - 8.0);
    let rows = n.div_ceil(cols);
    ui.spacing_mut().item_spacing.y = GAP * 0.6;
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink(false)
        .show_rows(ui, card_height(w), rows, |ui, range| {
            for r in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for k in r * cols..((r + 1) * cols).min(n) {
                        let a = stagger(ui.ctx(), k);
                        ui.scope(|ui| {
                            ui.set_opacity(a);
                            card(ui, k, w);
                        });
                    }
                });
            }
            ui.add_space(16.0);
        });
}

/// Carte d'album : pochette qui se soulève au survol, bouton lecture, titre et artiste.
pub fn album_card(ui: &mut Ui, c: &mut Ctx, a: usize, w: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, card_height(w)), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let album = &c.lib.albums[a];
    let resp = ui.interact(rect, ui.id().with(("album", a)), Sense::click());
    // Survol de toute la carte, bouton lecture compris : sinon, survoler le bouton « quitte » la
    // carte, qui redescend, ce qui déplace le bouton… et la carte tremblait.
    let h = widgets::hover_t(ui, resp.id, ui.rect_contains_pointer(rect));
    let cover = Rect::from_min_size(rect.min, vec2(w, w)).translate(vec2(0.0, -3.0 * h));
    let tex = c.covers.thumb(&c.lib.tracks[album.tracks[0]].path).cloned();
    let shadow = egui::Shadow {
        offset: [0, (4.0 + 6.0 * h) as i8],
        blur: (10.0 + 14.0 * h) as u8,
        spread: 0,
        color: Color32::from_black_alpha((70.0 + 50.0 * h) as u8),
    };
    ui.painter().add(shadow.as_shape(cover, CornerRadius::same(8)));
    widgets::cover(ui.painter(), cover, tex.as_ref(), 8, &album.title);
    // Album en cours de lecture : pastille avec l'égaliseur animé.
    if c.current.is_some_and(|t| c.lib.track_album[t] == a) {
        let center = pos2(cover.right() - 22.0, cover.top() + 22.0);
        ui.painter()
            .circle_filled(center, 15.0, Color32::from_black_alpha(150));
        widgets::equalizer(ui.painter(), center, 13.0, theme::accent_light(), c.playing);
    }

    // Bouton lecture, en bas à gauche de la pochette.
    let mut play_clicked = false;
    if h > 0.01 {
        let center = pos2(cover.left() + 30.0, cover.bottom() - 30.0);
        let r = Rect::from_center_size(center, vec2(40.0, 40.0));
        let pr = ui.interact(r, ui.id().with(("album_play", a)), Sense::click());
        let ph = widgets::hover_t(ui, pr.id, pr.hovered());
        let size = 36.0 + 4.0 * ph;
        let pr_rect = Rect::from_center_size(center, vec2(size, size));
        ui.painter().circle_filled(
            center + vec2(0.0, 2.0),
            size / 2.0 + 1.0,
            Color32::from_black_alpha((90.0 * h) as u8),
        );
        theme::brand_fill(ui.painter(), pr_rect, CornerRadius::same((size / 2.0) as u8), h);
        icons::paint_centered(
            ui.painter(),
            Icon::Play,
            center + vec2(1.5, 0.0),
            15.0,
            Color32::WHITE.gamma_multiply(h),
        );
        if pr.on_hover_text("Lire l'album").clicked() {
            play_clicked = true;
            c.actions.push(Action::Play {
                tracks: album.tracks.clone(),
                start: 0,
            });
        }
    }

    let title = widgets::galley(ui, &album.title, theme::semibold(14.0), theme::TEXT, w);
    ui.painter().galley(
        pos2(rect.left(), cover.bottom() + 10.0 + 3.0 * h),
        title,
        theme::TEXT,
    );
    let sub = match album.year {
        Some(y) => format!("{} · {y}", album.artist),
        None => album.artist.clone(),
    };
    let artist = widgets::galley(ui, &sub, FontId::proportional(13.0), theme::TEXT_2, w);
    ui.painter().galley(
        pos2(rect.left(), cover.bottom() + 31.0 + 3.0 * h),
        artist,
        theme::TEXT_2,
    );

    if resp.clicked() && !play_clicked {
        c.go(View::Album(a));
    }
    resp.context_menu(|ui| album_menu(ui, c, a));
}

pub fn album_menu(ui: &mut Ui, c: &mut Ctx, a: usize) {
    let tracks = c.lib.albums[a].tracks.clone();
    if widgets::menu_item(ui, Some(Icon::PlayOutline), "Lire").clicked() {
        c.actions.push(Action::Play {
            tracks: tracks.clone(),
            start: 0,
        });
    }
    if widgets::menu_item(ui, Some(Icon::Shuffle), "Lecture aléatoire").clicked() {
        c.actions.push(Action::PlayShuffled(tracks.clone()));
    }
    if widgets::menu_item(ui, Some(Icon::ListStart), "Lire ensuite").clicked() {
        c.actions.push(Action::PlayNext(tracks.clone()));
    }
    if widgets::menu_item(ui, Some(Icon::ListEnd), "Ajouter à la file d'attente").clicked() {
        c.actions.push(Action::Enqueue(tracks.clone()));
    }
    add_to_menu(ui, c, tracks.clone());
    ui.separator();
    if widgets::menu_item(ui, Some(Icon::Mic), "Afficher l'artiste").clicked() {
        c.go(View::Artist(c.lib.album_artist[a]));
    }
    if widgets::menu_item(ui, Some(Icon::FolderOpen), "Ouvrir le dossier").clicked() {
        if let Some(dir) = c.lib.tracks[tracks[0]].path.parent() {
            c.actions.push(Action::OpenFolder(dir.to_path_buf()));
        }
    }
}

/// Carte d'artiste : avatar rond (pochette de son premier album), nom, nombre d'albums.
pub fn artist_card(ui: &mut Ui, c: &mut Ctx, ar: usize, w: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, card_height(w)), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let artist = &c.lib.artists[ar];
    let resp = ui.interact(rect, ui.id().with(("artist", ar)), Sense::click());
    let h = widgets::hover_t(ui, resp.id, resp.hovered());
    let radius = w * 0.44 + 3.0 * h;
    let center = pos2(rect.center().x, rect.top() + w / 2.0);
    let first = artist.albums.first().map(|&a| c.lib.albums[a].tracks[0]);
    let tex = first.and_then(|t| c.covers.thumb(&c.lib.tracks[t].path).cloned());
    ui.painter().circle_filled(
        center + vec2(0.0, 4.0),
        radius + 2.0,
        Color32::from_black_alpha((50.0 + 40.0 * h) as u8),
    );
    widgets::circle_cover(ui, center, radius, tex.as_ref(), &artist.name);
    if h > 0.0 {
        ui.painter().circle_stroke(
            center,
            radius + 3.0,
            egui::Stroke::new(2.0_f32, theme::accent_light().gamma_multiply(h)),
        );
    }
    let name = widgets::galley(ui, &artist.name, theme::semibold(14.0), theme::TEXT, w);
    ui.painter().galley(
        pos2(rect.center().x - name.size().x / 2.0, rect.top() + w + 10.0),
        name,
        theme::TEXT,
    );
    let sub = plural(artist.albums.len(), "album", "albums");
    let g = widgets::galley(ui, &sub, FontId::proportional(13.0), theme::TEXT_2, w);
    ui.painter().galley(
        pos2(rect.center().x - g.size().x / 2.0, rect.top() + w + 31.0),
        g,
        theme::TEXT_2,
    );
    if resp.clicked() {
        c.go(View::Artist(ar));
    }
    let tracks = artist.tracks.clone();
    resp.context_menu(|ui| {
        if widgets::menu_item(ui, Some(Icon::PlayOutline), "Lire").clicked() {
            c.actions.push(Action::Play {
                tracks: tracks.clone(),
                start: 0,
            });
        }
        if widgets::menu_item(ui, Some(Icon::Shuffle), "Lecture aléatoire").clicked() {
            c.actions.push(Action::PlayShuffled(tracks.clone()));
        }
        if widgets::menu_item(ui, Some(Icon::ListEnd), "Ajouter à la file d'attente").clicked() {
            c.actions.push(Action::Enqueue(tracks.clone()));
        }
        add_to_menu(ui, c, tracks.clone());
    });
}

/// Titre de section avec un lien « Tout afficher » optionnel ; renvoie vrai si on a cliqué le lien.
pub fn section(ui: &mut Ui, title: &str, link: Option<&str>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .font(theme::semibold(20.0))
                .color(theme::TEXT),
        );
        if let Some(l) = link {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clicked = widgets::link(ui, l).clicked();
            });
        }
    });
    ui.add_space(8.0);
    clicked
}

fn page_ids(key: &str) -> (egui::Id, egui::Id) {
    let id = egui::Id::new(("page_fade", key));
    (id, id.with("stagger"))
}

/// Apparition d'une page : renvoie sa progression (0 → 1) et prépare la cascade des éléments.
pub fn page_progress(ctx: &egui::Context, key: &str) -> f32 {
    let (fade, cascade) = page_ids(key);
    let t = anim::value_from(ctx, fade, 0.0, 1.0, anim::SLOW, anim::Easing::OutExpo);
    let s = anim::value_from(ctx, cascade, 0.0, 1.0, 0.75, anim::Easing::Out);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("page_stagger"), s));
    t
}

/// Rejoue l'apparition d'une page (navigation).
pub fn reset_page(ctx: &egui::Context, key: &str) {
    let (fade, cascade) = page_ids(key);
    anim::reset(ctx, fade);
    anim::reset(ctx, cascade);
}

/// Opacité du `i`-ème élément d'une page qui apparaît (les premiers d'abord).
pub fn stagger(ctx: &egui::Context, i: usize) -> f32 {
    let s = ctx
        .data(|d| d.get_temp::<f32>(egui::Id::new("page_stagger")))
        .unwrap_or(1.0);
    ((s - (i.min(15) as f32) * 0.04) / 0.4).clamp(0.0, 1.0)
}
