//! Ambience: what changes smoothly from one track to the next.
//!
//! - the visualizer colors move from those of one album art to those of the next;
//! - the adaptive theme (option) does the same with the colors of the whole interface; without
//!   album art, it goes back to the chosen theme;
//! - album art ("Now Playing", its blurred background) cross-fades: the old one stays on screen
//!   until the new one is loaded, without going through an empty frame.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, TextureHandle};

use super::SterMusic;
use crate::theme;

/// Duration of a color change.
const COLOR_FADE: f64 = 0.9;
/// Duration of an album art cross-fade.
pub const COVER_FADE: f64 = 0.45;
/// Maximum wait for the new album art before switching anyway.
const COVER_WAIT: f64 = 0.35;

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `N` colors gliding towards a target.
pub struct ColorFade<const N: usize> {
    from: [Color32; N],
    to: [Color32; N],
    since: f64,
    started: bool,
}

impl<const N: usize> Default for ColorFade<N> {
    fn default() -> Self {
        Self {
            from: [Color32::BLACK; N],
            to: [Color32::BLACK; N],
            since: 0.0,
            started: false,
        }
    }
}

impl<const N: usize> ColorFade<N> {
    /// New target: start from what is on screen (the first time, jump straight there).
    pub fn set_target(&mut self, now: f64, target: [Color32; N]) {
        if !self.started {
            self.started = true;
            self.from = target;
            self.to = target;
            self.since = now - COLOR_FADE;
        } else if target != self.to {
            self.from = self.value(now);
            self.to = target;
            self.since = now;
        }
    }

    pub fn value(&self, now: f64) -> [Color32; N] {
        let t = ease(((now - self.since) / COLOR_FADE) as f32);
        std::array::from_fn(|i| theme::mix(self.from[i], self.to[i], t))
    }

    pub fn fading(&self, now: f64) -> bool {
        self.started && now - self.since < COLOR_FADE
    }
}

/// An image (album art, blurred background) that cross-fades into the next one.
#[derive(Default)]
pub struct TexFade {
    cur: Option<(PathBuf, Option<TextureHandle>)>,
    prev: Option<Option<TextureHandle>>,
    since: f64,
    waiting: Option<f64>,
}

impl TexFade {
    /// `wanted`: the track to show; `loaded`: its image if it is loaded (`Some(None)`: loaded, it
    /// has none). Returns true at the moment we switch to this track.
    pub fn update(&mut self, now: f64, wanted: &Path, loaded: Option<Option<TextureHandle>>) -> bool {
        match &mut self.cur {
            Some((path, tex)) if path == wanted => {
                // Image arrived after the wait: it takes its place.
                if tex.is_none() {
                    if let Some(Some(t)) = loaded {
                        *tex = Some(t);
                    }
                }
                self.waiting = None;
                false
            }
            None => {
                self.cur = Some((wanted.to_path_buf(), loaded.flatten()));
                self.since = now - COVER_FADE;
                true
            }
            Some(_) => {
                let waited = now - *self.waiting.get_or_insert(now);
                if loaded.is_none() && waited < COVER_WAIT {
                    return false;
                }
                self.prev = self.cur.take().map(|(_, tex)| tex);
                self.cur = Some((wanted.to_path_buf(), loaded.flatten()));
                self.since = now;
                self.waiting = None;
                true
            }
        }
    }

    /// Progress of the cross-fade (0 → 1).
    pub fn t(&self, now: f64) -> f32 {
        ease(((now - self.since) / COVER_FADE) as f32)
    }

    pub fn fading(&self, now: f64) -> bool {
        now - self.since < COVER_FADE || self.waiting.is_some()
    }

    /// The image on screen.
    pub fn current(&self) -> Option<&TextureHandle> {
        self.cur.as_ref().and_then(|(_, t)| t.as_ref())
    }

    /// The image fading out, while the cross-fade lasts (`Some(None)`: a track without an image).
    pub fn previous(&self, now: f64) -> Option<Option<&TextureHandle>> {
        if now - self.since >= COVER_FADE {
            return None;
        }
        self.prev.as_ref().map(|t| t.as_ref())
    }
}

/// Theme colors taken from the palette of an album art: background, accent, second accent,
/// light accent. The background keeps the depth of the chosen theme, tinted by the album art;
/// the accents are brought to a lightness where the white text of the buttons stays readable.
pub fn theme_from_palette(palette: [Color32; 3], base_bg: Color32) -> [Color32; 4] {
    let accent = tone(palette[0], 0.50);
    let mut accent_2 = tone(palette[1], 0.46);
    // Two almost identical accents: the brand gradient would be flat.
    if distance(accent, accent_2) < 40.0 {
        accent_2 = tone(theme::mix(palette[1], palette[2], 0.6), 0.40);
    }
    let accent_light = tone(theme::mix(palette[0], Color32::WHITE, 0.30), 0.74);
    let tint = tone(palette[0], 0.10);
    let bg = theme::mix(base_bg, tint, 0.55);
    [bg, accent, accent_2, accent_light]
}

