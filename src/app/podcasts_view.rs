//! Podcasts: subscriptions, search, episodes, and listening to an episode (outside the queue,
//! resuming where you left off).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, pos2, vec2, Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense};

use super::views::heading;
use super::{Action, SterMusic, View};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::podcasts::{self, Episode, Podcast, Progress};
use crate::trf;
use crate::{theme, widgets};

/// The episode being listened to.
pub struct Playing {
    pub podcast: Podcast,
    pub episode: Episode,
}

type FeedResult = Result<(Podcast, Vec<Episode>), String>;
/// A download in progress: progress (‰) and result.
type Download = (Arc<AtomicU32>, Receiver<Result<PathBuf, String>>);

#[derive(Default)]
pub struct PodcastState {
    pub playing: Option<Playing>,
    /// Episodes of each feed (read from disk on demand).
    episodes: HashMap<String, Vec<Episode>>,
    /// Refreshes in progress, and the time of the last one (per feed).
    refreshing: Vec<(String, Receiver<FeedResult>)>,
    refreshed: HashMap<String, f64>,
    query: String,
    results: Vec<Podcast>,
    searching: Option<Receiver<Result<Vec<Podcast>, String>>>,
    error: Option<String>,
    /// Adding a feed by its address.
    add_url: Option<String>,
    /// Downloads: progress (‰) and result.
    downloads: HashMap<String, Download>,
    last_save: f64,
}

impl PodcastState {
    fn episodes(&mut self, feed: &str) -> &[Episode] {
        if !self.episodes.contains_key(feed) {
            self.episodes
                .insert(feed.to_owned(), podcasts::load_episodes(feed));
        }
        &self.episodes[feed]
    }

    fn refresh(&mut self, ctx: &egui::Context, feed: &str) {
        if self.refreshing.iter().any(|(f, _)| f == feed) {
            return;
        }
        let (tx, rx) = channel();
        let (f, c) = (feed.to_owned(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(podcasts::fetch(&f));
            c.request_repaint();
        });
        self.refreshing.push((feed.to_owned(), rx));
        self.refreshed.insert(feed.to_owned(), ctx.input(|i| i.time));
    }
}

/// "12 mars 2024" (or "12 Mar 2024" in English).
fn fmt_date(secs: u64) -> String {
    fmt_date_in(secs, crate::i18n::lang())
}

