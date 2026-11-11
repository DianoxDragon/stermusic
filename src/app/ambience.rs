//! Ambiance : ce qui change en douceur d'un morceau à l'autre.
//!
//! - les couleurs du visualiseur passent de celles d'une pochette à celles de la suivante ;
//! - le thème adaptatif (option) fait de même avec les couleurs de toute l'interface ; sans
//!   pochette, on revient au thème choisi ;
//! - les pochettes (« En cours de lecture », son fond flouté) se fondent l'une dans l'autre :
//!   l'ancienne reste à l'écran le temps que la nouvelle soit chargée, sans passer par une case
//!   vide.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, TextureHandle};

use super::SterMusic;
use crate::theme;

/// Durée d'un changement de couleurs.
const COLOR_FADE: f64 = 0.9;
/// Durée d'un fondu de pochette.
pub const COVER_FADE: f64 = 0.45;
/// Attente maximale de la nouvelle pochette avant de changer quand même.
const COVER_WAIT: f64 = 0.35;

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `N` couleurs qui glissent vers une cible.
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
    /// Nouvelle cible : on part de ce qui est à l'écran (la première fois, on y est d'emblée).
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

/// Une image (pochette, fond flouté) qui se fond dans la suivante.
#[derive(Default)]
pub struct TexFade {
    cur: Option<(PathBuf, Option<TextureHandle>)>,
    prev: Option<Option<TextureHandle>>,
    since: f64,
    waiting: Option<f64>,
}

impl TexFade {
    /// `wanted` : le morceau à montrer ; `loaded` : son image si elle est chargée (`Some(None)` :
    /// chargée, il n'en a pas). Renvoie vrai à l'instant où l'on passe à ce morceau.
    pub fn update(&mut self, now: f64, wanted: &Path, loaded: Option<Option<TextureHandle>>) -> bool {
        match &mut self.cur {
            Some((path, tex)) if path == wanted => {
                // Image arrivée après l'attente : elle prend sa place.
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

    /// Avancement du fondu (0 → 1).
    pub fn t(&self, now: f64) -> f32 {
        ease(((now - self.since) / COVER_FADE) as f32)
    }

    pub fn fading(&self, now: f64) -> bool {
        now - self.since < COVER_FADE || self.waiting.is_some()
    }

    /// L'image à l'écran.
    pub fn current(&self) -> Option<&TextureHandle> {
        self.cur.as_ref().and_then(|(_, t)| t.as_ref())
    }

    /// L'image qui s'efface, tant que le fondu dure (`Some(None)` : un morceau sans image).
    pub fn previous(&self, now: f64) -> Option<Option<&TextureHandle>> {
        if now - self.since >= COVER_FADE {
            return None;
        }
        self.prev.as_ref().map(|t| t.as_ref())
    }
}

/// Couleurs d'un thème tirées de la palette d'une pochette : fond, accent, second accent, accent
/// clair. Le fond garde la profondeur du thème choisi, teinté par la pochette ; les accents sont
/// ramenés à une clarté où le texte blanc des boutons reste lisible.
pub fn theme_from_palette(palette: [Color32; 3], base_bg: Color32) -> [Color32; 4] {
    let accent = tone(palette[0], 0.50);
    let mut accent_2 = tone(palette[1], 0.46);
    // Deux accents presque identiques : le dégradé de marque serait plat.
    if distance(accent, accent_2) < 40.0 {
        accent_2 = tone(theme::mix(palette[1], palette[2], 0.6), 0.40);
    }
    let accent_light = tone(theme::mix(palette[0], Color32::WHITE, 0.30), 0.74);
    let tint = tone(palette[0], 0.10);
    let bg = theme::mix(base_bg, tint, 0.55);
    [bg, accent, accent_2, accent_light]
}

/// Ramène une couleur à la clarté `target` (0 → 1, moyenne pondérée des canaux) en gardant sa teinte.
fn tone(c: Color32, target: f32) -> Color32 {
    let (r, g, b) = (c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0);
    let luma = (0.30 * r + 0.59 * g + 0.11 * b).max(1e-3);
    let k = target / luma;
    let scale = |x: f32| x * k;
    let (mut r, mut g, mut b) = (scale(r), scale(g), scale(b));
    // Ce qui dépasse 1 est rendu en éclaircissant vers le blanc, sans changer de teinte.
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

/// État de l'ambiance (voir le module).
#[derive(Default)]
pub struct Ambience {
    /// Couleurs du visualiseur.
    palette: ColorFade<3>,
    /// Couleurs du thème (adaptatif ou non).
    theme: ColorFade<4>,
    /// Fond flouté de « En cours de lecture ».
    pub backdrop: TexFade,
    /// Pochette de « En cours de lecture ».
    pub cover: TexFade,
    /// Sens du dernier changement de morceau (1 : suivant, −1 : précédent) et place dans la file.
    pub dir: f32,
    queue_pos: Option<usize>,
}

impl SterMusic {
    /// À chaque image : fait glisser les couleurs vers celles du morceau en cours.
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
        // Pochette du morceau en cours : `None` tant qu'elle charge (on garde alors les couleurs
        // d'avant), `Some(None)` s'il n'en a pas (retour au thème choisi).
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

    /// Couleurs du visualiseur : celles de la pochette en cours (en fondu d'un morceau à
    /// l'autre), sinon celles du thème.
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
        assert_eq!(f.value(10.0), [red]); // d'emblée, la première fois
        f.set_target(20.0, [blue]);
        assert_eq!(f.value(20.0), [red]);
        let mid = f.value(20.0 + COLOR_FADE / 2.0)[0];
        assert!(mid.r() > 60 && mid.b() > 60, "{mid:?}");
        assert_eq!(f.value(21.0), [blue]);
        assert!(f.fading(20.5) && !f.fading(21.0));
    }

    /// Le thème tiré d'une pochette garde des boutons lisibles (texte blanc sur l'accent) et un
    /// fond sombre, quelle que soit la couleur de départ.
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
        assert_eq!(f.t(0.0), 1.0); // pas de fondu au premier affichage
        // La nouvelle pochette charge : on reste sur l'ancienne, puis on bascule quand même.
        assert!(!f.update(5.0, b, None));
        assert!(!f.update(5.2, b, None));
        assert!(f.update(5.4, b, None));
        assert!(f.t(5.4) < 0.01 && f.previous(5.5).is_some());
        assert!(f.previous(6.0).is_none());
        // Chargée tout de suite : bascule immédiate.
        assert!(f.update(9.0, a, Some(None)));
    }
}
