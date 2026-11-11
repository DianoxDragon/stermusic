//! Albums sans pochette : on les repère dans la bibliothèque (ni image intégrée, ni image dans le
//! dossier), on cherche leurs pochettes sur MusicBrainz et le Cover Art Archive, on vérifie le
//! résultat d'un coup d'œil, puis les images retenues sont intégrées à tous les morceaux des
//! albums cochés, en une fois.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use eframe::egui::{self, pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, Sense};

use super::lib_tools;
use super::tag_editor::{cover_from_bytes, cover_from_file, paint_cover_texture, CoverSlot, Job};
use super::SterMusic;
use crate::i18n::tr;
use crate::icons::Icon;
use crate::library::{plural, unknown_album, unknown_artist};
use crate::musicbrainz;
use crate::tags::Cover;
use crate::trf;
use crate::{theme, widgets};

const WARN: Color32 = Color32::from_rgb(0xFF, 0xC0, 0x70);

enum State {
    /// Pas encore cherché.
    Idle,
    Queued,
    Searching,
    /// Pochette trouvée (ou choisie sur le disque), et d'où elle vient.
    Found(Cover, String),
    NotFound,
    Failed(String),
}

struct Missing {
    title: String,
    artist: String,
    paths: Vec<PathBuf>,
    checked: bool,
    state: State,
    tex: CoverSlot,
}

impl Missing {
    /// Un album sans nom ne peut pas être cherché en ligne.
    fn searchable(&self) -> bool {
        self.title != unknown_album()
    }
}

enum ScanMsg {
    Progress(usize, usize),
    Done(Vec<Missing>),
}

struct Search {
    rx: Receiver<(usize, State)>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct CoverTool {
    scan: Option<Receiver<ScanMsg>>,
    scan_progress: (usize, usize),
    albums: Vec<Missing>,
    search: Option<Search>,
}

impl CoverTool {
    pub fn busy(&self) -> bool {
        self.scan.is_some() || self.search.is_some()
    }

    pub fn stop(&mut self) {
        if let Some(s) = self.search.take() {
            s.cancel.store(true, Ordering::Relaxed);
        }
        for a in &mut self.albums {
            if matches!(a.state, State::Queued | State::Searching) {
                a.state = State::Idle;
            }
        }
    }

