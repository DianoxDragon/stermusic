//! Track lists, album and artist grids, context menus.

use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, Ui};

use super::{Action, Ctx, PlaylistId, View};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::{fmt_duration, plural};
use crate::trf;
use crate::{anim, theme, widgets};

pub const ROW_H: f32 = 48.0;

#[derive(Default, Clone, Copy)]
pub struct ListOpts {
    /// Album art in the margin (otherwise the number).
    pub cover: bool,
    pub album: bool,
    /// Track number (album view) rather than the position in the list.
    pub track_no: bool,
    pub playlist: Option<PlaylistId>,
    /// Queue list: the current row is identified by its position.
    pub queue_pos: Option<Option<usize>>,
    /// Position number before the album art (queue).
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

/// Column a track list can be sorted by.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum SortKey {
    Title,
    Artist,
    Album,
    Year,
    Duration,
}

/// Sorting a list by its columns: up to three criteria, from strongest to weakest, each
/// ascending or descending (`true`). Empty: the original order of the list.
#[derive(Clone, Default, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct SortSpec(pub Vec<(SortKey, bool)>);

impl SortSpec {
    /// Click on a column header. Alone: it becomes the criterion (ascending, then descending, then
    /// back to the original order). With Shift: it is added to the existing criteria, or changes
    /// direction.
    pub fn click(&mut self, key: SortKey, add: bool) {
        let pos = self.0.iter().position(|(k, _)| *k == key);
        if add {
            match pos {
                Some(i) => self.0[i].1 = !self.0[i].1,
                None if self.0.len() < 3 => self.0.push((key, false)),
                None => {}
            }
            return;
        }
        match (pos, self.0.len()) {
            (Some(0), 1) if !self.0[0].1 => self.0[0].1 = true,
            (Some(0), 1) => self.0.clear(),
            _ => self.0 = vec![(key, false)],
        }
    }

    /// Sorts `list` (stable sort: with equal criteria, the original order is kept).
    pub fn sort(&self, lib: &crate::library::Library, list: &mut [usize]) {
        if self.0.is_empty() {
            return;
        }
        // Keys computed once per track.
        let text = |s: &str| crate::library::fold(s);
        let mut keyed: Vec<(usize, Vec<(String, u64)>)> = list
            .iter()
            .map(|&i| {
                let t = &lib.tracks[i];
                let keys = self
                    .0
                    .iter()
                    .map(|(k, _)| match k {
                        SortKey::Title => (text(&t.title), 0),
                        SortKey::Artist => (text(t.artist_or_unknown()), 0),
                        SortKey::Album => (text(t.album_or_unknown()), t.track_no.unwrap_or(0) as u64),
                        SortKey::Year => (String::new(), t.year.unwrap_or(0) as u64),
                        SortKey::Duration => (String::new(), t.duration_ms),
                    })
                    .collect();
                (i, keys)
            })
            .collect();
        keyed.sort_by(|a, b| {
            for (n, (_, desc)) in self.0.iter().enumerate() {
                let o = a.1[n].cmp(&b.1[n]);
                if o != std::cmp::Ordering::Equal {
                    return if *desc { o.reverse() } else { o };
                }
            }
            std::cmp::Ordering::Equal
        });
        for (slot, (i, _)) in list.iter_mut().zip(keyed) {
            *slot = i;
        }
    }
}

/// Column header.
pub fn header(ui: &mut Ui, opts: &ListOpts) {
    header_sortable(ui, opts, None);
}

