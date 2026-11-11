//! Import depuis MusicBrainz, à la Mp3tag : on cherche l'album de la sélection, on choisit la
//! bonne sortie, on vérifie l'association pistes ↔ fichiers, puis les infos remplissent les
//! champs de l'éditeur (rien n'est écrit avant « Enregistrer »).

use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense,
};

use super::tag_editor::{cover_from_bytes, file_name, paint_cover_texture, CoverSlot, TagEditor};
use super::SterMusic;
use crate::i18n::{n_, tr};
use crate::icons::{self, Icon};
use crate::library::{fmt_duration, plural};
use crate::musicbrainz::{self, Hit, Release};
use crate::tags::{Cover, Field};
use crate::trf;
use crate::{theme, widgets};

type Task<T> = Option<Receiver<Result<T, String>>>;

const OPTIONS: [&str; 6] = [
    n_("Titres"),
    n_("Artistes"),
    n_("Album et année"),
    n_("Numéros"),
    n_("Genre"),
    n_("Pochette"),
];

pub struct MbImport {
    /// Fichiers à associer aux pistes, dans l'ordre (disque, piste) s'ils sont numérotés.
    targets: Vec<usize>,
    artist: String,
    album: String,
    search: Task<Vec<Hit>>,
    hits: Vec<Hit>,
    searched: bool,
    chosen: Option<String>,
    fetch: Task<(Release, Option<Cover>)>,
    release: Option<Release>,
    cover: Option<Cover>,
    cover_tex: CoverSlot,
    error: Option<String>,
    options: [bool; 6],
}

/// Valeur la plus fréquente (non vide) d'un champ parmi les fichiers.
fn common(ed: &TagEditor, ks: &[usize], f: Field) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for &k in ks {
        let v = ed.values[k][f.index()].as_str();
        if v.is_empty() {
            continue;
        }
        match counts.iter_mut().find(|c| c.0 == v) {
            Some(c) => c.1 += 1,
            None => counts.push((v, 1)),
        }
    }
    counts
        .into_iter()
        .max_by_key(|c| c.1)
        .map(|c| c.0.to_owned())
        .unwrap_or_default()
}

impl MbImport {
    pub fn new(ed: &TagEditor, scope: Vec<usize>) -> Self {
        let num = |k: usize, f: Field| ed.values[k][f.index()].parse::<u32>().ok();
        let mut targets = scope;
        if targets.iter().all(|&k| num(k, Field::Track).is_some()) {
            targets.sort_by_key(|&k| {
                (
                    num(k, Field::Disc).unwrap_or(1),
                    num(k, Field::Track).unwrap_or(0),
                )
            });
        }
        let mut artist = common(ed, &targets, Field::AlbumArtist);
        if artist.is_empty() {
            artist = common(ed, &targets, Field::Artist);
        }
        let album = common(ed, &targets, Field::Album);
        let mut me = Self {
            targets,
            artist,
            album,
            search: None,
            hits: Vec::new(),
            searched: false,
            chosen: None,
            fetch: None,
            release: None,
            cover: None,
            cover_tex: None,
            error: None,
            options: [true; 6],
        };
        if !me.album.is_empty() || !me.artist.is_empty() {
            me.start_search();
        }
        me
    }

    pub fn busy(&self) -> bool {
        self.search.is_some() || self.fetch.is_some()
    }