    pub fn poll(&mut self) {
        if let Some(rx) = &self.scan {
            let mut done = None;
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    ScanMsg::Progress(n, total) => self.scan_progress = (n, total),
                    ScanMsg::Done(albums) => done = Some(albums),
                }
            }
            if let Some(albums) = done {
                self.albums = albums;
                self.scan = None;
            }
        }
        if let Some(s) = &self.search {
            let mut alive = true;
            loop {
                match s.rx.try_recv() {
                    Ok((i, state)) => {
                        if let Some(a) = self.albums.get_mut(i) {
                            // Rien trouvé : l'album n'est plus à intégrer.
                            a.checked = !matches!(state, State::NotFound | State::Failed(_));
                            a.state = state;
                            a.tex = None;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        alive = false;
                        break;
                    }
                }
            }
            if !alive {
                self.search = None;
            }
        }
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        let mut wanted = Vec::new();
        for (i, a) in self.albums.iter_mut().enumerate() {
            let todo = matches!(a.state, State::Idle | State::Failed(_));
            if a.checked && a.searchable() && todo {
                a.state = State::Queued;
                wanted.push((i, a.artist.clone(), a.title.clone(), a.paths.len() as u32));
            }
        }
        if wanted.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (stop, ctx) = (cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            for (i, artist, title, tracks) in wanted {
                if stop.load(Ordering::Relaxed) || tx.send((i, State::Searching)).is_err() {
                    return;
                }
                ctx.request_repaint();
                let found = find_cover(&artist, &title, tracks);
                if stop.load(Ordering::Relaxed) || tx.send((i, found)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        self.search = Some(Search { rx, cancel });
    }

    fn found(&self) -> impl Iterator<Item = (&Missing, &Cover)> {
        self.albums
            .iter()
            .filter(|a| a.checked)
            .filter_map(|a| match &a.state {
                State::Found(c, _) => Some((a, c)),
                _ => None,
            })
    }
}

/// Cherche l'album sur MusicBrainz et prend la pochette de la première sortie qui en a une, en
/// commençant par celles qui ont le même nombre de pistes.
fn find_cover(artist: &str, title: &str, tracks: u32) -> State {
    // « Artiste inconnu » ou une compilation : le titre seul donne de meilleurs résultats.
    let various = ["various artists", "various", "artistes divers", "va"];
    let artist = if artist == unknown_artist() || various.contains(&artist.to_lowercase().as_str()) {
        ""
    } else {
        artist
    };
    let hits = match musicbrainz::search(artist, title) {
        Ok(h) => h,
        Err(e) => return State::Failed(e),
    };
    let mut order: Vec<&musicbrainz::Hit> = hits.iter().collect();
    order.sort_by_key(|h| h.tracks != tracks);
    for h in order.into_iter().take(4) {
        match musicbrainz::cover(&h.id) {
            Ok(Some(data)) => {
                if let Ok(cover) = cover_from_bytes(data) {
                    let year: String = h.date.chars().take(4).collect();
                    let source = [h.artist.as_str(), h.title.as_str(), year.as_str()]
                        .iter()
                        .filter(|s| !s.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join(" · ");
                    return State::Found(cover, source);
                }
            }
            Ok(None) => {}
            Err(e) => return State::Failed(e),
        }
    }
    State::NotFound
}

impl SterMusic {
    /// Repère les albums dont le premier morceau n'a ni pochette intégrée ni image de dossier
    /// (c'est aussi ce morceau qui donne sa pochette à l'album dans la bibliothèque).
    pub(super) fn start_cover_scan(&mut self, ctx: &egui::Context) {
        let tool = &mut self.lib_tools.covers;
        if tool.busy() {
            return;
        }
        let albums: Vec<(String, String, Vec<PathBuf>)> = self
            .lib
            .albums
            .iter()
            .map(|a| {
                let paths = a
                    .tracks
                    .iter()
                    .filter(|&&t| self.lib.tracks[t].is_plain_file())
                    .map(|&t| self.lib.tracks[t].path.clone())
                    .collect();
                (a.title.clone(), a.artist.clone(), paths)
            })
            .collect();
        tool.albums.clear();
        tool.scan_progress = (0, albums.len());
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let total = albums.len();
            let mut missing = Vec::new();
            for (n, (title, artist, paths)) in albums.into_iter().enumerate() {
                if n % 10 == 0 {
                    if tx.send(ScanMsg::Progress(n, total)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
                if paths.first().is_some_and(|p| !crate::covers::has_cover(p)) {
                    let mut m = Missing {
                        title,
                        artist,
                        paths,
                        checked: true,
                        state: State::Idle,
                        tex: None,
                    };
                    m.checked = m.searchable();
                    missing.push(m);
                }
            }
            let _ = tx.send(ScanMsg::Done(missing));
            ctx.request_repaint();
        });
        tool.scan = Some(rx);
    }

    pub(super) fn cover_dialog(&mut self, ctx: &egui::Context) {
        let tool = &mut self.lib_tools.covers;
        let (mut close, mut search, mut stop, mut apply) = (false, false, false, false);
        let mut pick: Option<usize> = None;
        let width = (ctx.screen_rect().width() - 100.0).clamp(600.0, 760.0);
        let list_h = (ctx.screen_rect().height() - 260.0).clamp(160.0, 460.0);
        let modal = crate::widgets::modal(ctx, egui::Id::new("missing_covers"))
            .frame(lib_tools::modal_frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(width);
                let detail = if tool.scan.is_some() {
                    String::new()
                } else {
                    plural(tool.albums.len(), "album", "albums")
                };
                lib_tools::title(ui, Icon::Images, tr("Albums sans pochette"), &detail);
                ui.add_space(12.0);
                if tool.scan.is_some() {
                    let (n, total) = tool.scan_progress;
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(16.0).color(theme::accent_light()));
                        ui.label(
                            egui::RichText::new(trf!(
                                "Examen de la bibliothèque… {n} / {total} albums",
                                n,
                                total
                            ))
                            .color(theme::TEXT),
                        );
                    });
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(n as f32 / total.max(1) as f32).desired_height(6.0));
                } else if tool.albums.is_empty() {
                    ui.label(
                        egui::RichText::new(tr("Tous les albums de la bibliothèque ont une pochette."))
                            .color(theme::TEXT_2),
                    );
                } else {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(tr(
                                "Coche les albums à compléter, lance la recherche, puis vérifie les \
                                 images avant de les intégrer aux fichiers.",
                            ))
                            .color(theme::TEXT_2),
                        );
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if widgets::link(ui, tr("Tout cocher")).clicked() {
                            for a in &mut tool.albums {
                                a.checked = a.searchable() || matches!(a.state, State::Found(..));
                            }
                        }
                        if widgets::link(ui, tr("Tout décocher")).clicked() {
                            tool.albums.iter_mut().for_each(|a| a.checked = false);
                        }
                    });
                    ui.add_space(6.0);
                    pick = album_list(ui, tool, width, list_h);
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    let found = tool.found().count();
                    let files: usize = tool.found().map(|(a, _)| a.paths.len()).sum();
                    if found > 0 {
                        ui.label(
                            egui::RichText::new(trf!(
                                "{0} pour {1}",
                                plural(found, "pochette prête", "pochettes prêtes"),
                                plural(files, "fichier", "fichiers")
                            ))
                            .color(theme::TEXT_2),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let searching = tool.search.is_some();
                        ui.scope(|ui| {
                            if found == 0 || searching {
                                ui.disable();
                                ui.set_opacity(0.5);
                            }
                            if widgets::primary_button(ui, None, tr("Intégrer aux fichiers"))
                                .on_hover_text(tr(
                                    "Écrit chaque pochette dans tous les morceaux de son album",
                                ))
                                .clicked()
                            {
                                apply = true;
                            }
                        });
                        if searching {
                            if widgets::secondary_button(ui, Some(Icon::Stop), tr("Arrêter")).clicked() {
                                stop = true;
                            }
                            ui.add(egui::Spinner::new().size(16.0).color(theme::accent_light()));
                        } else {
                            let todo = tool
                                .albums
                                .iter()
                                .filter(|a| {
                                    a.checked
                                        && a.searchable()
                                        && matches!(a.state, State::Idle | State::Failed(_))
                                })
                                .count();
                            if todo > 0
                                && widgets::secondary_button(
                                    ui,
                                    Some(Icon::Globe),
                                    &trf!("Chercher en ligne ({todo})", todo),
                                )
                                .on_hover_text(tr(
                                    "MusicBrainz et le Cover Art Archive, un album par seconde",
                                ))
                                .clicked()
                            {
                                search = true;
                            }
                        }
                        if widgets::secondary_button(ui, None, tr("Fermer")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if search {
            tool.start_search(ctx);
        }
        if stop {
            tool.stop();
        }
        if let Some(i) = pick {
            if let Some(file) = rfd::FileDialog::new()
                .set_title(tr("Choisir une pochette"))
                .add_filter(tr("Images"), &["jpg", "jpeg", "png", "webp", "bmp", "gif"])
                .pick_file()
            {
                let a = &mut tool.albums[i];
                match cover_from_file(&file) {
                    Ok(c) => {
                        a.state = State::Found(c, tr("image choisie sur le disque").into());
                        a.checked = true;
                    }
                    Err(e) => a.state = State::Failed(e),
                }
                a.tex = None;
            }
        }
        if apply {
            let jobs: Vec<Job> = tool
                .found()
                .flat_map(|(a, c)| {
                    a.paths
                        .iter()
                        .map(|p| (p.clone(), Vec::new(), Some(Some(c.clone()))))
                })
                .collect();
            if self.start_tag_jobs(ctx, jobs) {
                // Les albums complétés quittent la liste ; l'avancement est dans l'éditeur.
                self.lib_tools
                    .covers
                    .albums
                    .retain(|a| !(a.checked && matches!(a.state, State::Found(..))));
                self.lib_tools.open = None;
                self.toast(tr("Les pochettes s'intègrent aux fichiers en arrière-plan…"));
            }
        } else if close || modal.should_close() {
            self.lib_tools.open = None;
        }
    }
}

/// Liste des albums sans pochette ; renvoie celui pour lequel on veut choisir une image.
fn album_list(ui: &mut egui::Ui, tool: &mut CoverTool, width: f32, height: f32) -> Option<usize> {
    const ROW: f32 = 68.0;
    let mut pick = None;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        theme::white(0x05),
        egui::Stroke::new(1.0_f32, theme::white(0x0E)),
        egui::StrokeKind::Inside,
    );
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(4.0)));
    inner.spacing_mut().item_spacing.y = 0.0;
    let count = tool.albums.len();
    egui::ScrollArea::vertical()
        .id_salt("missing_covers")
        .auto_shrink(false)
        .show_rows(&mut inner, ROW, count, |ui, range| {
            for i in range {
                let a = &mut tool.albums[i];
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                if i % 2 == 1 {
                    ui.painter()
                        .rect_filled(r, CornerRadius::same(6), theme::white(0x03));
                }
                // Case à cocher, pochette, textes, puis le bouton « image du disque ».
                let check = Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(22.0, 22.0));
                widgets::place(ui, check, |ui| {
                    ui.add(egui::Checkbox::without_text(&mut a.checked))
                });
                let art = Rect::from_min_size(pos2(r.left() + 42.0, r.top() + 8.0), vec2(52.0, 52.0));
                match &a.state {
                    State::Found(c, _) => {
                        paint_cover_texture(ui, &mut a.tex, c, art);
                    }
                    _ => widgets::cover(ui.painter(), art, None, 6, &a.title),
                }
                let x = art.right() + 14.0;
                let tw = r.right() - x - 52.0;
                let g = widgets::galley(ui, &a.title, theme::semibold(14.0), theme::TEXT, tw);
                ui.painter().galley(pos2(x, r.top() + 8.0), g, theme::TEXT);
                let line = format!("{} · {}", a.artist, plural(a.paths.len(), "morceau", "morceaux"));
                let g = widgets::galley(ui, &line, FontId::proportional(12.5), theme::TEXT_2, tw);
                ui.painter().galley(pos2(x, r.top() + 28.0), g, theme::TEXT_2);
                let (status, color) = match &a.state {
                    State::Idle if !a.searchable() => (
                        tr("Album sans nom : choisis une image, ou complète d'abord ses tags").to_owned(),
                        theme::TEXT_3,
                    ),
                    State::Idle => (String::new(), theme::TEXT_3),
                    State::Queued => (tr("En attente…").to_owned(), theme::TEXT_3),
                    State::Searching => (tr("Recherche…").to_owned(), theme::accent_light()),
                    State::Found(_, source) => (trf!("Trouvée : {source}", source), theme::accent_light()),
                    State::NotFound => (tr("Aucune pochette trouvée en ligne").to_owned(), WARN),
                    State::Failed(e) => (e.clone(), WARN),
                };
                let g = widgets::galley(ui, &status, FontId::proportional(12.0), color, tw);
                ui.painter().galley(pos2(x, r.top() + 46.0), g, color);
                if matches!(a.state, State::Searching) {
                    egui::Spinner::new()
                        .color(theme::accent_light())
                        .paint_at(ui, Rect::from_center_size(art.center(), vec2(18.0, 18.0)));
                }
                let button = Rect::from_center_size(pos2(r.right() - 26.0, r.center().y), vec2(34.0, 34.0));
                let resp = widgets::place(ui, button, |ui| {
                    widgets::icon_button(ui, Icon::Image, tr("Choisir une image sur le disque"), 34.0, 16.0)
                });
                if resp.clicked() {
                    pick = Some(i);
                }
            }
        });
    if count == 0 {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            tr("Plus aucun album à compléter"),
            FontId::proportional(13.0),
            theme::TEXT_3,
        );
    }
    pick
}