/// Column header; with `sort`, the headers can be clicked to sort (see [`SortSpec::click`])
/// and "#" restores the original order. Returns true if the sorting changed.
pub fn header_sortable(ui: &mut Ui, opts: &ListOpts, mut sort: Option<&mut SortSpec>) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let c = columns(rect, opts);
    let font = FontId::proportional(12.5);
    let y = rect.center().y;
    let mut changed = false;
    // "#": the original order.
    let sorted = sort.as_ref().is_some_and(|s| !s.0.is_empty());
    let num = Rect::from_center_size(pos2(rect.left() + c.gutter / 2.0, y), vec2(28.0, 24.0));
    let mut num_color = theme::TEXT_3;
    if sorted {
        let resp = ui
            .interact(num, ui.id().with("col_reset"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr("Revenir à l'ordre d'origine"));
        if resp.hovered() {
            num_color = theme::TEXT;
        }
        if resp.clicked() {
            if let Some(s) = sort.as_deref_mut() {
                s.0.clear();
                changed = true;
            }
        }
    }
    ui.painter().text(num.center(), Align2::CENTER_CENTER, "#", font.clone(), num_color);

    let mut cols: Vec<(SortKey, f32, f32, Option<&str>)> = vec![
        (SortKey::Title, c.title.0, c.title.1, Some(tr("Titre"))),
        (SortKey::Artist, c.artist.0, c.artist.1, Some(tr("Artiste"))),
    ];
    if let Some(a) = c.album {
        cols.push((SortKey::Album, a.0, a.1, Some(tr("Album"))));
    }
    if let Some(a) = c.year {
        cols.push((SortKey::Year, a.0, a.1, Some(tr("Année"))));
    }
    cols.push((SortKey::Duration, c.duration.0, c.duration.1, None));
    for (key, x, w, label) in cols {
        // Width of the title (or the clock), plus room for the arrow.
        let text_w = match label {
            Some(l) => ui.fonts(|f| f.layout_no_wrap(l.to_owned(), font.clone(), theme::TEXT_3).size().x),
            None => 24.0,
        };
        let state = sort.as_ref().and_then(|s| {
            s.0.iter().position(|(k, _)| *k == key).map(|i| (i, s.0[i].1, s.0.len()))
        });
        let mut color = if state.is_some() { theme::accent_light() } else { theme::TEXT_3 };
        if sort.is_some() {
            let zone = Rect::from_min_size(pos2(x - 6.0, rect.top() + 3.0), vec2((text_w + 34.0).min(w + 12.0), 24.0));
            let resp = ui
                .interact(zone, ui.id().with(("col_sort", key)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(tr("Clic : trier par cette colonne (encore : sens inverse, puis ordre d'origine). Maj+clic : ajouter un second critère."));
            if resp.hovered() {
                ui.painter().rect_filled(zone, 5, theme::white(0x0C));
                if state.is_none() {
                    color = theme::TEXT;
                }
            }
            if resp.clicked() {
                let add = ui.input(|i| i.modifiers.shift);
                if let Some(s) = sort.as_deref_mut() {
                    s.click(key, add);
                    changed = true;
                }
            }
        }
        let p = ui.painter();
        match label {
            Some(l) => {
                p.text(pos2(x, y), Align2::LEFT_CENTER, l, font.clone(), color);
            }
            None => icons::paint_centered(p, Icon::Clock, pos2(x + 12.0, y), 14.0, color),
        }
        if let Some((rank, desc, total)) = state {
            // Arrow: points up for ascending, down for descending.
            let ax = x + text_w + 9.0;
            let (top, bottom) = if desc { (y + 3.5, y - 2.5) } else { (y - 3.5, y + 2.5) };
            p.add(egui::Shape::convex_polygon(
                vec![pos2(ax, top), pos2(ax - 4.0, bottom), pos2(ax + 4.0, bottom)],
                color,
                egui::Stroke::NONE,
            ));
            if total > 1 {
                p.text(
                    pos2(ax + 7.0, y),
                    Align2::LEFT_CENTER,
                    (rank + 1).to_string(),
                    FontId::proportional(10.0),
                    color,
                );
            }
        }
    }
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0_f32, theme::stroke()),
    );
    changed
}

pub struct RowOut {
    pub rect: Rect,
    /// A drag and drop starts on this row (queue).
    pub drag_started: bool,
}