fn fmt_date_in(secs: u64, lang: crate::i18n::Lang) -> String {
    if secs == 0 {
        return String::new();
    }
    let days = (secs / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    const FR: [&str; 12] = [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc.",
    ];
    const EN: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let months = if lang == crate::i18n::Lang::Fr { FR } else { EN };
    format!("{d} {} {y}", months[(m - 1) as usize])
}

impl SterMusic {
    // ---------- Playback ----------

    /// Plays an episode from where you left off (the downloaded file if it exists).
    pub(super) fn play_episode(&mut self, podcast: Podcast, episode: Episode) {
        self.save_episode_progress();
        self.leave_radio();
        self.finish_listen();
        let media = match podcasts::downloaded(&episode) {
            Some(file) => {
                let mut m = crate::media::Media::file(file);
                m.duration = Some(Duration::from_millis(episode.duration_ms)).filter(|d| !d.is_zero());
                m
            }
            None => crate::media::Media::url(
                &episode.url,
                Some(Duration::from_millis(episode.duration_ms)).filter(|d| !d.is_zero()),
            ),
        };
        let Some(player) = &mut self.player else { return };
        player.set_meta(crate::cast::Meta {
            title: episode.title.clone(),
            artist: podcast.title.clone(),
            album: tr("Podcast").into(),
            cover_url: Some(if episode.image.is_empty() {
                podcast.image.clone()
            } else {
                episode.image.clone()
            })
            .filter(|u| !u.is_empty()),
            ..Default::default()
        });
        if let Err(e) = player.load(&media, false) {
            self.toast(format!("{} — {e}", episode.title));
            return;
        }
        let saved = self
            .settings
            .episode_progress
            .get(&episode.key())
            .copied()
            .unwrap_or_default();
        if !saved.done && saved.position > 5.0 {
            // A little before the place left, to pick up the thread.
            let _ = player.seek(Duration::from_secs_f32(saved.position - 2.0));
        }
        self.podcasts.playing = Some(Playing { podcast, episode });
        self.seeked = true;
    }

    /// Records how far the current episode got (called regularly, and when leaving it).
    pub(super) fn save_episode_progress(&mut self) {
        let Some(p) = &self.podcasts.playing else { return };
        let Some(player) = &self.player else { return };
        if !player.has_track() {
            return;
        }
        let pos = player.position().as_secs_f32();
        let total = p.episode.duration_ms as f32 / 1000.0;
        // The last seconds (credits) count as finished.
        let done = total > 0.0 && pos >= total - 20.0;
        let entry = self.settings.episode_progress.entry(p.episode.key()).or_default();
        if done {
            *entry = Progress {
                position: 0.0,
                done: true,
            };
        } else if pos > 1.0 {
            entry.position = pos;
        }
    }

    /// Leaves the current episode (a track or a radio is about to play, or Stop).
    pub(super) fn leave_episode(&mut self) {
        self.save_episode_progress();
        self.podcasts.playing = None;
    }

    /// The episode is finished: marked as played, playback stops.
    pub(super) fn episode_finished(&mut self) {
        if let Some(p) = self.podcasts.playing.take() {
            self.settings.episode_progress.insert(
                p.episode.key(),
                Progress {
                    position: 0.0,
                    done: true,
                },
            );
        }
        if let Some(player) = &mut self.player {
            player.stop();
        }
        self.seeked = true;
    }

    pub(super) fn poll_podcasts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let mut done = Vec::new();
        self.podcasts.refreshing.retain(|(feed, rx)| match rx.try_recv() {
            Ok(r) => {
                done.push((feed.clone(), r));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for (feed, result) in done {
            match result {
                Ok((info, episodes)) => {
                    podcasts::save_episodes(&feed, &episodes);
                    self.podcasts.episodes.insert(feed.clone(), episodes);
                    if let Some(p) = self.settings.podcasts.iter_mut().find(|p| p.feed == feed) {
                        // The feed may have changed its title or image.
                        p.title = info.title;
                        p.author = info.author;
                        if !info.image.is_empty() {
                            p.image = info.image;
                        }
                        p.description = info.description;
                    }
                }
                Err(e) => self.podcasts.error = Some(e),
            }
        }
        if let Some(rx) = &self.podcasts.searching {
            if let Ok(r) = rx.try_recv() {
                self.podcasts.searching = None;
                match r {
                    Ok(list) => self.podcasts.results = list,
                    Err(e) => self.podcasts.error = Some(trf!("Annuaire injoignable : {e}", e)),
                }
            }
        }
        let mut finished = Vec::new();
        for (key, (_, rx)) in &self.podcasts.downloads {
            if let Ok(r) = rx.try_recv() {
                finished.push((key.clone(), r));
            }
        }
        for (key, r) in finished {
            self.podcasts.downloads.remove(&key);
            if let Err(e) = r {
                self.toast(trf!("Téléchargement impossible : {e}", e));
            }
        }
        if self.podcasts.playing.is_some() && now - self.podcasts.last_save > 5.0 {
            self.podcasts.last_save = now;
            self.save_episode_progress();
        }
        if !self.podcasts.refreshing.is_empty()
            || self.podcasts.searching.is_some()
            || !self.podcasts.downloads.is_empty()
        {
            ctx.request_repaint_after(Duration::from_millis(300));
        }
    }

    fn subscribe(&mut self, ctx: &egui::Context, podcast: Podcast) {
        if self.settings.podcasts.iter().any(|p| p.feed == podcast.feed) {
            return;
        }
        let feed = podcast.feed.clone();
        self.toast(trf!("Abonné à « {0} »", podcast.title));
        self.settings.podcasts.push(podcast);
        self.podcasts.refresh(ctx, &feed);
    }

    fn unsubscribe(&mut self, feed: &str) {
        self.settings.podcasts.retain(|p| p.feed != feed);
        if matches!(&self.view, View::Podcast(f) if f == feed) {
            self.actions.push(Action::Back);
        }
    }

    // ---------- Pages ----------

    pub(super) fn podcasts_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // Feeds older than an hour are read again when the page opens.
        let now = ctx.input(|i| i.time);
        for p in self.settings.podcasts.clone() {
            let last = self.podcasts.refreshed.get(&p.feed).copied();
            if last.is_none_or(|t| now - t > 3600.0) {
                self.podcasts.refresh(&ctx, &p.feed);
            }
        }
        ui.horizontal(|ui| {
            heading(ui, tr("Podcasts"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if widgets::secondary_button(ui, Some(Icon::Link), tr("Ajouter un flux RSS…")).clicked() {
                    self.podcasts.add_url = Some(String::new());
                }
            });
        });
        ui.add_space(14.0);
        egui::ScrollArea::vertical()
            .id_salt("podcasts_page")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                // Directory search.
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 130.0).clamp(160.0, 420.0);
                    let field = widgets::search_field(
                        ui,
                        egui::Id::new("podcast_search"),
                        &mut self.podcasts.query,
                        tr("Chercher un podcast"),
                        w,
                    );
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let go = widgets::secondary_button(ui, Some(Icon::Search), tr("Chercher")).clicked();
                    let q = self.podcasts.query.trim().to_owned();
                    if (enter || go) && !q.is_empty() {
                        let (tx, rx) = channel();
                        let ctx = ctx.clone();
                        let country = crate::radio::desktop_country();
                        std::thread::spawn(move || {
                            let _ = tx.send(podcasts::search(&q, &country));
                            ctx.request_repaint();
                        });
                        self.podcasts.searching = Some(rx);
                        self.podcasts.error = None;
                    }
                });
                if let Some(url) = &mut self.podcasts.add_url {
                    ui.add_space(8.0);
                    let mut add = false;
                    let mut cancel = false;
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(url)
                                .desired_width(420.0)
                                .hint_text(tr("Adresse du flux (https://…/feed.xml)")),
                        );
                        add = widgets::primary_button(ui, None, tr("S'abonner")).clicked();
                        cancel = widgets::secondary_button(ui, None, tr("Annuler")).clicked();
                    });
                    let feed = url.trim().to_owned();
                    if add && feed.starts_with("http") {
                        self.podcasts.add_url = None;
                        let p = Podcast {
                            feed: feed.clone(),
                            title: feed,
                            ..Default::default()
                        };
                        self.subscribe(&ctx, p);
                    } else if cancel {
                        self.podcasts.add_url = None;
                    }
                }
                if let Some(e) = &self.podcasts.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).small().color(theme::TEXT_3));
                }
                if self.podcasts.searching.is_some() {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(tr("Recherche dans l'annuaire…")).color(theme::TEXT_3));
                    });
                }
                if !self.podcasts.results.is_empty() {
                    ui.add_space(14.0);
                    let clear = super::lists::section(ui, tr("Résultats"), Some(tr("Effacer")));
                    if clear {
                        self.podcasts.results.clear();
                    }
                    let results = self.podcasts.results.clone();
                    self.podcast_grid(ui, "results", &results, true);
                }

                ui.add_space(14.0);
                super::lists::section(ui, tr("Mes abonnements"), None);
                let subs = self.settings.podcasts.clone();
                if subs.is_empty() {
                    ui.label(
                        RichText::new(tr("Cherche un podcast ci-dessus, ou ajoute son flux RSS."))
                            .color(theme::TEXT_3),
                    );
                } else {
                    self.podcast_grid(ui, "subs", &subs, false);
                    // The latest episodes of all subscriptions.
                    let mut latest: Vec<(Podcast, Episode)> = Vec::new();
                    for p in &subs {
                        for e in self.podcasts.episodes(&p.feed).iter().take(5) {
                            latest.push((p.clone(), e.clone()));
                        }
                    }
                    latest.sort_by_key(|e| std::cmp::Reverse(e.1.date));
                    if !latest.is_empty() {
                        ui.add_space(14.0);
                        super::lists::section(ui, tr("Derniers épisodes"), None);
                        for (i, (p, e)) in latest.iter().take(15).enumerate() {
                            self.episode_row(ui, p, e, i, true);
                        }
                    }
                }
                ui.add_space(24.0);
            });
    }

    /// Podcast grid (subscriptions, or search results with "Subscribe").
    fn podcast_grid(&mut self, ui: &mut egui::Ui, salt: &str, list: &[Podcast], results: bool) {
        let ctx = ui.ctx().clone();
        let mut go = None;
        let mut sub = None;
        super::lists::grid(ui, salt, list.len(), None, |ui, k, w| {
            let p = &list[k];
            let (rect, resp) = ui.allocate_exact_size(vec2(w, w + 58.0), Sense::click());
            if !ui.is_rect_visible(rect) {
                return;
            }
            let h = widgets::hover_t(ui, resp.id, resp.hovered());
            let art = Rect::from_min_size(rect.min, vec2(w, w)).translate(vec2(0.0, -3.0 * h));
            ui.painter().add(
                egui::Shadow {
                    offset: [0, (4.0 + 6.0 * h) as i8],
                    blur: (10.0 + 14.0 * h) as u8,
                    spread: 0,
                    color: Color32::from_black_alpha((70.0 + 50.0 * h) as u8),
                }
                .as_shape(art, CornerRadius::same(10)),
            );
            match self.radio.image(&ctx, &p.image) {
                Some(tex) => widgets::cover(ui.painter(), art, Some(&tex), 10, &p.title),
                None => {
                    widgets::cover(ui.painter(), art, None, 10, &p.title);
                    crate::icons::paint_centered(
                        ui.painter(),
                        Icon::Podcast,
                        art.center(),
                        w * 0.3,
                        Color32::WHITE,
                    );
                }
            }
            let g = widgets::galley(ui, &p.title, theme::semibold(14.0), theme::TEXT, w);
            ui.painter()
                .galley(pos2(rect.left(), art.bottom() + 10.0 + 3.0 * h), g, theme::TEXT);
            let g = widgets::galley(ui, &p.author, FontId::proportional(13.0), theme::TEXT_2, w);
            ui.painter()
                .galley(pos2(rect.left(), art.bottom() + 31.0 + 3.0 * h), g, theme::TEXT_2);
            if results {
                let subscribed = self.settings.podcasts.iter().any(|s| s.feed == p.feed);
                let b = Rect::from_min_size(art.left_top() + vec2(8.0, 8.0), vec2(32.0, 32.0));
                ui.painter()
                    .circle_filled(b.center(), 16.0, Color32::from_black_alpha(150));
                crate::icons::paint_centered(
                    ui.painter(),
                    if subscribed { Icon::HeartFilled } else { Icon::Plus },
                    b.center(),
                    15.0,
                    Color32::WHITE,
                );
            }
            if resp
                .on_hover_text(if results {
                    tr("S'abonner")
                } else {
                    tr("Voir les épisodes")
                })
                .clicked()
            {
                if results {
                    sub = Some(p.clone());
                } else {
                    go = Some(p.feed.clone());
                }
            }
        });
        if let Some(p) = sub {
            let feed = p.feed.clone();
            self.subscribe(&ctx, p);
            go = Some(feed);
        }
        if let Some(feed) = go {
            self.actions.push(Action::Go(View::Podcast(feed)));
        }
    }

    /// Page of a podcast: its header and all its episodes.
    pub(super) fn podcast_page(&mut self, ui: &mut egui::Ui, feed: &str) {
        let ctx = ui.ctx().clone();
        let Some(p) = self.settings.podcasts.iter().find(|p| p.feed == feed).cloned() else {
            ui.label(RichText::new(tr("Plus abonné à ce podcast.")).color(theme::TEXT_3));
            return;
        };
        let episodes = self.podcasts.episodes(feed).to_vec();
        let refreshing = self.podcasts.refreshing.iter().any(|(f, _)| f == feed);
        egui::ScrollArea::vertical()
            .id_salt(("podcast", feed))
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 16.0);
                let w = ui.available_width();
                let (rect, _) = ui.allocate_exact_size(vec2(w, 212.0), Sense::hover());
                let art = Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(200.0, 200.0));
                match self.radio.image(&ctx, &p.image) {
                    Some(tex) => widgets::cover(ui.painter(), art, Some(&tex), 10, &p.title),
                    None => widgets::cover(ui.painter(), art, None, 10, &p.title),
                }
                let x = art.right() + 28.0;
                let tw = rect.right() - x - 16.0;
                ui.painter().text(
                    pos2(x, art.top() + 12.0),
                    egui::Align2::LEFT_TOP,
                    tr("PODCAST"),
                    FontId::proportional(12.0),
                    theme::accent_light(),
                );
                let g = widgets::galley(ui, &p.title, theme::semibold(30.0), theme::TEXT, tw);
                ui.painter().galley(pos2(x, art.top() + 30.0), g, theme::TEXT);
                let g = widgets::galley(ui, &p.author, FontId::proportional(15.0), theme::TEXT, tw);
                ui.painter().galley(pos2(x, art.top() + 74.0), g, theme::TEXT);
                let desc = ui.fonts(|f| {
                    let mut job = egui::text::LayoutJob::single_section(
                        p.description.clone(),
                        egui::TextFormat::simple(FontId::proportional(13.0), theme::TEXT_2),
                    );
                    job.wrap = egui::text::TextWrapping {
                        max_width: tw,
                        max_rows: 3,
                        break_anywhere: false,
                        overflow_character: Some('…'),
                    };
                    f.layout_job(job)
                });
                ui.painter()
                    .galley(pos2(x, art.top() + 100.0), desc, theme::TEXT_2);
                let b = Rect::from_min_max(pos2(x, art.bottom() - 36.0), pos2(rect.right(), art.bottom()));
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(b)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        if let Some(first) = episodes.iter().find(|e| {
                            !self
                                .settings
                                .episode_progress
                                .get(&e.key())
                                .is_some_and(|s| s.done)
                        }) {
                            if widgets::primary_button(ui, Some(Icon::Play), tr("Écouter le dernier"))
                                .clicked()
                            {
                                self.play_episode(p.clone(), first.clone());
                            }
                        }
                        if refreshing {
                            ui.spinner();
                        } else if widgets::secondary_button(ui, Some(Icon::Refresh), tr("Actualiser"))
                            .clicked()
                        {
                            self.podcasts.refresh(&ctx, feed);
                        }
                        if widgets::secondary_button(ui, Some(Icon::Trash), tr("Se désabonner")).clicked() {
                            self.unsubscribe(feed);
                        }
                    },
                );
                ui.add_space(18.0);
                super::lists::section(
                    ui,
                    &crate::library::plural(episodes.len(), "épisode", "épisodes"),
                    None,
                );
                if episodes.is_empty() {
                    ui.label(
                        RichText::new(if refreshing {
                            tr("Chargement…")
                        } else {
                            tr("Aucun épisode.")
                        })
                        .color(theme::TEXT_3),
                    );
                }
                for (i, e) in episodes.iter().enumerate() {
                    self.episode_row(ui, &p, e, i, false);
                }
                ui.add_space(24.0);
            });
    }

    /// Row of an episode: play, date and duration, progress, download.
    fn episode_row(&mut self, ui: &mut egui::Ui, p: &Podcast, e: &Episode, i: usize, show_podcast: bool) {
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 64.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let key = e.key();
        let progress = self
            .settings
            .episode_progress
            .get(&key)
            .copied()
            .unwrap_or_default();
        let is_current = self
            .podcasts
            .playing
            .as_ref()
            .is_some_and(|c| c.episode.key() == key);
        let playing = is_current && self.is_playing();
        if is_current {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), theme::alpha(theme::accent(), 0x26));
        } else if i % 2 == 1 {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), theme::white(0x04));
        }
        widgets::paint_interaction(ui, rect, 8, &resp);
        // Round play button.
        let play = Rect::from_center_size(pos2(rect.left() + 28.0, rect.center().y), vec2(36.0, 36.0));
        let pr = ui.interact(play, ui.id().with(("ep_play", &key)), Sense::click());
        let ph = widgets::hover_t(ui, pr.id, pr.hovered());
        theme::brand_fill(ui.painter(), play, CornerRadius::same(18), 0.75 + 0.25 * ph);
        crate::icons::paint_centered(
            ui.painter(),
            if playing { Icon::Pause } else { Icon::Play },
            play.center() + vec2(if playing { 0.0 } else { 1.0 }, 0.0),
            14.0,
            Color32::WHITE,
        );
        let x = play.right() + 14.0;
        let right_w = 150.0;
        let tw = rect.right() - x - right_w;
        let title_color = if progress.done && !is_current {
            theme::TEXT_2
        } else {
            theme::TEXT
        };
        let g = widgets::galley(ui, &e.title, theme::semibold(14.0), title_color, tw);
        ui.painter().galley(pos2(x, rect.top() + 10.0), g, title_color);
        let mut meta = Vec::new();
        if show_podcast {
            meta.push(p.title.clone());
        }
        let date = fmt_date(e.date);
        if !date.is_empty() {
            meta.push(date);
        }
        if e.duration_ms > 0 {
            meta.push(crate::library::fmt_long(Duration::from_millis(e.duration_ms)));
        }
        if progress.done {
            meta.push(tr("écouté").into());
        } else if progress.position > 0.0 && e.duration_ms > 0 {
            let left = (e.duration_ms as f32 / 1000.0 - progress.position).max(0.0);
            meta.push(trf!(
                "reste {0}",
                crate::library::fmt_long(Duration::from_secs_f32(left))
            ));
        }
        let g = widgets::galley(
            ui,
            &meta.join(" · "),
            FontId::proportional(12.5),
            theme::TEXT_3,
            tw,
        );
        ui.painter().galley(pos2(x, rect.top() + 31.0), g, theme::TEXT_3);
        // Progress, as a thin line under the text.
        if !progress.done && progress.position > 0.0 && e.duration_ms > 0 {
            let frac = (progress.position * 1000.0 / e.duration_ms as f32).clamp(0.0, 1.0);
            let bar = Rect::from_min_size(pos2(x, rect.bottom() - 10.0), vec2(tw.min(240.0), 3.0));
            ui.painter()
                .rect_filled(bar, CornerRadius::same(2), theme::white(0x20));
            let fill = Rect::from_min_size(bar.min, vec2(bar.width() * frac, 3.0));
            ui.painter()
                .rect_filled(fill, CornerRadius::same(2), theme::accent_light());
        }
        // On the right: download, mark as played.
        let cy = rect.center().y;
        let dl = Rect::from_center_size(pos2(rect.right() - 60.0, cy), vec2(32.0, 32.0));
        let mark = Rect::from_center_size(pos2(rect.right() - 22.0, cy), vec2(32.0, 32.0));
        let downloading = self
            .podcasts
            .downloads
            .get(&key)
            .map(|(p, _)| p.load(Ordering::Relaxed));
        if let Some(per_mille) = downloading {
            ui.painter().text(
                dl.center(),
                egui::Align2::CENTER_CENTER,
                crate::i18n::percent(per_mille as f32 / 10.0),
                FontId::proportional(11.0),
                theme::TEXT_2,
            );
        } else {
            let have = podcasts::downloaded(e).is_some();
            let (icon, tip) = if have {
                (Icon::Trash, tr("Supprimer le fichier téléchargé"))
            } else {
                (Icon::Download, tr("Télécharger pour l'écoute hors ligne"))
            };
            if widgets::icon_button(
                &mut ui.new_child(egui::UiBuilder::new().max_rect(dl)),
                icon,
                tip,
                32.0,
                15.0,
            )
            .clicked()
            {
                if have {
                    let _ = std::fs::remove_file(podcasts::download_path(e));
                } else {
                    let progress = Arc::new(AtomicU32::new(0));
                    let (tx, rx) = channel();
                    let (ep, pr, ctx) = (e.clone(), progress.clone(), ui.ctx().clone());
                    std::thread::spawn(move || {
                        let _ = tx.send(podcasts::download(&ep, &pr));
                        ctx.request_repaint();
                    });
                    self.podcasts.downloads.insert(key.clone(), (progress, rx));
                }
            }
        }
        let (icon, tip) = if progress.done {
            (Icon::Undo, tr("Marquer comme non écouté"))
        } else {
            (Icon::Sparkles, tr("Marquer comme écouté"))
        };
        if widgets::icon_button(
            &mut ui.new_child(egui::UiBuilder::new().max_rect(mark)),
            icon,
            tip,
            32.0,
            15.0,
        )
        .clicked()
        {
            self.settings.episode_progress.insert(
                key.clone(),
                Progress {
                    position: 0.0,
                    done: !progress.done,
                },
            );
        }
        let clicked = pr.clicked() || resp.clicked();
        resp.on_hover_text(e.description.chars().take(300).collect::<String>());
        if clicked {
            if is_current {
                self.actions.push(Action::TogglePlay);
            } else {
                self.play_episode(p.clone(), e.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates() {
        use crate::i18n::Lang;
        assert_eq!(super::fmt_date_in(1_709_662_930, Lang::Fr), "5 mars 2024");
        assert_eq!(super::fmt_date_in(1_709_662_930, Lang::En), "5 Mar 2024");
        assert_eq!(super::fmt_date_in(0, Lang::Fr), "");
    }
}
