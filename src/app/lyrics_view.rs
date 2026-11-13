//! Lyrics in "Now Playing": they scroll by themselves, with the sung line highlighted; clicking
//! a line seeks playback there.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use eframe::egui::{self, pos2, vec2, Align2, FontId, Rect, Sense};

use super::{Action, SterMusic};
use crate::i18n::tr;
use crate::lyrics::{Lyrics, Source};
use crate::{anim, theme};

#[derive(Default)]
pub struct LyricsState {
    /// Track whose lyrics we have (or are looking for).
    path: Option<PathBuf>,
    rx: Option<Receiver<(PathBuf, Option<Lyrics>)>>,
    pub lyrics: Option<Lyrics>,
}

impl LyricsState {
    /// Forgets the stored lyrics (setting changed, .lrc file added…).
    pub fn forget(&mut self) {
        *self = Self::default();
    }
}

impl SterMusic {
    /// Looks up the lyrics of the current track if it changed (called while "Now Playing" is shown:
    /// nothing is requested for tracks nobody is looking at).
    pub(super) fn want_lyrics(&mut self, ctx: &egui::Context) {
        let Some(i) = self.current() else { return };
        let track = self.lib.tracks[i].clone();
        if self.lyrics.path.as_ref() == Some(&track.path) {
            return;
        }
        let (tx, rx) = channel();
        let online = self.settings.lyrics_online;
        let ctx = ctx.clone();
        self.lyrics = LyricsState {
            path: Some(track.path.clone()),
            rx: Some(rx),
            lyrics: None,
        };
        std::thread::spawn(move || {
            let found = crate::lyrics::find(&track, online);
            let _ = tx.send((track.path, found));
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_lyrics(&mut self, _ctx: &egui::Context) {
        let Some(rx) = &self.lyrics.rx else { return };
        match rx.try_recv() {
            Ok((path, found)) => {
                self.lyrics.rx = None;
                if self.lyrics.path.as_ref() == Some(&path) {
                    self.lyrics.lyrics = found;
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.lyrics.rx = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Lyrics in `rect`: synced lyrics scroll with the music; otherwise, a text scrolled by hand.
    pub(super) fn lyrics_panel(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let Some(lyrics) = self.lyrics.lyrics.clone() else {
            return;
        };
        let credit = (lyrics.source == Source::Online).then(|| tr("Paroles : LRCLIB"));
        let panel = if credit.is_some() {
            Rect::from_min_max(rect.min, rect.max - vec2(0.0, 22.0))
        } else {
            rect
        };
        if let Some(text) = credit {
            ui.painter().text(
                pos2(panel.left(), panel.bottom() + 6.0),
                Align2::LEFT_TOP,
                text,
                FontId::proportional(11.5),
                theme::TEXT_3,
            );
        }
        if !lyrics.synced {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(panel));
            egui::ScrollArea::vertical()
                .id_salt(("plain_lyrics", self.lyrics.path.clone()))
                .auto_shrink(false)
                .show(&mut child, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for line in &lyrics.lines {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(if line.text.is_empty() { " " } else { &line.text })
                                    .size(17.0)
                                    .color(theme::TEXT_2),
                            )
                            .wrap(),
                        );
                    }
                    ui.add_space(24.0);
                });
            return;
        }

        let pos = self
            .player
            .as_ref()
            .map(|p| p.position().as_secs_f32())
            .unwrap_or(0.0);
        let active = lyrics.current(pos);
        let w = panel.width();
        // Layout of every line (the current line is larger).
        let mut layout = Vec::with_capacity(lyrics.lines.len());
        let mut y = 0.0;
        for (k, line) in lyrics.lines.iter().enumerate() {
            let is_active = Some(k) == active;
            let font = if is_active {
                theme::semibold(23.0)
            } else {
                FontId::proportional(19.0)
            };
            // Empty line: an instrumental break, shown by a note.
            let text = if line.text.is_empty() {
                "♪"
            } else {
                line.text.as_str()
            };
            let g = ui.fonts(|f| f.layout(text.to_owned(), font, theme::TEXT, w));
            let h = g.size().y;
            layout.push((y, h, g));
            y += h + 14.0;
        }
        // The sung line sits at a third of the height; scrolling glides smoothly.
        let anchor = panel.height() * 0.36;
        let target = match active {
            Some(k) => layout[k].0 + layout[k].1 / 2.0 - anchor,
            None => -anchor + 20.0,
        };
        let scroll = anim::value(
            ui.ctx(),
            egui::Id::new(("lyrics_scroll", self.lyrics.path.clone())),
            target,
            0.45,
            anim::Easing::OutExpo,
        );
        let painter = ui.painter().with_clip_rect(panel);
        let fade = 48.0;
        let mut seek = None;
        for (k, (ly, h, g)) in layout.into_iter().enumerate() {
            let top = panel.top() + ly - scroll;
            if top + h < panel.top() || top > panel.bottom() {
                continue;
            }
            // Fade towards the top and bottom of the panel.
            let edge = ((top - panel.top()).min(panel.bottom() - top - h) / fade).clamp(0.0, 1.0);
            let color = match active {
                Some(a) if k == a => theme::TEXT,
                Some(a) if k < a => theme::TEXT_3,
                _ => theme::TEXT_2,
            };
            let r = Rect::from_min_size(pos2(panel.left(), top), vec2(w, h));
            let resp = ui.interact(r.intersect(panel), ui.id().with(("lyric", k)), Sense::click());
            let color = if resp.hovered() {
                theme::accent_light()
            } else {
                color
            };
            painter.galley_with_override_text_color(r.min, g, color.gamma_multiply(0.25 + 0.75 * edge));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                seek = lyrics.lines[k].time;
            }
        }
        if let Some(t) = seek {
            self.actions.push(Action::SeekTo(t));
        }
        if self.is_playing() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
}