/// A track row. `list` is the list shown: it is read from row `i`. The first rows cascade in
/// when the page opens.
pub fn track_row(ui: &mut Ui, c: &mut Ctx, list: &[usize], i: usize, opts: &ListOpts) -> RowOut {
    let a = stagger(ui.ctx(), i);
    ui.scope(|ui| {
        ui.multiply_opacity(a);
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
    // Queue: the whole row can be dragged to reorder.
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
    let selected = c.selected.contains(idx) && opts.queue_pos.is_none();
    let cols = columns(rect, opts);
    let t = &c.lib.tracks[idx];
    let fav = c.settings.favorites.contains(&t.path);

    // The highlight of the current track and of the selection fades in and out: on a track
    // change, it glides from one row to the other.
    let cur_t = anim::toggle(ui.ctx(), resp.id.with("cur"), is_current, anim::NORMAL);
    let sel_t = anim::toggle(ui.ctx(), resp.id.with("sel"), selected, anim::FAST);
    let painter = ui.painter();
    if i % 2 == 1 {
        painter.rect_filled(row, CornerRadius::same(6), theme::white(0x04));
    }
    if sel_t > 0.0 && !is_current {
        painter.rect_filled(row, CornerRadius::same(6), theme::white(0x0E).gamma_multiply(sel_t));
    }
    if cur_t > 0.0 {
        painter.rect_filled(
            row,
            CornerRadius::same(6),
            theme::alpha(theme::accent(), 0x1E).gamma_multiply(cur_t),
        );
        // Border on the left, growing from the center.
        let bar = Rect::from_center_size(
            pos2(row.left() + 1.5, row.center().y),
            vec2(3.0, (row.height() - 16.0) * cur_t),
        );
        if bar.height() > 1.0 {
            painter.rect_filled(bar, CornerRadius::same(2), theme::accent_light().gamma_multiply(cur_t));
        }
    }
    widgets::paint_interaction(ui, row, 6, &resp);
    let hovered = resp.hovered();

    // Margin: position number (queue), album art / number, and play button on hover.
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
    let play_t = anim::toggle(ui.ctx(), resp.id.with("play"), show_play, anim::FAST);
    if opts.cover {
        let cover = Rect::from_center_size(gutter.center(), vec2(38.0, 38.0));
        let tex = c.covers.thumb(&t.path).cloned();
        widgets::cover(ui.painter(), cover, tex.as_ref(), 4, &t.album);
        let shade = if is_current && !opts.numbered { 1.0 } else { play_t };
        if shade > 0.0 {
            ui.painter().rect_filled(
                cover,
                CornerRadius::same(4),
                Color32::from_black_alpha((130.0 * shade) as u8),
            );
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
        // The current track offers Pause (or Play if it is paused), not "restart".
        let icon = if is_current && c.playing {
            Icon::Pause
        } else {
            Icon::Play
        };
        // The play button grows a little as it appears on hover.
        icons::paint_centered(
            ui.painter(),
            icon,
            center,
            15.0 * (0.7 + 0.3 * play_t),
            color.gamma_multiply(play_t.max(0.3)),
        );
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

    // Text columns.
    let y = row.center().y;
    let title_color = theme::mix(theme::TEXT, theme::accent_light(), cur_t);
    let other = theme::mix(
        theme::TEXT_2,
        theme::mix(theme::accent_light(), theme::TEXT_2, 0.5),
        cur_t,
    );
    let text = |ui: &Ui, s: &str, col: (f32, f32), font: FontId, color: Color32| {
        let g = widgets::galley(ui, s, font, color, col.1);
        ui.painter().galley(pos2(col.0, y - g.size().y / 2.0), g, color);
    };
    // Rating: nothing for a track without a rating, its stars in small print otherwise; all five,
    // clickable, when hovering the row. Lists that are too narrow don't show it.
    let rating = c.settings.ratings.get(&t.path).copied().unwrap_or(0);
    let row_hovered = ui.rect_contains_pointer(row) && opts.queue_pos.is_none();
    let stars_w = 5.0 * 13.0;
    let mut title_col = cols.title;
    if (rating > 0 || row_hovered) && title_col.1 > 220.0 {
        title_col.1 -= stars_w + 10.0;
        let x0 = title_col.0 + title_col.1 + 8.0;
        let mut preview = None;
        if row_hovered {
            for k in 0..5u8 {
                let r = Rect::from_center_size(pos2(x0 + 6.5 + 13.0 * k as f32, y), vec2(13.0, 18.0));
                let s = ui.interact(r, ui.id().with(("star", i, idx, k)), Sense::click());
                if s.hovered() {
                    preview = Some(k + 1);
                }
                if s.clicked() {
                    // Clicking the current rating again removes it.
                    let n = if rating == k + 1 { 0 } else { k + 1 };
                    c.actions.push(Action::SetRating(idx, n));
                }
            }
        }
        let shown = preview.unwrap_or(rating);
        for k in 0..5u8 {
            let filled = k < shown;
            if !filled && !row_hovered {
                continue;
            }
            let color = if preview.is_some() {
                theme::accent_light()
            } else if filled {
                theme::alpha(theme::accent_light(), 0xC0)
            } else {
                theme::TEXT_3
            };
            icons::paint_centered(
                ui.painter(),
                if filled { Icon::StarFilled } else { Icon::Star },
                pos2(x0 + 6.5 + 13.0 * k as f32, y),
                11.0,
                color,
            );
        }
    }
    text(ui, &t.title, title_col, FontId::proportional(14.0), title_color);
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

    // Favorite, or drag handle in the queue.
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
            .on_hover_text(tr("Glisser pour déplacer"));
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
            tr("Retirer des favoris")
        } else {
            tr("Ajouter aux favoris")
        })
        .clicked()
        {
            c.actions.push(Action::ToggleFavorite(idx));
        }
    }

    let play = || match opts.queue_pos {
        // The current track: pause / resume rather than restarting it from the beginning.
        _ if is_current && c.has_track => Action::TogglePlay,
        Some(_) => Action::QueueJump(i),
        None => Action::Play {
            tracks: list.to_vec(),
            start: i,
        },
    };
    if gutter_resp.clicked() || resp.double_clicked() {
        c.actions.push(play());
    } else if resp.clicked() && opts.queue_pos.is_none() {
        let mods = ui.input(|i| i.modifiers);
        c.selected.click(list, i, mods);
    }
    resp.context_menu(|ui| track_menu(ui, c, list, i, opts));
    gutter_resp.context_menu(|ui| track_menu(ui, c, list, i, opts));
    RowOut { rect, drag_started }
}

fn track_menu(ui: &mut Ui, c: &mut Ctx, list: &[usize], i: usize, opts: &ListOpts) {
    let idx = list[i];
    // Right click within a selection of several tracks: the menu acts on the whole selection.
    let targets = if opts.queue_pos.is_none() {
        c.selected.targets(idx)
    } else {
        vec![idx]
    };
    if targets.len() > 1 {
        selection_menu(ui, c, targets);
        return;
    }
    let t = &c.lib.tracks[idx];
    if widgets::menu_item(ui, Some(Icon::PlayOutline), tr("Lire")).clicked() {
        c.actions.push(match opts.queue_pos {
            Some(_) => Action::QueueJump(i),
            None => Action::Play {
                tracks: list.to_vec(),
                start: i,
            },
        });
    }
    if opts.queue_pos.is_none() {
        if widgets::menu_item(ui, Some(Icon::ListStart), tr("Lire ensuite")).clicked() {
            c.actions.push(Action::PlayNext(vec![idx]));
        }
        if widgets::menu_item(ui, Some(Icon::ListEnd), tr("Ajouter à la file d'attente")).clicked() {
            c.actions.push(Action::Enqueue(vec![idx]));
        }
    }
    add_to_menu(ui, c, vec![idx]);
    // Rating, as a row of stars.
    let rating = c.settings.ratings.get(&t.path).copied().unwrap_or(0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 24.0), Sense::hover());
        icons::paint_centered(ui.painter(), Icon::Star, r.center(), 15.0, theme::TEXT_2);
        ui.label(egui::RichText::new(tr("Note")).color(theme::TEXT));
        ui.add_space(8.0);
        for k in 1..=5u8 {
            let (r, s) = ui.allocate_exact_size(vec2(20.0, 24.0), Sense::click());
            let on = k <= rating;
            let col = if s.hovered() { theme::accent_light() } else if on { theme::alpha(theme::accent_light(), 0xC0) } else { theme::TEXT_3 };
            icons::paint_centered(ui.painter(), if on { Icon::StarFilled } else { Icon::Star }, r.center(), 14.0, col);
            if s.clicked() {
                c.actions.push(Action::SetRating(idx, if rating == k { 0 } else { k }));
                ui.close_menu();
            }
        }
    });
    ui.separator();
    let fav = c.settings.favorites.contains(&t.path);
    if widgets::menu_item(
        ui,
        Some(if fav { Icon::HeartFilled } else { Icon::Heart }),
        if fav {
            tr("Retirer des favoris")
        } else {
            tr("Ajouter aux favoris")
        },
    )
    .clicked()
    {
        c.actions.push(Action::ToggleFavorite(idx));
    }
    let album = c.lib.track_album[idx];
    if widgets::menu_item(ui, Some(Icon::Disc), tr("Afficher l'album")).clicked() {
        c.go(View::Album(album));
    }
    if widgets::menu_item(ui, Some(Icon::Mic), tr("Afficher l'artiste")).clicked() {
        c.go(View::Artist(c.lib.album_artist[album]));
    }
    if widgets::menu_item(ui, Some(Icon::FolderOpen), tr("Ouvrir le dossier")).clicked() {
        if let Some(dir) = t.path.parent() {
            c.actions.push(Action::OpenFolder(dir.to_path_buf()));
        }
    }
    ui.separator();
    if widgets::menu_item(ui, Some(Icon::Tag), tr("Modifier les infos…")).clicked() {
        c.actions.push(Action::EditTags(vec![t.path.clone()]));
    }
    if widgets::menu_item(ui, Some(Icon::Refresh), tr("Convertir…")).clicked() {
        let targets = c.selected.targets(idx);
        c.actions.push(Action::Convert(targets));
    }
    if widgets::menu_item(ui, Some(Icon::Disc), tr("Modifier les infos de l'album…")).clicked() {
        c.actions
            .push(Action::EditTags(paths(c, &c.lib.albums[album].tracks)));
    }
    if widgets::menu_item(ui, Some(Icon::Volume2), tr("ReplayGain de l'album…")).clicked() {
        c.actions.push(Action::ReplayGain(
            paths(c, &c.lib.albums[album].tracks),
            c.lib.albums[album].title.clone(),
        ));
    }
    if let Some(pl) = opts.playlist {
        ui.separator();
        if widgets::menu_item(ui, Some(Icon::Trash), tr("Retirer de la playlist")).clicked() {
            c.actions.push(Action::RemoveFromPlaylist(pl, idx));
        }
    }
    if opts.queue_pos.is_some() {
        ui.separator();
        if widgets::menu_item(ui, Some(Icon::Close), tr("Retirer de la file d'attente")).clicked() {
            c.actions.push(Action::QueueRemove(i));
        }
    }
}