    fn start_search(&mut self) {
        let (artist, album) = (self.artist.clone(), self.album.clone());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(musicbrainz::search(&artist, &album));
        });
        self.search = Some(rx);
        self.error = None;
    }

    fn choose(&mut self, id: String) {
        if self.chosen.as_ref() == Some(&id) {
            return;
        }
        self.chosen = Some(id.clone());
        self.release = None;
        self.cover = None;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let result = musicbrainz::release(&id).map(|r| {
                // Pas de pochette n'est pas une erreur : l'import continue sans.
                let cover = musicbrainz::cover(&id)
                    .ok()
                    .flatten()
                    .and_then(|data| cover_from_bytes(data).ok());
                (r, cover)
            });
            let _ = tx.send(result);
        });
        self.fetch = Some(rx);
        self.error = None;
    }

    pub fn poll(&mut self) {
        if let Some(rx) = &self.search {
            if let Ok(r) = rx.try_recv() {
                self.search = None;
                self.searched = true;
                match r {
                    Ok(hits) => {
                        self.hits = hits;
                        // Le premier résultat qui a autant de pistes que de fichiers est proposé d'office.
                        let n = self.targets.len() as u32;
                        let best = self
                            .hits
                            .iter()
                            .find(|h| h.tracks == n)
                            .or_else(|| self.hits.first())
                            .map(|h| h.id.clone());
                        if let Some(id) = best {
                            self.chosen = None;
                            self.choose(id);
                        }
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
        if let Some(rx) = &self.fetch {
            if let Ok(r) = rx.try_recv() {
                self.fetch = None;
                match r {
                    Ok((release, cover)) => {
                        self.release = Some(release);
                        self.cover = cover;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
    }

    /// Remplit les champs de l'éditeur ; renvoie le nombre de fichiers touchés.
    fn apply(&self, ed: &mut TagEditor) -> usize {
        let Some(rel) = &self.release else { return 0 };
        let [titles, artists, album, numbers, genre, cover] = self.options;
        for (i, &k) in self.targets.iter().enumerate() {
            let v = &mut ed.values[k];
            let mut set = |f: Field, s: &str| v[f.index()] = s.to_owned();
            // Champs de l'album : pour tous les fichiers.
            if album {
                set(Field::Album, &rel.title);
                set(Field::AlbumArtist, &rel.artist);
                if !rel.year.is_empty() {
                    set(Field::Year, &rel.year);
                }
            }
            if genre && !rel.genre.is_empty() {
                set(Field::Genre, &rel.genre);
            }
            // Champs de la piste : seulement pour les fichiers associés à une piste.
            if let Some(t) = rel.tracks.get(i) {
                if titles {
                    set(Field::Title, &t.title);
                }
                if artists {
                    set(
                        Field::Artist,
                        if t.artist.is_empty() {
                            &rel.artist
                        } else {
                            &t.artist
                        },
                    );
                }
                if numbers {
                    set(Field::Track, &t.position.to_string());
                    set(Field::TrackTotal, &t.total.to_string());
                    if t.discs > 1 {
                        set(Field::Disc, &t.disc.to_string());
                        set(Field::DiscTotal, &t.discs.to_string());
                    }
                }
            }
            if let (true, Some(c)) = (cover, &self.cover) {
                ed.set_file_cover(k, Some(c.clone()));
            }
        }
        self.targets.len()
    }
}

impl SterMusic {
    pub(super) fn mb_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut mb) = self.tag_editor.mb.take() else {
            return;
        };
        let mut close = false;
        let mut apply = false;
        let width = (ctx.screen_rect().width() - 100.0).clamp(700.0, 1000.0);
        let modal = crate::widgets::modal(ctx, egui::Id::new("mb_import"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .inner_margin(24.0)
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                    icons::paint_centered(ui.painter(), Icon::Globe, r.center(), 20.0, theme::accent_light());
                    ui.label(RichText::new(tr("Importer depuis MusicBrainz")).font(theme::semibold(20.0)));
                    ui.label(
                        RichText::new(plural(mb.targets.len(), "fichier", "fichiers")).color(theme::TEXT_3),
                    );
                });
                ui.add_space(14.0);

                // Recherche.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let enter = |r: &egui::Response, ui: &egui::Ui| {
                        r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    };
                    let a = widgets::text_field(
                        ui,
                        egui::Id::new("mb_artist"),
                        &mut mb.artist,
                        tr("Artiste"),
                        260.0,
                        1,
                    );
                    let b = widgets::text_field(
                        ui,
                        egui::Id::new("mb_album"),
                        &mut mb.album,
                        tr("Album"),
                        300.0,
                        1,
                    );
                    let go = widgets::primary_button(ui, Some(Icon::Search), tr("Rechercher")).clicked();
                    if go || enter(&a, ui) || enter(&b, ui) {
                        mb.start_search();
                    }
                    if mb.busy() {
                        ui.spinner();
                    }
                });
                ui.add_space(14.0);

                let body_h = 380.0;
                let list_w = 340.0;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    ui.vertical(|ui| {
                        ui.set_width(list_w);
                        ui.set_height(body_h);
                        results(ui, &mut mb, list_w);
                    });
                    ui.vertical(|ui| {
                        ui.set_width(width - list_w - 18.0);
                        ui.set_height(body_h);
                        detail(ui, &mut mb, &self.tag_editor);
                    });
                });
                if let Some(e) = &mb.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).color(Color32::from_rgb(0xFF, 0x99, 0x99)));
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new(tr("Importer :")).color(theme::TEXT_2));
                    for (i, name) in OPTIONS.iter().enumerate() {
                        let available = i != 5 || mb.cover.is_some();
                        ui.scope(|ui| {
                            if !available {
                                ui.disable();
                                ui.multiply_opacity(0.4);
                            }
                            if super::views::segment(ui, tr(name), mb.options[i] && available).clicked() {
                                mb.options[i] = !mb.options[i];
                            }
                        });
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.scope(|ui| {
                            if mb.release.is_none() {
                                ui.disable();
                                ui.multiply_opacity(0.5);
                            }
                            if widgets::primary_button(ui, None, tr("Appliquer")).clicked() {
                                apply = true;
                            }
                        });
                        if widgets::secondary_button(ui, None, tr("Annuler")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if modal.should_close() {
            close = true;
        }
        if apply {
            let n = mb.apply(&mut self.tag_editor);
            self.toast(trf!(
                "Infos MusicBrainz appliquées à {0} : vérifie, puis enregistre",
                plural(n, "fichier", "fichiers")
            ));
        } else if !close {
            self.tag_editor.mb = Some(mb);
        }
    }
}