/// Brings a color to lightness `target` (0 → 1, weighted average of the channels) while keeping its hue.
fn tone(c: Color32, target: f32) -> Color32 {
    let (r, g, b) = (c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0);
    let luma = (0.30 * r + 0.59 * g + 0.11 * b).max(1e-3);
    let k = target / luma;
    let scale = |x: f32| x * k;
    let (mut r, mut g, mut b) = (scale(r), scale(g), scale(b));
    // What goes beyond 1 is rendered by lightening towards white, without changing the hue.
    let max = r.max(g).max(b);
    if max > 1.0 {
        let l = 0.30 * r + 0.59 * g + 0.11 * b;
        let w = ((max - 1.0) / (max - l).max(1e-3)).clamp(0.0, 1.0);
        r = r + (l - r) * w;
        g = g + (l - g) * w;
        b = b + (l - b) * w;
    }
    Color32::from_rgb(
        (r.clamp(0.0, 1.0) * 255.0) as u8,
        (g.clamp(0.0, 1.0) * 255.0) as u8,
        (b.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

fn distance(a: Color32, b: Color32) -> f32 {
    let d = |x: u8, y: u8| (x as f32 - y as f32).powi(2);
    (d(a.r(), b.r()) + d(a.g(), b.g()) + d(a.b(), b.b())).sqrt()
}

/// State of the ambience (see the module).
#[derive(Default)]
pub struct Ambience {
    /// Visualizer colors.
    palette: ColorFade<3>,
    /// Theme colors (adaptive or not).
    theme: ColorFade<4>,
    /// Blurred background of "Now Playing".
    pub backdrop: TexFade,
    /// Album art of "Now Playing".
    pub cover: TexFade,
    /// Direction of the last track change (1: next, −1: previous) and position in the queue.
    pub dir: f32,
    queue_pos: Option<usize>,
}

impl SterMusic {
    /// Every frame: glides the colors towards those of the current track.
    pub(super) fn update_ambience(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.queue_pos != self.ambience.queue_pos {
            let back = matches!((self.queue_pos, self.ambience.queue_pos), (Some(a), Some(b)) if a < b);
            self.ambience.dir = if back { -1.0 } else { 1.0 };
            self.ambience.queue_pos = self.queue_pos;
        }
        let base = theme::current();
        let base_colors = [base.bg, base.accent, base.accent_2, base.accent_light];
        let base_palette = [base.accent, base.accent_2, base.accent_light];
        // Album art of the current track: `None` while it loads (the previous colors are kept),
        // `Some(None)` if it has none (back to the chosen theme).
        let cover: Option<Option<[Color32; 3]>> = match self.current() {
            None => Some(None),
            Some(i) => {
                let path = self.lib.tracks[i].path.clone();
                self.covers.large(&path).map(|l| l.palette)
            }
        };
        if let Some(palette) = cover {
            self.ambience.palette.set_target(now, palette.unwrap_or(base_palette));
            let adaptive = palette.filter(|_| self.settings.adaptive_theme);
            self.ambience.theme.set_target(
                now,
                adaptive.map_or(base_colors, |p| theme_from_palette(p, base.bg)),
            );
        }
        let colors = self.ambience.theme.value(now);
        theme::set_adaptive(ctx, (colors != base_colors).then_some(colors));
        if self.ambience.palette.fading(now) || self.ambience.theme.fading(now) {
            ctx.request_repaint();
        }
    }

    /// Visualizer colors: those of the current album art (cross-faded from one track to the
    /// next), otherwise those of the theme.
    pub(super) fn viz_palette(&self) -> [Color32; 3] {
        let now = self.egui_ctx.input(|i| i.time);
        self.ambience.palette.value(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_fade_to_target() {
        let (red, blue) = (Color32::from_rgb(200, 0, 0), Color32::from_rgb(0, 0, 200));
        let mut f = ColorFade::<1>::default();
        f.set_target(10.0, [red]);
        assert_eq!(f.value(10.0), [red]); // straight away, the first time
        f.set_target(20.0, [blue]);
        assert_eq!(f.value(20.0), [red]);
        let mid = f.value(20.0 + COLOR_FADE / 2.0)[0];
        assert!(mid.r() > 60 && mid.b() > 60, "{mid:?}");
        assert_eq!(f.value(21.0), [blue]);
        assert!(f.fading(20.5) && !f.fading(21.0));
    }

    /// The theme taken from an album art keeps readable buttons (white text on the accent) and a
    /// dark background, whatever the starting color.
    #[test]
    fn adaptive_theme_stays_readable() {
        let luma = |c: Color32| (0.30 * c.r() as f32 + 0.59 * c.g() as f32 + 0.11 * c.b() as f32) / 255.0;
        let base_bg = Color32::from_rgb(0x14, 0x15, 0x1D);
        for palette in [
            [Color32::from_rgb(255, 240, 60), Color32::from_rgb(250, 250, 250), Color32::from_rgb(200, 200, 90)],
            [Color32::from_rgb(20, 20, 235), Color32::from_rgb(30, 10, 120), Color32::from_rgb(10, 10, 60)],
            [Color32::from_rgb(214, 220, 236), Color32::from_rgb(150, 162, 190), Color32::from_rgb(96, 106, 132)],
        ] {
            let [bg, accent, accent_2, light] = theme_from_palette(palette, base_bg);
            assert!(luma(bg) < 0.16, "fond {bg:?}");
            assert!((0.35..=0.60).contains(&luma(accent)), "accent {accent:?}");
            assert!(luma(accent_2) <= 0.60, "accent 2 {accent_2:?}");
            assert!(luma(light) > 0.62, "accent clair {light:?}");
        }
    }

    #[test]
    fn cover_waits_then_switches() {
        let (a, b) = (Path::new("/a.mp3"), Path::new("/b.mp3"));
        let mut f = TexFade::default();
        assert!(f.update(0.0, a, Some(None)));
        assert_eq!(f.t(0.0), 1.0); // no cross-fade on the first display
        // The new album art is loading: stay on the old one, then switch anyway.
        assert!(!f.update(5.0, b, None));
        assert!(!f.update(5.2, b, None));
        assert!(f.update(5.4, b, None));
        assert!(f.t(5.4) < 0.01 && f.previous(5.5).is_some());
        assert!(f.previous(6.0).is_none());
        // Loaded right away: immediate switch.
        assert!(f.update(9.0, a, Some(None)));
    }
}