fn paths(c: &Ctx, tracks: &[usize]) -> Vec<std::path::PathBuf> {
    tracks.iter().map(|&t| c.lib.tracks[t].path.clone()).collect()
}

/// Menu of a selection of several tracks.
fn selection_menu(ui: &mut Ui, c: &mut Ctx, targets: Vec<usize>) {
    let n = targets.len();
    if widgets::menu_item(ui, Some(Icon::PlayOutline), &trf!("Lire les {n} morceaux", n)).clicked() {
        c.actions.push(Action::Play {
            tracks: targets.clone(),
            start: 0,
        });
    }
    if widgets::menu_item(ui, Some(Icon::ListStart), tr("Lire ensuite")).clicked() {
        c.actions.push(Action::PlayNext(targets.clone()));
    }
    if widgets::menu_item(ui, Some(Icon::ListEnd), tr("Ajouter à la file d'attente")).clicked() {
        c.actions.push(Action::Enqueue(targets.clone()));
    }
    add_to_menu(ui, c, targets.clone());
    ui.separator();
    if widgets::menu_item(
        ui,
        Some(Icon::Tag),
        &trf!("Modifier les infos des {n} morceaux…", n),
    )
    .clicked()
    {
        c.actions.push(Action::EditTags(paths(c, &targets)));
    }
    if widgets::menu_item(ui, Some(Icon::Volume2), tr("ReplayGain de la sélection…")).clicked() {
        c.actions
            .push(Action::ReplayGain(paths(c, &targets), tr("La sélection").into()));
    }
    if widgets::menu_item(ui, Some(Icon::Close), tr("Désélectionner")).clicked() {
        c.selected.clear();
    }
}