fn results(ui: &mut egui::Ui, mb: &mut MbImport, w: f32) {
    if mb.hits.is_empty() {
        let text = if mb.search.is_some() {
            tr("Recherche…")
        } else if mb.searched {
            tr("Aucun album trouvé. Essaie avec moins de mots, ou sans l'artiste.")
        } else {
            tr("Indique l'artiste et l'album, puis lance la recherche.")
        };
        ui.label(RichText::new(text).color(theme::TEXT_3));
        return;
    }
    let n = mb.targets.len() as u32;
    let mut pick = None;
    egui::ScrollArea::vertical()
        .id_salt("mb_hits")
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for h in &mb.hits {
                let (r, resp) = ui.allocate_exact_size(vec2(w - 10.0, 58.0), Sense::click());
                let chosen = mb.chosen.as_ref() == Some(&h.id);
                if chosen {
                    ui.painter()
                        .rect_filled(r, CornerRadius::same(6), theme::alpha(theme::accent(), 0x38));
                }
                widgets::paint_interaction(ui, r, 6, &resp);
                let x = r.left() + 10.0;
                let tw = r.width() - 20.0;
                let g = widgets::galley(ui, &h.title, theme::semibold(13.5), theme::TEXT, tw);
                ui.painter().galley(pos2(x, r.top() + 6.0), g, theme::TEXT);
                let year: String = h.date.chars().take(4).collect();
                let line2 = [h.artist.as_str(), year.as_str(), h.country.as_str()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" · ");
                let g = widgets::galley(ui, &line2, FontId::proportional(12.0), theme::TEXT_2, tw);
                ui.painter().galley(pos2(x, r.top() + 24.0), g, theme::TEXT_2);
                let line3 = [
                    h.format.clone(),
                    plural(h.tracks as usize, "piste", "pistes"),
                    h.extra.clone(),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
                let color = if h.tracks == n {
                    theme::accent_light()
                } else {
                    theme::TEXT_3
                };
                let g = widgets::galley(ui, &line3, FontId::proportional(12.0), color, tw);
                ui.painter().galley(pos2(x, r.top() + 40.0), g, color);
                if resp.clicked() {
                    pick = Some(h.id.clone());
                }
            }
        });
    if let Some(id) = pick {
        mb.choose(id);
    }
}