/// "Add to" submenu: new playlist, queue, existing playlists.
pub fn add_to_menu(ui: &mut Ui, c: &mut Ctx, tracks: Vec<usize>) {
    ui.menu_button(tr("       Ajouter à une playlist"), |ui| {
        if widgets::menu_item(ui, Some(Icon::Plus), tr("Nouvelle playlist…")).clicked() {
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

/// Virtual list (only visible rows are drawn), with its own scrolling.
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

/// Full list, to be placed in an area that already scrolls (album, artist, search).
pub fn track_list_inline(ui: &mut Ui, c: &mut Ctx, list: &[usize], opts: &ListOpts) {
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    for i in 0..list.len() {
        track_row(ui, c, list, i, opts);
    }
    ui.spacing_mut().item_spacing.y = spacing;
}

// ---------- Grids ----------

pub const CARD_MIN: f32 = 160.0;
pub const GAP: f32 = 20.0;

/// Number of columns and card width to fill `width`.
pub fn grid_layout(width: f32) -> (usize, f32) {
    let cols = ((width + GAP) / (CARD_MIN + GAP)).floor().max(1.0) as usize;
    let w = ((width - GAP * (cols - 1) as f32) / cols as f32).min(240.0);
    (cols, w)
}

pub fn card_height(w: f32) -> f32 {
    w + 58.0
}

/// Grid of items (in an area that already scrolls), limited to `max_rows` rows.
pub fn grid(
    ui: &mut Ui,
    salt: &str,
    n: usize,
    max_rows: Option<usize>,
    mut card: impl FnMut(&mut Ui, usize, f32),
) {
    let (cols, w) = grid_layout(ui.available_width());
    let rows = n.div_ceil(cols).min(max_rows.unwrap_or(usize::MAX));
    // The same album can appear in two sections: distinct identifiers per section.
    ui.push_id(salt, |ui| {
        for r in 0..rows {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for k in r * cols..((r + 1) * cols).min(n) {
                    let a = stagger(ui.ctx(), k);
                    ui.scope(|ui| {
                        ui.multiply_opacity(a);
                        card(ui, k, w);
                    });
                }
            });
            ui.add_space(GAP * 0.6);
        }
    });
}

/// Virtual grid, with its own scrolling.
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
                            ui.multiply_opacity(a);
                            card(ui, k, w);
                        });
                    }
                });
            }
            ui.add_space(16.0);
        });
}