fn detail(ui: &mut egui::Ui, mb: &mut MbImport, ed: &TagEditor) {
    let Some(rel) = &mb.release else {
        if mb.fetch.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tr("Chargement de l'album…")).color(theme::TEXT_2));
            });
        }
        return;
    };
    // En-tête de l'album.
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 84.0), Sense::hover());
    let art = Rect::from_min_size(row.min, vec2(84.0, 84.0));
    match &mb.cover {
        Some(c) => {
            paint_cover_texture(ui, &mut mb.cover_tex, c, art);
        }
        None => widgets::cover(ui.painter(), art, None, 6, ""),
    }
    let x = art.right() + 14.0;
    let tw = row.right() - x;
    let g = widgets::galley(ui, &rel.title, theme::semibold(17.0), theme::TEXT, tw);
    ui.painter().galley(pos2(x, row.top() + 6.0), g, theme::TEXT);
    let g = widgets::galley(ui, &rel.artist, FontId::proportional(14.0), theme::TEXT, tw);
    ui.painter().galley(pos2(x, row.top() + 32.0), g, theme::TEXT);
    let mut info = vec![plural(rel.tracks.len(), "piste", "pistes")];
    if !rel.genre.is_empty() {
        info.insert(0, rel.genre.clone());
    }
    if !rel.year.is_empty() {
        info.insert(0, rel.year.clone());
    }
    if mb.cover.is_none() {
        info.push(tr("sans pochette").into());
    }
    ui.painter().text(
        pos2(x, row.top() + 56.0),
        Align2::LEFT_TOP,
        info.join(" · "),
        FontId::proportional(12.5),
        theme::TEXT_2,
    );
    ui.add_space(10.0);

    let (nt, nf) = (rel.tracks.len(), mb.targets.len());
    let note = if nt == nf {
        trf!("{nt} pistes ↔ {nf} fichiers, associés dans l'ordre :", nt, nf)
    } else {
        trf!(
            "{0} pour {1} : seuls les {2} premiers sont associés, dans l'ordre.",
            plural(nt, "piste", "pistes"),
            plural(nf, "fichier", "fichiers"),
            nt.min(nf)
        )
    };
    ui.label(RichText::new(note).small().color(if nt == nf {
        theme::TEXT_2
    } else {
        Color32::from_rgb(0xFF, 0xC0, 0x70)
    }));
    ui.add_space(4.0);

    let w = ui.available_width() - 10.0;
    egui::ScrollArea::vertical()
        .id_salt("mb_tracks")
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in 0..nt.max(nf) {
                let (r, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
                if i % 2 == 1 {
                    ui.painter()
                        .rect_filled(r, CornerRadius::same(4), theme::white(0x04));
                }
                let y = r.center().y;
                let half = (w - 30.0) / 2.0;
                let file = mb.targets.get(i).map(|&k| &ed.files[k]);
                let track = rel.tracks.get(i);
                // Durées très différentes : probablement le mauvais fichier en face.
                let off = match (file, track.and_then(|t| t.length_ms)) {
                    (Some(f), Some(l)) if f.duration_ms > 0 => (f.duration_ms as i64 - l as i64).abs() > 5000,
                    _ => false,
                };
                let warn = Color32::from_rgb(0xFF, 0xC0, 0x70);
                match track {
                    Some(t) => {
                        let num = if t.discs > 1 {
                            format!("{}-{}", t.disc, t.position)
                        } else {
                            t.position.to_string()
                        };
                        ui.painter().text(
                            pos2(r.left() + 26.0, y),
                            Align2::RIGHT_CENTER,
                            num,
                            FontId::proportional(12.0),
                            theme::TEXT_3,
                        );
                        let g = widgets::galley(
                            ui,
                            &t.title,
                            FontId::proportional(13.0),
                            theme::TEXT,
                            half - 80.0,
                        );
                        ui.painter()
                            .galley(pos2(r.left() + 34.0, y - g.size().y / 2.0), g, theme::TEXT);
                        if let Some(l) = t.length_ms {
                            ui.painter().text(
                                pos2(r.left() + half, y),
                                Align2::RIGHT_CENTER,
                                fmt_duration(Duration::from_millis(l)),
                                FontId::proportional(12.0),
                                if off { warn } else { theme::TEXT_3 },
                            );
                        }
                    }
                    None => {
                        ui.painter().text(
                            pos2(r.left() + 34.0, y),
                            Align2::LEFT_CENTER,
                            "—",
                            FontId::proportional(12.0),
                            theme::TEXT_3,
                        );
                    }
                }
                ui.painter().text(
                    pos2(r.left() + half + 15.0, y),
                    Align2::CENTER_CENTER,
                    "←",
                    FontId::proportional(13.0),
                    theme::TEXT_3,
                );
                let fx = r.left() + half + 30.0;
                match file {
                    Some(f) => {
                        let g = widgets::galley(
                            ui,
                            &file_name(&f.path),
                            FontId::proportional(12.5),
                            theme::TEXT_2,
                            half - 60.0,
                        );
                        ui.painter()
                            .galley(pos2(fx, y - g.size().y / 2.0), g, theme::TEXT_2);
                        ui.painter().text(
                            pos2(r.right() - 4.0, y),
                            Align2::RIGHT_CENTER,
                            fmt_duration(Duration::from_millis(f.duration_ms)),
                            FontId::proportional(12.0),
                            if off { warn } else { theme::TEXT_3 },
                        );
                    }
                    None => {
                        ui.painter().text(
                            pos2(fx, y),
                            Align2::LEFT_CENTER,
                            tr("pas de fichier"),
                            FontId::proportional(12.0),
                            theme::TEXT_3,
                        );
                    }
                }
            }
        });
}