/// Album card: album art lifting on hover, play button, title and artist.
pub fn album_card(ui: &mut Ui, c: &mut Ctx, a: usize, w: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, card_height(w)), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let album = &c.lib.albums[a];
    let resp = ui.interact(rect, ui.id().with(("album", a)), Sense::click());
    // Hover over the whole card, play button included: otherwise, hovering the button "leaves"
    // the card, which goes back down, which moves the button… and the card shook.
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
    // Album playing: badge with the animated equalizer.
    if c.current.is_some_and(|t| c.lib.track_album[t] == a) {
        let center = pos2(cover.right() - 22.0, cover.top() + 22.0);
        ui.painter()
            .circle_filled(center, 15.0, Color32::from_black_alpha(150));
        widgets::equalizer(ui.painter(), center, 13.0, theme::accent_light(), c.playing);
    }

    // Play button, at the bottom left of the album art.
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
        if pr.on_hover_text(tr("Lire l'album")).clicked() {
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
    if widgets::menu_item(ui, Some(Icon::PlayOutline), tr("Lire")).clicked() {
        c.actions.push(Action::Play {
            tracks: tracks.clone(),
            start: 0,
        });
    }
    if widgets::menu_item(ui, Some(Icon::Shuffle), tr("Lecture aléatoire")).clicked() {
        c.actions.push(Action::PlayShuffled(tracks.clone()));
    }
    if widgets::menu_item(ui, Some(Icon::ListStart), tr("Lire ensuite")).clicked() {
        c.actions.push(Action::PlayNext(tracks.clone()));
    }
    if widgets::menu_item(ui, Some(Icon::ListEnd), tr("Ajouter à la file d'attente")).clicked() {
        c.actions.push(Action::Enqueue(tracks.clone()));
    }
    add_to_menu(ui, c, tracks.clone());
    ui.separator();
    if widgets::menu_item(ui, Some(Icon::Mic), tr("Afficher l'artiste")).clicked() {
        c.go(View::Artist(c.lib.album_artist[a]));
    }
    if widgets::menu_item(ui, Some(Icon::FolderOpen), tr("Ouvrir le dossier")).clicked() {
        if let Some(dir) = c.lib.tracks[tracks[0]].path.parent() {
            c.actions.push(Action::OpenFolder(dir.to_path_buf()));
        }
    }
    ui.separator();
    if widgets::menu_item(ui, Some(Icon::Tag), tr("Modifier les infos de l'album…")).clicked() {
        c.actions.push(Action::EditTags(paths(c, &tracks)));
    }
    if widgets::menu_item(ui, Some(Icon::Volume2), tr("ReplayGain de l'album…")).clicked() {
        c.actions.push(Action::ReplayGain(
            paths(c, &tracks),
            c.lib.albums[a].title.clone(),
        ));
    }
    if widgets::menu_item(ui, Some(Icon::Refresh), tr("Convertir l'album…")).clicked() {
        c.actions.push(Action::Convert(tracks.clone()));
    }
}

/// Artist card: round avatar (album art of their first album), name, number of albums.
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
        if widgets::menu_item(ui, Some(Icon::PlayOutline), tr("Lire")).clicked() {
            c.actions.push(Action::Play {
                tracks: tracks.clone(),
                start: 0,
            });
        }
        if widgets::menu_item(ui, Some(Icon::Shuffle), tr("Lecture aléatoire")).clicked() {
            c.actions.push(Action::PlayShuffled(tracks.clone()));
        }
        if widgets::menu_item(ui, Some(Icon::ListEnd), tr("Ajouter à la file d'attente")).clicked() {
            c.actions.push(Action::Enqueue(tracks.clone()));
        }
        add_to_menu(ui, c, tracks.clone());
        ui.separator();
        if widgets::menu_item(
            ui,
            Some(Icon::Tag),
            tr("Modifier les infos de tous ses morceaux…"),
        )
        .clicked()
        {
            c.actions.push(Action::EditTags(paths(c, &tracks)));
        }
    });
}

/// Section title with an optional "Show all" link; returns true if the link was clicked.
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

/// Appearance of a page: returns its progress (0 → 1) and prepares the cascade of items.
pub fn page_progress(ctx: &egui::Context, key: &str) -> f32 {
    let (fade, cascade) = page_ids(key);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("page_cascade_id"), cascade));
    let t = anim::value_from(ctx, fade, 0.0, 1.0, anim::SLOW, anim::Easing::OutExpo);
    let s = anim::value_from(ctx, cascade, 0.0, 1.0, 0.75, anim::Easing::Out);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("page_stagger"), s));
    t
}

/// Replays the appearance of a page (navigation).
pub fn reset_page(ctx: &egui::Context, key: &str) {
    let (fade, cascade) = page_ids(key);
    anim::reset(ctx, fade);
    anim::reset(ctx, cascade);
}

/// Replays the cascade of items of the displayed page (tab change), without making the whole
/// page appear again.
pub fn restart_cascade(ctx: &egui::Context) {
    if let Some(id) = ctx.data(|d| d.get_temp::<egui::Id>(egui::Id::new("page_cascade_id"))) {
        anim::reset(ctx, id);
    }
}

/// Opacity of the `i`-th item of an appearing page (the first ones first).
pub fn stagger(ctx: &egui::Context, i: usize) -> f32 {
    let s = ctx
        .data(|d| d.get_temp::<f32>(egui::Id::new("page_stagger")))
        .unwrap_or(1.0);
    ((s - (i.min(15) as f32) * 0.04) / 0.4).clamp(0.0, 1.0)
}

#[cfg(test)]
mod sort_tests {
    use super::*;

    #[test]
    fn column_clicks() {
        let mut s = SortSpec::default();
        s.click(SortKey::Title, false);
        assert_eq!(s.0, vec![(SortKey::Title, false)]);
        s.click(SortKey::Title, false);
        assert_eq!(s.0, vec![(SortKey::Title, true)]);
        s.click(SortKey::Title, false);
        assert!(s.0.is_empty()); // third click: original order
        s.click(SortKey::Artist, false);
        s.click(SortKey::Year, true);
        s.click(SortKey::Year, true);
        assert_eq!(s.0, vec![(SortKey::Artist, false), (SortKey::Year, true)]);
        // A simple click on another column starts again from a single criterion.
        s.click(SortKey::Album, false);
        assert_eq!(s.0, vec![(SortKey::Album, false)]);
    }
}
