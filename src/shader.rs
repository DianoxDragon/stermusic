//! Vagues de lumière dessinées par la carte graphique (shader GLSL), comme le verre dépoli de
//! SterCode : un rappel OpenGL glissé dans le rendu d'egui. Nettes à toutes les tailles, avec une
//! vraie lueur et des fondus doux ; le spectre et les couleurs de la pochette arrivent à chaque image.
//!
//! Si le pilote refuse le shader, `available()` devient faux et l'appelant revient au rendu
//! calculé par le processeur.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Rect};
use eframe::egui_glow;
use eframe::glow::{self, HasContext};

/// Nombre de niveaux de spectre envoyés au shader (du centre vers les bords).
pub const LEVELS: usize = 16;

/// Réglages d'une image de vagues.
#[derive(Clone, Copy)]
pub struct Waves {
    pub time: f32,
    pub levels: [f32; LEVELS],
    pub colors: [[f32; 3]; 3],
    pub kick: f32,
    pub bass: f32,
    pub intensity: f32,
    /// Ligne de base des vagues (0 = haut, 1 = bas du rectangle).
    pub base: f32,
    /// Amplitude relative des vagues.
    pub amp: f32,
    /// Fondus progressifs en haut et en bas (part de la hauteur).
    pub fade_top: f32,
    pub fade_bottom: f32,
    /// Atténuation à gauche (texte du bandeau de l'accueil), 0 = aucune.
    pub left: f32,
    /// Rayon des coins, en points.
    pub radius: f32,
    /// Visualiseur (voir [`STYLES`]).
    pub mode: i32,
    pub energy: f32,
    /// Zoom des motifs (caustiques, nébuleuse, plasma) : plus grand = motifs plus larges.
    pub scale: f32,
    /// Forme d'onde récente (−1 → 1), pour l'Oscilloscope.
    pub wave: [f32; WAVE],
    /// Pochette (floutée) pour la Pochette liquide.
    pub texture: Option<egui::TextureId>,
    /// Centre et rayon de la pochette, en points depuis le coin haut gauche du rectangle (Anneau).
    pub focus: [f32; 3],
    /// Réglage « détail » de l'effet (0 → 1 ; 0,5 : d'origine), voir [`DETAILS`].
    pub detail: f32,
    /// Zoom de l'effet (1 : d'origine) et son centre, en points depuis le coin haut gauche.
    pub zoom: f32,
    pub zoom_center: [f32; 2],
}

/// Nombre de points de la forme d'onde envoyés au shader.
pub const WAVE: usize = 128;

/// Les visualiseurs : nom, et vrai s'il remplit toute la page (sinon, une bande en bas).
/// Choix « Automatique » : un visualiseur au hasard à chaque morceau (et toutes les 45 s).
pub const AUTO: usize = STYLES.len();

pub const STYLES: [(&str, bool); 50] = [
    (crate::i18n::n_("Aurore"), false),
    (crate::i18n::n_("Caustiques"), true),
    (crate::i18n::n_("Nébuleuse"), true),
    (crate::i18n::n_("Spectre"), false),
    (crate::i18n::n_("Ondes"), true),
    (crate::i18n::n_("Lignes"), false),
    (crate::i18n::n_("Plasma"), true),
    (crate::i18n::n_("Faisceaux"), true),
    (crate::i18n::n_("Étoiles"), true),
    (crate::i18n::n_("Rayons"), true),
    (crate::i18n::n_("Pochette liquide"), true),
    (crate::i18n::n_("Bokeh"), true),
    (crate::i18n::n_("Anneau"), true),
    (crate::i18n::n_("Oscilloscope"), false),
    (crate::i18n::n_("Fumée"), true),
    (crate::i18n::n_("Lucioles"), true),
    (crate::i18n::n_("Horizon"), false),
    (crate::i18n::n_("Orage"), true),
    (crate::i18n::n_("Tunnel"), true),
    (crate::i18n::n_("Spirale"), true),
    (crate::i18n::n_("Pluie de lumière"), true),
    (crate::i18n::n_("Kaléidoscope"), true),
    (crate::i18n::n_("Rosace"), true),
    (crate::i18n::n_("Flammes"), true),
    (crate::i18n::n_("Galaxie"), true),
    (crate::i18n::n_("Cristaux"), true),
    (crate::i18n::n_("Lampe à lave"), true),
    (crate::i18n::n_("Diodes"), false),
    (crate::i18n::n_("Vinyle"), true),
    (crate::i18n::n_("Ferrofluide"), true),
    (crate::i18n::n_("Hologramme"), true),
    (crate::i18n::n_("Soie"), true),
    (crate::i18n::n_("Irisé"), true),
    (crate::i18n::n_("Topographie"), true),
    (crate::i18n::n_("Verre"), false),
    (crate::i18n::n_("Flux"), true),
    (crate::i18n::n_("Trame"), true),
    (crate::i18n::n_("Lissajous"), true),
    (crate::i18n::n_("Rétro"), true),
    (crate::i18n::n_("Mosaïque"), true),
    (crate::i18n::n_("Vitrail"), true),
    (crate::i18n::n_("Radar"), true),
    (crate::i18n::n_("Hélice"), true),
    (crate::i18n::n_("Pendules"), true),
    (crate::i18n::n_("Constellation"), true),
    (crate::i18n::n_("Dunes"), true),
    (crate::i18n::n_("Bulles"), true),
    (crate::i18n::n_("Éclipse"), true),
    (crate::i18n::n_("Moiré"), true),
    (crate::i18n::n_("Néon"), true),
];

/// Le réglage « détail » de chaque visualiseur (dans l'ordre de [`STYLES`]) : son nom, ou `None`
/// s'il n'en a pas.
pub const DETAILS: [Option<&str>; 50] = [
    Some(crate::i18n::n_("Finesse des rayons")),
    Some(crate::i18n::n_("Finesse des reflets")),
    Some(crate::i18n::n_("Finesse")),
    Some(crate::i18n::n_("Nombre de colonnes")),
    Some(crate::i18n::n_("Nombre d'ondes")),
    Some(crate::i18n::n_("Nombre de lignes")),
    Some(crate::i18n::n_("Finesse")),
    Some(crate::i18n::n_("Nombre de projecteurs")),
    Some(crate::i18n::n_("Densité")),
    Some(crate::i18n::n_("Nombre de rayons")),
    Some(crate::i18n::n_("Ondulation")),
    Some(crate::i18n::n_("Nombre de disques")),
    Some(crate::i18n::n_("Nombre de rayons")),
    Some(crate::i18n::n_("Amplitude")),
    Some(crate::i18n::n_("Turbulence")),
    Some(crate::i18n::n_("Nombre de lucioles")),
    Some(crate::i18n::n_("Relief")),
    Some(crate::i18n::n_("Fréquence des éclairs")),
    Some(crate::i18n::n_("Nombre de rayons")),
    Some(crate::i18n::n_("Nombre de bras")),
    Some(crate::i18n::n_("Densité")),
    Some(crate::i18n::n_("Nombre de miroirs")),
    Some(crate::i18n::n_("Nombre de pétales")),
    Some(crate::i18n::n_("Hauteur des flammes")),
    Some(crate::i18n::n_("Enroulement des bras")),
    Some(crate::i18n::n_("Nombre de facettes")),
    Some(crate::i18n::n_("Nombre de bulles")),
    Some(crate::i18n::n_("Nombre de colonnes")),
    Some(crate::i18n::n_("Nombre de plages")),
    Some(crate::i18n::n_("Nombre de pointes")),
    Some(crate::i18n::n_("Nombre de points")),
    Some(crate::i18n::n_("Relief des plis")),
    Some(crate::i18n::n_("Nombre de nervures")),
    Some(crate::i18n::n_("Nombre de courbes")),
    Some(crate::i18n::n_("Nombre de colonnes")),
    Some(crate::i18n::n_("Finesse des traînées")),
    Some(crate::i18n::n_("Finesse de la trame")),
    Some(crate::i18n::n_("Taille de la boucle")),
    Some(crate::i18n::n_("Relief")),
    Some(crate::i18n::n_("Nombre de carreaux")),
    Some(crate::i18n::n_("Nombre d'éclats")),
    Some(crate::i18n::n_("Nombre d'échos")),
    Some(crate::i18n::n_("Nombre de barreaux")),
    Some(crate::i18n::n_("Nombre de pendules")),
    Some(crate::i18n::n_("Nombre d'étoiles")),
    Some(crate::i18n::n_("Relief")),
    Some(crate::i18n::n_("Nombre de bulles")),
    Some(crate::i18n::n_("Taille de la couronne")),
    Some(crate::i18n::n_("Écart des cercles")),
    Some(crate::i18n::n_("Halo des tubes")),
];

/// Visualiseurs centrés sur la pochette (leur zoom se fait autour d'elle).
pub const FOCUSED: [usize; 10] = [12, 21, 22, 28, 29, 30, 37, 41, 47, 49];

struct Resources {
    program: glow::Program,
    vao: glow::VertexArray,
    loc: Vec<Option<glow::UniformLocation>>,
    /// Textures d'egui stockées en sRGB (leur lecture renvoie alors des couleurs linéaires).
    srgb_textures: bool,
}

const UNIFORMS: [&str; 22] = [
    "u_rect",
    "u_radius",
    "u_time",
    "u_levels",
    "u_colors",
    "u_kick",
    "u_bass",
    "u_intensity",
    "u_shape",
    "u_left",
    "u_ppp",
    "u_fade",
    "u_mode",
    "u_energy",
    "u_scale",
    "u_wave",
    "u_focus",
    "u_tex",
    "u_has_tex",
    "u_srgb",
    "u_detail",
    "u_zoom",
];

/// `None` tant que rien n'a été créé, `Some(None)` si la création a échoué.
static RESOURCES: Mutex<Option<Option<Resources>>> = Mutex::new(None);
/// 0 : inconnu, 1 : fonctionne, 2 : refusé par le pilote.
static STATE: AtomicU8 = AtomicU8::new(0);

/// Faux si le pilote a refusé le shader (utiliser alors le rendu du processeur).
pub fn available() -> bool {
    STATE.load(Ordering::Relaxed) != 2
}

/// Dessine des vagues dans `rect`.
pub fn paint(painter: &egui::Painter, rect: Rect, waves: Waves) {
    let callback = egui_glow::CallbackFn::new(move |info, painter| {
        let mut guard = RESOURCES.lock().unwrap();
        let resources = guard.get_or_insert_with(|| {
            let r = unsafe { create(painter.gl()) };
            STATE.store(if r.is_some() { 1 } else { 2 }, Ordering::Relaxed);
            r
        });
        if let Some(res) = resources.as_ref() {
            let texture = waves.texture.and_then(|id| painter.texture(id));
            unsafe { draw(painter.gl(), res, &info, &waves, texture) };
        }
    });
    painter.add(egui::Shape::Callback(egui::PaintCallback {
        rect,
        callback: Arc::new(callback),
    }));
}

const FRAGMENT: &str = r#"
uniform vec4 u_rect;        // x0, y0, x1, y1 en pixels (origine en bas à gauche)
uniform float u_radius;     // pixels
uniform float u_time;
uniform float u_levels[16]; // spectre, du centre (graves) vers les bords (aigus)
uniform vec3 u_colors[3];
uniform float u_kick;
uniform float u_bass;
uniform float u_energy;
uniform float u_intensity;
uniform vec4 u_shape;       // base, amplitude, —, —
uniform vec2 u_fade;        // fondu haut, fondu bas
uniform float u_left;
uniform float u_ppp;
uniform int u_mode;
uniform float u_scale;
uniform float u_wave[128];
uniform vec3 u_focus;       // centre x, y (pixels depuis le coin haut gauche), rayon
uniform sampler2D u_tex;
uniform int u_has_tex;
uniform int u_srgb;
uniform float u_detail;     // réglage « détail » de l'effet (0 → 1)
uniform vec3 u_zoom;        // zoom, puis son centre x, y (pixels depuis le coin haut gauche)
out vec4 out_color;

// ---------- Outils ----------

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
vec2 hash2(vec2 p) {
    return fract(sin(vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)))) * 43758.5453);
}
float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 w = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), w.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), w.x), w.y);
}
float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 5; i++) {
        v += a * noise(p);
        p = p * 2.03 + vec2(1.7, 9.2);
        a *= 0.5;
    }
    return v;
}
float level(float u) {
    float d = clamp(abs(u * 2.0 - 1.0), 0.0, 1.0) * 15.0;
    int i = int(floor(d));
    return mix(u_levels[i], u_levels[min(i + 1, 15)], fract(d));
}
float envelope(float u) { return pow(sin(3.14159 * clamp(u, 0.0, 1.0)), 1.2); }
float wave(float u, float freq, float speed, float phase) {
    return sin(u * freq * 6.28318 + u_time * speed + phase)
         + 0.35 * sin(u * freq * 2.3 * 6.28318 - u_time * speed * 1.4 + phase);
}
// Distance au bord de cellule (F2 − F1) d'un réseau de Voronoï animé : des filets de lumière.
float cell_edge(vec2 p, float t) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    float d1 = 8.0;
    float d2 = 8.0;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 g = vec2(float(x), float(y));
            vec2 o = 0.5 + 0.45 * sin(t + 6.28318 * hash2(i + g));
            float d = length(g + o - f);
            if (d < d1) { d2 = d1; d1 = d; } else if (d < d2) { d2 = d; }
        }
    }
    return d2 - d1;
}

vec3 srgb_from_linear(vec3 c) {
    return mix(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, vec3(lessThan(c, vec3(0.0031308))));
}
// Niveau d'une fréquence, de 0 (graves) à 1 (aigus).
float band(float f) {
    float d = clamp(f, 0.0, 1.0) * 15.0;
    int i = int(floor(d));
    return mix(u_levels[i], u_levels[min(i + 1, 15)], fract(d));
}
mat2 rot(float a) {
    float c = cos(a);
    float s = sin(a);
    return mat2(c, -s, s, c);
}
// Couleur de la pochette (floutée) en `uv` (0 → 1), sinon un dégradé de la palette.
vec3 cover_color(vec2 uv) {
    if (u_has_tex == 1) {
        vec3 c = texture(u_tex, clamp(uv, 0.0, 1.0)).rgb;
        return u_srgb == 1 ? srgb_from_linear(c) : c;
    }
    return mix(u_colors[2], u_colors[0], uv.y);
}
// Réglage « détail » de l'effet (0 → 1, 0,5 au départ) : `lo` à 0, `mid` à 0,5, `hi` à 1.
float detail(float lo, float mid, float hi) {
    return u_detail < 0.5 ? mix(lo, mid, u_detail * 2.0) : mix(mid, hi, u_detail * 2.0 - 1.0);
}

// ---------- 1. Caustiques : reflets de l'eau au fond d'une piscine ----------

// Distance au bord de cellule la plus proche (réseau de Voronoï animé).
float caustic_layer(vec2 p, float t) {
    // Forte déformation par un bruit doux : les filets se courbent comme des reflets d'eau.
    vec2 warp = vec2(fbm(p * 0.55 + vec2(t * 0.12, 0.0)), fbm(p * 0.55 + vec2(5.2, t * 0.10))) - 0.5;
    p += warp * 1.8;
    float e = cell_edge(p, t);
    return exp(-e / 0.045);
}
vec3 caustics(vec2 px, float H, float v) {
    vec2 p = px / (H * 0.42 * u_scale) * detail(0.55, 1.0, 1.9);
    float t = u_time * 0.8;
    float a = caustic_layer(p, t);
    float b = caustic_layer(p * 1.35 + vec2(3.7, 1.9), t * 1.2);
    // Brillance irrégulière ; les croisements des deux couches s'illuminent davantage.
    float light = (0.55 * a + 0.45 * b + 1.4 * a * b) * (0.55 + 0.6 * noise(p * 0.7 + t * 0.1));
    float shimmer = 0.55 + 0.35 * u_energy + 0.6 * u_kick;
    vec3 water = mix(u_colors[2], u_colors[1], v) * (0.07 + 0.05 * v);
    return water + mix(u_colors[1], vec3(1.0), 0.6) * light * 0.30 * shimmer;
}

// ---------- 2. Nébuleuse : nuages de couleur qui dérivent ----------

vec3 nebula(vec2 px, float H) {
    vec2 p = px / (H * u_scale) * 1.3;
    float t = u_time * 0.12;
    p *= detail(0.45, 1.0, 2.4);
    float n = fbm(p + vec2(t, -t * 0.6) + 1.5 * fbm(p * 0.7 - vec2(t * 0.8, t * 0.3)));
    vec3 col = mix(u_colors[2], u_colors[0], smoothstep(0.30, 0.75, n));
    col = mix(col, u_colors[1], smoothstep(0.62, 0.92, n));
    return col * (0.22 + 1.2 * n * n) * (0.75 + 0.45 * u_bass + 0.6 * u_kick);
}

// ---------- 4. Ondes : anneaux qui partent du bas sur le rythme ----------

vec3 ripples(vec2 px, float W, float H, float u) {
    vec2 c = vec2(0.5 * W, H * 1.02);
    float r = length(px - c) / H;
    vec3 acc = vec3(0.0);
    int count = int(floor(detail(3.0, 6.0, 12.0)));
    for (int k = 0; k < 12; k++) {
        if (k >= count) break;
        float rk = fract(u_time * 0.10 + float(k) / float(count)) * 1.5;
        float width = 0.010 + 0.025 * level(u) + 0.015 * u_kick;
        float ring = exp(-pow((r - rk) / width, 2.0)) * (1.0 - rk / 1.5);
        acc += mix(u_colors[k - (k / 3) * 3], vec3(1.0), 0.2) * ring * (0.35 + 0.7 * u_energy);
    }
    acc += u_colors[2] * exp(-r * r * 6.0) * (0.15 + 0.6 * u_kick);
    return acc;
}

// ---------- 6. Plasma : couleurs douces qui se mélangent ----------

vec3 plasma(vec2 px, float H) {
    vec2 p = px / (H * u_scale) * detail(0.5, 1.0, 2.2);
    float t = u_time * 0.35;
    float v = sin(p.x * 2.6 + t) + sin(p.y * 3.1 - t * 1.2) + sin((p.x + p.y) * 2.0 + t * 0.7)
            + sin(length(p - vec2(1.0, 0.6)) * 4.5 - t);
    v = v * 0.25 + 0.5;
    vec3 col = mix(u_colors[2], u_colors[0], smoothstep(0.2, 0.6, v));
    col = mix(col, u_colors[1], smoothstep(0.6, 0.95, v));
    return col * (0.38 + 0.30 * u_bass + 0.45 * u_kick);
}

// ---------- 8. Étoiles : un champ d'étoiles qui file vers soi ----------

vec3 stars(vec2 px, float W, float H) {
    vec2 c = vec2(0.5 * W, 0.55 * H);
    vec2 p = (px - c) / H;
    vec3 acc = vec3(0.0);
    for (int i = 0; i < 3; i++) {
        float z = fract(u_time * 0.06 + float(i) / 3.0);
        float zoom = mix(10.0, 1.2, z) * detail(0.5, 1.0, 1.9);
        vec2 q = p * zoom + float(i) * 17.0;
        vec2 g0 = floor(q);
        vec2 f0 = fract(q);
        // Petit point net et son halo ; les étoiles grossissent en approchant.
        float px_size = u_ppp / H * zoom;
        float life = smoothstep(0.0, 0.3, z) * (1.0 - smoothstep(0.85, 1.0, z));
        // Une étoile par case, mais chaque pixel regarde aussi les cases voisines : sinon le halo
        // d'une étoile proche d'un bord était coupé net et dessinait une grille.
        for (int y = -1; y <= 1; y++) {
            for (int x = -1; x <= 1; x++) {
                vec2 cell = vec2(float(x), float(y));
                vec2 g = g0 + cell;
                vec2 o = 0.15 + 0.7 * hash2(g);
                float d = length(f0 - cell - o);
                float core = exp(-pow(d / (px_size * (1.2 + 2.5 * z)), 2.0));
                float halo = exp(-d / (0.02 + 0.03 * z)) * 0.25;
                float twinkle = 0.6 + 0.4 * sin(u_time * 3.0 + hash(g) * 30.0);
                acc += mix(u_colors[1], vec3(1.0), 0.7) * (core + halo) * life * twinkle
                     * (0.8 + 1.0 * u_energy + 1.2 * u_kick);
            }
        }
    }
    acc += u_colors[2] * exp(-dot(p, p) * 3.0) * (0.10 + 0.3 * u_bass);
    return acc;
}

// ---------- 10. Pochette liquide : la pochette floutée qui coule lentement ----------

vec3 liquid(vec2 px, float W, float H) {
    float t = u_time * 0.07;
    vec2 q = (px - vec2(W, H) * 0.5) / H;
    float a = t * 0.5;
    q = mat2(cos(a), -sin(a), sin(a), cos(a)) * q / (1.4 * u_scale);
    vec2 warp = vec2(fbm(q * 2.0 + vec2(t, -t)), fbm(q * 2.0 + vec2(-t, t) + 5.3)) - 0.5;
    q += warp * (detail(0.12, 0.55, 1.2) + 0.30 * u_bass + 0.25 * u_kick);
    vec2 uv = q + 0.5;
    uv = 1.0 - abs(1.0 - mod(uv, 2.0));  // miroir : jamais de bord visible
    vec3 col;
    if (u_has_tex == 1) {
        col = texture(u_tex, uv).rgb;
        if (u_srgb == 1) col = srgb_from_linear(col);
    } else {
        col = mix(u_colors[2], u_colors[0], fbm(q * 3.0));
    }
    float l = dot(col, vec3(0.299, 0.587, 0.114));
    col = max(mix(vec3(l), col, 1.35), 0.0);
    return col * (1.05 + 0.35 * u_bass + 0.45 * u_kick);
}

// ---------- 11. Bokeh : disques de lumière flous qui dérivent ----------

vec3 bokeh(vec2 px, float W, float H) {
    vec3 acc = vec3(0.0);
    int count = int(floor(detail(8.0, 22.0, 44.0)));
    for (int i = 0; i < 44; i++) {
        if (i >= count) break;
        float fi = float(i);
        vec2 h = vec2(hash(vec2(fi, 1.3)), hash(vec2(fi, 7.1)));
        vec2 c = vec2(W * h.x, H * h.y)
               + vec2(sin(u_time * (0.05 + 0.05 * h.y) + fi), cos(u_time * (0.04 + 0.05 * h.x) + fi * 1.7)) * H * 0.12;
        float r = H * (0.04 + 0.10 * hash(vec2(fi, 3.7))) * (1.0 + 0.25 * u_bass + 0.2 * u_kick);
        float d = length(px - c);
        float disc = smoothstep(r, r * 0.92 - 1.5 * u_ppp, d);
        float rim = exp(-pow((d - r * 0.93) / (r * 0.07), 2.0));
        float lv = u_levels[i - (i / 16) * 16];
        acc += u_colors[i - (i / 3) * 3] * (disc * 0.11 + rim * 0.08) * (0.7 + 0.9 * lv);
    }
    return acc;
}

// ---------- 12. Anneau : cercle de lumière autour de la pochette ----------

vec3 ring(vec2 px, float W, float H) {
    vec2 c = u_focus.xy;
    float r0 = u_focus.z * 1.06 + 8.0 * u_ppp;
    vec2 d = px - c;
    float r = length(d);
    float u = fract(atan(d.y, d.x) / 6.28318 + 0.25);
    float n = floor(detail(40.0, 120.0, 240.0));
    float lvq = level(floor(u * n) / n + 0.5 / n);
    float ang = (fract(u * n) - 0.5) / n * 6.28318;
    float line = exp(-pow(abs(ang) * r / (0.9 * u_ppp + 0.6), 2.0));
    float len = H * 0.03 + lvq * H * 0.22 * (1.0 + 0.3 * u_kick);
    float along = r - r0;
    float body = smoothstep(0.0, 2.0 * u_ppp, along) * (1.0 - smoothstep(len * 0.8, len, along));
    vec3 col = mix(u_colors[0], mix(u_colors[1], vec3(1.0), 0.3), clamp(along / max(len, 1.0), 0.0, 1.0));
    float rim = exp(-pow((r - r0) / ((1.6 + 2.0 * u_bass) * u_ppp), 2.0));
    float halo = exp(-pow((r - r0) / (H * 0.10), 2.0)) * (0.12 + 0.45 * u_kick);
    return col * line * body * 0.9 + mix(u_colors[1], vec3(1.0), 0.4) * rim * 0.55 + u_colors[2] * halo;
}

// ---------- 14. Fumée : volutes d'encre colorée ----------

vec3 smoke(vec2 px, float H) {
    vec2 p = px / (H * u_scale) * 1.6;
    float t = u_time * 0.10;
    vec2 q = vec2(fbm(p + vec2(0.0, t)), fbm(p + vec2(5.2, -t)));
    float swirl = detail(0.8, 3.0, 6.0);
    vec2 r = vec2(fbm(p + swirl * q + vec2(1.7, 9.2) + t * 0.7), fbm(p + swirl * q + vec2(8.3, 2.8) - t * 0.5));
    float f = fbm(p + 2.5 * r);
    vec3 col = mix(u_colors[2], u_colors[0], clamp(f * f * 2.0, 0.0, 1.0));
    col = mix(col, u_colors[1], clamp(length(q) * 0.8, 0.0, 1.0) * 0.6);
    return col * pow(f, 2.2) * 0.95 * (0.7 + 0.5 * u_bass + 0.9 * u_kick);
}

// ---------- 15. Lucioles : petites lumières et leur traînée ----------

vec3 fireflies(vec2 px, float W, float H) {
    vec3 acc = vec3(0.0);
    vec3 warm = mix(u_colors[1], vec3(1.0, 0.95, 0.75), 0.4);
    int count = int(floor(detail(8.0, 26.0, 48.0)));
    for (int i = 0; i < 48; i++) {
        if (i >= count) break;
        float fi = float(i);
        vec2 seed = vec2(hash(vec2(fi, 2.1)), hash(vec2(fi, 5.9)));
        // u_time avance déjà plus vite quand la musique est forte : le multiplier par l'énergie
        // instantanée faisait sauter les lucioles d'avant en arrière à chaque variation.
        float speed = 0.30 + 0.35 * hash(vec2(fi, 8.3));
        float blink = 0.5 + 0.5 * sin(u_time * (1.5 + seed.x * 2.0) + fi);
        for (int k = 0; k < 6; k++) {
            float tk = u_time * speed - float(k) * 0.05;
            vec2 pos = vec2(W * (seed.x + 0.18 * sin(tk * 1.3 + fi) + 0.10 * sin(tk * 2.7 + fi * 3.0)),
                            H * (seed.y + 0.18 * cos(tk * 1.1 + fi * 2.0) + 0.10 * cos(tk * 2.3 + fi)));
            float d = length(px - pos);
            float size = (2.4 + 1.8 * u_kick) * u_ppp * (1.0 - float(k) * 0.12);
            float fade = 1.0 - float(k) / 6.0;
            acc += warm * (exp(-d * d / (size * size)) + 0.35 * exp(-d / (16.0 * u_ppp))) * fade * blink * (0.8 + 0.7 * u_energy);
        }
    }
    return acc;
}

// ---------- 17. Orage : nuages et éclairs sur les coups de grosse caisse ----------

vec3 storm(vec2 px, float W, float H) {
    vec2 p = px / (H * u_scale) * 1.4;
    float t = u_time * 0.05;
    float clouds = fbm(p + vec2(t, 0.0) + fbm(p * 0.5 - t));
    float seed = floor(u_time * 0.7);
    float fx = W * (0.2 + 0.6 * hash(vec2(seed, 4.2)));
    float flash = u_kick * u_kick;
    float near = exp(-pow(length(px - vec2(fx, H * 0.25)) / (H * 0.6), 2.0));
    vec3 col = mix(u_colors[2], u_colors[1], clouds) * clouds * clouds * (0.55 + 0.3 * u_bass);
    col += mix(u_colors[1], vec3(0.85, 0.9, 1.0), 0.6) * clouds * near * flash * 2.2;
    float bx = fx + (fbm(vec2(px.y / H * 6.0, seed)) - 0.5) * H * 0.35;
    float bolt = exp(-pow((px.x - bx) / (1.5 * u_ppp), 2.0)) * smoothstep(H * 0.75, H * 0.35, px.y);
    // Réglage : plus il est haut, plus un petit coup suffit à faire tomber un éclair.
    float th = detail(0.6, 0.35, 0.10);
    col += vec3(0.95, 0.95, 1.0) * bolt * smoothstep(th, th + 0.45, u_kick);
    return col;
}

// ---------- 18. Tunnel : anneaux et rayons qui filent vers soi ----------

vec3 tunnel(vec2 px, float W, float H) {
    vec2 p = (px - vec2(0.5 * W, 0.5 * H)) / H;
    float r = max(length(p), 1e-3);
    float u = fract(atan(p.y, p.x) / 6.28318 + 0.25);
    float z = 0.3 / r + u_time * 0.5;  // profondeur
    float lv = level(u);
    // Largeur d'un pixel en profondeur : les anneaux lointains, serrés, restent nets.
    float pixel = u_ppp / H * 0.3 / (r * r);
    float rings = exp(-pow((fract(z) - 0.5) / max(0.045, pixel * 1.5), 2.0));
    float nsp = floor(detail(8.0, 24.0, 48.0));
    float ds = abs(fract(u * nsp) - 0.5) / nsp * 6.28318 * r * H;
    float spokes = exp(-pow(ds / (1.2 * u_ppp), 2.0));
    float fog = smoothstep(0.03, 0.45, r);
    vec3 col = mix(u_colors[0], u_colors[1], 0.5 + 0.5 * sin(z * 0.8));
    float glow = exp(-r * r * 8.0) * (0.15 + 0.6 * u_kick);
    return col * (rings * (0.45 + 1.2 * lv + 0.6 * u_kick) + spokes * (0.15 + 0.6 * lv)) * fog
         + u_colors[2] * glow;
}

// ---------- 19. Spirale : bras de lumière qui tournent ----------

vec3 spiral(vec2 px, float W, float H) {
    vec2 p = (px - vec2(0.5 * W, 0.5 * H)) / H;
    float r = length(p);
    float a = atan(p.y, p.x);
    float u = fract(a / 6.28318 + 0.5);
    float lv = level(u);
    float band = 0.5 + 0.5 * cos(a * floor(detail(2.0, 5.0, 10.0)) + log(r + 1e-3) * 9.0 - u_time * 1.2);
    float line = pow(band, 8.0 - 4.0 * lv);
    vec3 col = mix(u_colors[0], u_colors[1], 0.5 + 0.5 * sin(a * 2.0 + u_time * 0.3));
    float fade = smoothstep(0.0, 0.08, r) * exp(-r * 1.6);
    float core = exp(-r * r * 40.0) * (0.3 + 1.0 * u_kick);
    return col * line * fade * (0.8 + 0.8 * u_energy + 0.6 * u_kick)
         + mix(u_colors[2], vec3(1.0), 0.3) * core;
}

// ---------- 23. Flammes : un feu qui monte plus haut sur les graves ----------

vec3 flames(vec2 px, float W, float H, float u) {
    vec2 p = vec2(px.x / H, (H - px.y) / H);  // y vers le haut, depuis le bas
    float t = u_time * 0.9;
    float lv = level(u) * envelope(u);
    float n = fbm(vec2(p.x * 3.0, p.y * 2.0 - t) + fbm(vec2(p.x * 1.5 + 3.0, p.y - t * 0.6)));
    float height = (0.22 + 0.45 * lv + 0.15 * u_bass + 0.15 * u_kick) * detail(0.55, 1.0, 1.6);
    float f = clamp(1.0 - p.y / height + (n - 0.5) * 1.1, 0.0, 1.0);
    f *= f;
    vec3 col = mix(u_colors[2] * 0.4, u_colors[0], smoothstep(0.0, 0.5, f));
    col = mix(col, mix(u_colors[1], vec3(1.0, 0.95, 0.85), 0.5), smoothstep(0.5, 1.0, f));
    return col * f * 1.3 * (0.7 + 0.5 * u_energy);
}

// ---------- 25. Cristaux : facettes qui s'allument sur leurs fréquences ----------

vec3 crystals(vec2 px, float W, float H) {
    float size = H * detail(0.34, 0.18, 0.08) * u_scale;
    vec2 p = px / size;
    vec2 i = floor(p);
    vec2 f = fract(p);
    float d1 = 8.0;
    float d2 = 8.0;
    vec2 best = vec2(0.0);
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 g = vec2(float(x), float(y));
            vec2 o = 0.5 + 0.4 * sin(u_time * 0.3 + 6.28318 * hash2(i + g));
            float d = length(g + o - f);
            if (d < d1) { d2 = d1; d1 = d; best = i + g; } else if (d < d2) { d2 = d; }
        }
    }
    float h = hash(best);
    float lv = level(clamp((best.x + 0.5) * size / W, 0.0, 1.0));
    float facet = (0.25 + 0.75 * h) * (0.25 + 1.1 * lv);
    float edge = exp(-(d2 - d1) / 0.03);
    vec3 col = mix(u_colors[0], u_colors[1], h);
    return col * facet * 0.35 * (0.7 + 0.6 * u_kick) * (1.0 - 0.4 * d1)
         + mix(col, vec3(1.0), 0.5) * edge * (0.25 + 0.5 * u_energy);
}

// ---------- 26. Lampe à lave : bulles qui montent, fusionnent et gonflent sur les graves ----------

vec3 lava(vec2 px, float W, float H) {
    vec2 p = px / H;
    float field = 0.0;
    vec3 tint = vec3(0.0);
    int count = int(floor(detail(4.0, 9.0, 16.0)));
    for (int i = 0; i < 16; i++) {
        if (i >= count) break;
        float fi = float(i);
        vec2 h = vec2(hash(vec2(fi, 2.7)), hash(vec2(fi, 9.4)));
        float speed = 0.05 + 0.06 * h.y;
        vec2 c = vec2((0.1 + 0.8 * h.x) * W / H + 0.08 * sin(u_time * 0.3 + fi),
                      0.5 + 0.45 * sin(u_time * speed * 3.0 + fi * 2.1));
        float r = (0.07 + 0.05 * h.x) * (1.0 + 0.25 * u_bass + 0.2 * u_kick + 0.3 * u_levels[i]);
        vec2 d = p - c;
        float k = r * r / max(dot(d, d), 1e-4);
        field += k;
        tint += u_colors[i - (i / 3) * 3] * k;
    }
    vec3 col = tint / max(field, 1e-4);
    float blob = smoothstep(0.9, 1.1, field);
    float rim = exp(-pow((field - 1.0) / 0.12, 2.0));
    float glow = clamp(field * 0.25, 0.0, 0.6);
    return col * (blob * 0.55 + glow * 0.35) + mix(col, vec3(1.0), 0.5) * rim * 0.35;
}

// ---------- 28. Vinyle : un disque qui tourne derrière la pochette, ses sillons scintillent ----------

vec3 vinyl(vec2 px, float W, float H) {
    float R = u_focus.z * 1.5;
    vec2 d = px - u_focus.xy;
    float rr = length(d);
    float r = rr / R;
    float a = atan(d.y, d.x);
    float spin = u_time * 1.4;
    vec3 acc = u_colors[2] * exp(-pow((r - 1.0) * 2.4, 2.0)) * (0.08 + 0.22 * u_bass + 0.35 * u_kick);
    float edge = 1.0 - smoothstep(1.0 - 1.5 * u_ppp / R, 1.0, r);
    if (edge <= 0.0) return acc;
    // Du bord (graves) vers l'étiquette (aigus).
    float f = clamp((1.0 - r) / 0.64, 0.0, 1.0);
    float lv = band(f);
    float groove = 0.55 + 0.45 * sin(rr / u_ppp * 1.9);
    float track = smoothstep(0.0, 0.015, abs(fract(r * floor(detail(3.0, 7.0, 14.0)) + 0.3) - 0.5) - 0.025);
    // La lampe est fixe, le disque tourne dessous : les reflets restent, la poussière défile.
    float hl = pow(abs(cos(a - 0.7)), 22.0) + 0.55 * pow(abs(cos(a + 0.9)), 40.0);
    float dust = smoothstep(0.82, 1.0, noise(vec2((a - spin) * 40.0, r * 90.0)));
    vec3 sheen = mix(u_colors[0], u_colors[1], f);
    vec3 disc = sheen * hl * (0.55 + 1.6 * lv + 0.6 * u_kick) * (0.45 + 0.55 * groove) * track
              + vec3(1.0) * hl * 0.12 * groove
              + sheen * (0.07 + 0.16 * lv) * groove * track
              + vec3(0.14) * dust * (0.3 + hl);
    disc += mix(sheen, vec3(1.0), 0.5) * exp(-pow((r - 0.993) * R / (1.2 * u_ppp), 2.0)) * 0.45;
    // Étiquette : la pochette qui tourne (visible quand rien ne la recouvre).
    float lab = 0.33;
    if (r < lab + 0.01) {
        vec2 q = rot(spin) * (d / (R * lab));
        vec3 lc = cover_color(q * 0.5 + 0.5);
        float hole = smoothstep(0.03, 0.045, r);
        float inner = 1.0 - smoothstep(lab - 1.5 * u_ppp / R, lab, r);
        disc = mix(disc, lc * hole * (0.85 + 0.3 * u_kick), inner)
             + vec3(0.5) * exp(-pow((r - lab) * R / (1.0 * u_ppp), 2.0));
    }
    return acc * (1.0 - edge) + disc * edge;
}

// ---------- 29. Ferrofluide : un liquide sombre hérissé de pointes qui se dressent sur les graves ----------

vec3 ferrofluid(vec2 px, float W, float H) {
    vec2 d = px - u_focus.xy;
    float r = length(d);
    float a = atan(d.y, d.x) + u_time * 0.04;
    float base = u_focus.z * 1.16 * (1.0 + 0.04 * u_bass + 0.05 * u_kick);
    float N = floor(detail(12.0, 30.0, 60.0));
    float s = a / 6.28318 * N;
    float idx = floor(s);
    float local = fract(s) - 0.5;
    float fi = abs(mod(idx, N) / N * 2.0 - 1.0);
    float h = (0.05 + 0.60 * band(fi) + 0.25 * u_kick) * u_focus.z * (0.6 + 0.7 * hash(vec2(idx, 3.0)));
    float tri = max(1.0 - abs(local) * 2.0, 0.0);
    float profile = pow(tri, 2.4);
    float edge = base + h * profile + u_focus.z * 0.025 * sin(a * 3.0 + u_time * 0.7);
    // Distance au bord corrigée par la pente : un trait d'épaisseur régulière sur les pointes.
    float slope = h * 2.4 * pow(tri, 1.4) * 2.0 * N / 6.28318 / max(r, 1.0);
    float dd = (r - edge) / sqrt(1.0 + slope * slope);
    float rim = exp(-pow(dd / (1.6 * u_ppp), 2.0));
    float refl = 0.5 + 0.5 * sin(a * 2.0 - r / u_focus.z * 3.0 + u_time * 0.5);
    vec3 col = mix(u_colors[0], u_colors[1], refl);
    float inside = step(dd, 0.0) * smoothstep(-u_focus.z * 0.5, 0.0, dd);
    vec3 acc = mix(col, vec3(1.0), 0.35) * rim * (0.55 + 0.8 * u_energy)
             + col * 0.10 * inside
             + u_colors[2] * exp(-pow(max(dd, 0.0) / (u_focus.z * 0.4), 2.0)) * (0.08 + 0.4 * u_kick);
    acc += vec3(1.0) * rim * pow(profile, 8.0) * smoothstep(0.1, 0.45, h / u_focus.z) * 0.7;
    return acc;
}

// ---------- 30. Hologramme : une sphère de points qui tourne, ses latitudes suivent le spectre ----------

vec3 hologram(vec2 px, float W, float H) {
    float R = u_focus.z * 1.9 * (1.0 + 0.04 * u_bass + 0.06 * u_kick);
    vec2 q = (px - u_focus.xy) / R;
    float r2 = dot(q, q);
    vec3 acc = u_colors[2] * exp(-r2 * 1.6) * (0.05 + 0.12 * u_bass);
    acc += u_colors[1] * exp(-pow((sqrt(r2) - 1.0) * R / (H * 0.015), 2.0)) * (0.10 + 0.35 * u_kick);
    if (r2 >= 1.0) return acc;
    float z = sqrt(1.0 - r2);
    float rows = floor(detail(8.0, 15.0, 26.0));
    for (int side = 0; side < 2; side++) {
        vec3 p = vec3(q.x, -q.y, side == 0 ? z : -z);
        p.xz = rot(u_time * 0.22) * p.xz;
        p.yz = rot(0.38) * p.yz;
        float lat = asin(clamp(p.y, -1.0, 1.0));
        float lon = atan(p.z, p.x);
        float li = floor(lat / 3.14159 * rows + 0.5);
        float latc = li / rows * 3.14159;
        float cols = max(1.0, floor(cos(latc) * rows * 2.0 + 0.5));
        float lonc = floor(lon / 6.28318 * cols + 0.5) / cols * 6.28318;
        vec2 dv = vec2((lon - lonc) * cos(lat), lat - latc);
        float dist = length(dv) * R;
        float f = abs(li) / (rows * 0.5);
        float lv = band(f);
        float size = (1.8 + 3.6 * lv + 1.5 * u_kick) * u_ppp * (side == 0 ? 1.0 : 0.7);
        float dotv = exp(-pow(dist / size, 2.0));
        float halo = exp(-pow(dist / (size * 3.0), 2.0)) * 0.25;
        vec3 col = mix(u_colors[0], u_colors[1], f);
        float shade = side == 0 ? (0.45 + 0.55 * z) : 0.18;
        acc += mix(col, vec3(1.0), 0.4 * lv) * (dotv + halo) * shade * (1.1 + 2.2 * lv);
    }
    return acc;
}

// ---------- 31. Soie : un drapé qui ondule, des reflets qui glissent dessus ----------

float silk_h(vec2 p, float t) {
    return sin(p.x * 1.6 + 1.7 * sin(p.y * 1.1 + t * 0.45) + 0.6 * sin(p.y * 2.3 - t * 0.31 + p.x * 0.7))
         + 0.5 * sin(p.y * 0.9 + p.x * 0.4 - t * 0.2);
}
vec3 silk(vec2 px, float H) {
    vec2 p = px / (H * 0.24 * u_scale);
    float t = u_time * 0.6;
    float amp = (0.9 + 0.6 * u_bass + 0.4 * u_kick) * detail(0.35, 1.0, 1.9);
    float e = 0.01;
    float h = silk_h(p, t);
    vec2 g = vec2(silk_h(p + vec2(e, 0.0), t) - h, silk_h(p + vec2(0.0, e), t) - h) / e;
    vec3 n = normalize(vec3(-g * amp * 0.55, 1.0));
    vec3 l = normalize(vec3(cos(t * 0.3) * 0.6, -0.5 + 0.2 * sin(t * 0.21), 0.7));
    float diff = max(dot(n, l), 0.0);
    vec3 hv = normalize(l + vec3(0.0, 0.0, 1.0));
    float spec = pow(max(dot(n, hv), 0.0), 70.0);
    float sheen = pow(max(dot(n, hv), 0.0), 12.0);
    float tone = 0.5 + 0.5 * sin(h * 1.2 + p.x * 0.3);
    vec3 col = mix(u_colors[2], u_colors[0], tone);
    col = mix(col, u_colors[1], smoothstep(0.6, 1.0, tone) * 0.5);
    return col * (0.02 + 0.30 * pow(diff, 4.0) + 0.25 * sheen) * (0.8 + 0.5 * u_energy)
         + mix(u_colors[1], vec3(1.0), 0.6) * spec * (0.45 + 0.6 * u_kick + 0.3 * u_energy);
}

// ---------- 32. Irisé : une fine pellicule aux couleurs changeantes, comme une bulle de savon ----------

vec3 iridescence(vec2 px, float H) {
    vec2 p = px / (H * 0.55 * u_scale);
    float t = u_time * 0.10;
    vec2 w = vec2(fbm(p + vec2(t, 0.3)), fbm(p * 1.3 - vec2(0.7, t)));
    float th = fbm(p * 0.9 + 2.2 * w + vec2(-t * 0.6, t * 0.4)) + 0.25 * u_bass + 0.15 * u_kick;
    vec3 film = 0.5 + 0.5 * cos(6.28318 * (th * 2.4 + vec3(0.0, 0.33, 0.67)));
    vec3 tint = mix(u_colors[0], u_colors[1], w.x);
    vec3 col = mix(film, tint * (0.6 + 0.8 * film), 0.6);
    float ridge = 1.0 - abs(fract(th * detail(2.5, 6.0, 12.0)) - 0.5) * 2.0;
    float glint = pow(ridge, 10.0);
    // Une pellicule sombre où seules les nervures brillent vraiment.
    return col * (0.03 + 0.07 * th) * (0.8 + 0.5 * u_energy) + col * glint * (0.16 + 0.35 * u_kick + 0.2 * u_energy);
}

// ---------- 33. Topographie : courbes de niveau d'un relief qui se soulève avec la musique ----------

vec3 topography(vec2 px, float W, float H) {
    vec2 p = px / (H * 0.5 * u_scale);
    float t = u_time * 0.06;
    float h = fbm(p * 0.8 + vec2(t, -t * 0.7)) * 1.2;
    for (int i = 0; i < 6; i++) {
        float fi = float(i);
        vec2 c = vec2(hash(vec2(fi, 2.0)) * W / (H * 0.5 * u_scale), hash(vec2(fi, 5.0)) * 2.0 / u_scale)
               + 0.2 * vec2(sin(u_time * 0.1 + fi), cos(u_time * 0.08 + fi * 2.0));
        float lv = band(fi / 5.0);
        h += (0.10 + 0.65 * lv + 0.15 * u_kick) * exp(-dot(p - c, p - c) * 2.5);
    }
    float k = h * detail(6.0, 14.0, 28.0);
    float fw = max(fwidth(k), 1e-4);
    float d = abs(fract(k + 0.5) - 0.5);
    float line = 1.0 - smoothstep(0.5 * fw, 1.6 * fw, d);
    float major = step(mod(floor(k + 0.5), 5.0), 0.5);
    vec3 col = mix(u_colors[2], u_colors[0], smoothstep(0.3, 1.2, h));
    col = mix(col, u_colors[1], smoothstep(1.0, 1.7, h));
    return col * line * (0.30 + 0.45 * major + 0.35 * u_energy + 0.3 * u_kick) + col * 0.03 * h;
}

// ---------- 34. Verre : colonnes de verre dépoli, un reflet sur leur flanc ----------

vec3 glass(vec2 px, float W, float H, float u) {
    float n = floor(detail(10.0, 24.0, 48.0));
    float cell = floor(u * n);
    float cu = (cell + 0.5) / n;
    float lv = level(cu) * envelope(cu);
    float cw = W / n;
    float bw = cw * 0.62;
    float base = H * u_shape.x;
    float h = max(lv * H * 0.55 * u_shape.y, bw);
    vec2 q = vec2(px.x - (cell + 0.5) * cw, px.y - (base - h * 0.5));
    float rr = bw * 0.5;
    vec2 dq = abs(q) - vec2(bw * 0.5, h * 0.5) + rr;
    float sd = length(max(dq, 0.0)) + min(max(dq.x, dq.y), 0.0) - rr;
    float inside = 1.0 - smoothstep(-u_ppp, u_ppp, sd);
    float border = exp(-pow(sd / u_ppp, 2.0));
    float yy = clamp((base - px.y) / h, 0.0, 1.0);
    vec3 col = mix(u_colors[0], u_colors[1], yy);
    float frost = 0.6 + 0.4 * noise(px / (5.0 * u_ppp));
    float stripe = exp(-pow((q.x + bw * 0.22) / (bw * 0.09), 2.0)) * smoothstep(0.0, 0.6, yy);
    vec3 acc = col * inside * (0.10 + 0.25 * yy) * frost * (0.7 + 0.6 * u_kick)
             + mix(col, vec3(1.0), 0.6) * border * 0.55
             + vec3(1.0) * stripe * inside * 0.18;
    // Reflet : la même colonne arrondie, renversée sous la ligne de base, qui s'efface vite.
    float below = px.y - base;
    vec2 mq = vec2(q.x, (2.0 * base - px.y) - (base - h * 0.5));
    vec2 mdq = abs(mq) - vec2(bw * 0.5, h * 0.5) + rr;
    float msd = length(max(mdq, 0.0)) + min(max(mdq.x, mdq.y), 0.0) - rr;
    float mirror = (1.0 - smoothstep(-u_ppp, u_ppp, msd)) * step(0.0, below);
    acc += col * mirror * (0.05 + 0.16 * lv) * exp(-below / (H * 0.04 + h * 0.25));
    return acc;
}

// ---------- 35. Flux : traînées de lumière qui suivent un courant invisible ----------

vec3 flux(vec2 px, float H) {
    vec2 p = px / (H * 0.6 * u_scale);
    float t = u_time * 0.15;
    float ang = fbm(p * 0.45 + vec2(t * 0.3, -t * 0.2)) * 6.28318 * 1.3;
    vec2 dir = vec2(cos(ang), sin(ang));
    vec2 nrm = vec2(-dir.y, dir.x);
    float thin = detail(0.4, 1.0, 2.0);
    float streak = noise(vec2(dot(p, dir) * 0.6 - u_time * 0.6, dot(p, nrm) * 40.0 * thin));
    streak = pow(streak, 5.0);
    float fine = pow(noise(vec2(dot(p, dir) * 1.2 - u_time * 1.1, dot(p, nrm) * 110.0 * thin)), 7.0);
    vec3 col = mix(u_colors[0], u_colors[1], 0.5 + 0.5 * sin(ang));
    float lv = band(fract(ang / 6.28318));
    return col * (streak * (0.30 + 0.8 * lv + 0.5 * u_kick) + fine * (0.2 + 0.6 * u_energy))
         + u_colors[2] * 0.025;
}

// ---------- 36. Trame : une trame de points, comme une affiche imprimée, qui respire ----------

vec3 halftone(vec2 px, float W, float H) {
    float cell = max(H * u_scale / detail(16.0, 34.0, 64.0), 5.0 * u_ppp);
    mat2 m = rot(0.52);
    vec2 q = m * px / cell;
    vec2 id = floor(q);
    vec2 f = fract(q) - 0.5;
    vec2 cpix = rot(-0.52) * ((id + 0.5) * cell);
    vec2 uv = cpix / H;
    float t = u_time * 0.2;
    float field = fbm(uv * 1.6 + vec2(t, -t * 0.5));
    float dist = length(cpix - u_focus.xy) / H;
    float pulse = 0.5 + 0.5 * sin(dist * 9.0 - u_time * 2.0);
    float lv = band(clamp(cpix.x / W, 0.0, 1.0));
    float v = clamp(field * 0.9 + pulse * (0.12 + 0.35 * u_kick) + 0.35 * lv - 0.25, 0.0, 1.0);
    float aa = u_ppp / cell;
    float dotv = 1.0 - smoothstep(-aa, aa, length(f) - v * 0.5);
    vec3 col = mix(u_colors[0], u_colors[1], clamp(cpix.x / W, 0.0, 1.0));
    col = mix(col, u_colors[2], 0.35 * (1.0 - v));
    return col * dotv * (0.25 + 0.55 * v);
}

// ---------- 37. Lissajous : la forme d'onde tracée contre elle-même, une boucle qui se tord ----------

vec3 lissajous(vec2 px, float W, float H) {
    float R = u_focus.z * 1.8;
    vec2 q = (px - u_focus.xy) / R;
    vec3 acc = u_colors[2] * exp(-dot(q, q) * 1.2) * (0.04 + 0.12 * u_bass);
    float guide = exp(-pow((length(q) - 1.0) * R / (1.0 * u_ppp), 2.0));
    acc += u_colors[1] * guide * 0.10;
    if (dot(q, q) > 2.4) return acc;
    mat2 m = rot(u_time * 0.08);
    // Points lissés (moyenne de trois voisins) : une boucle souple plutôt qu'une ligne brisée ;
    // puis gain automatique, pour que la boucle garde sa taille, son faible ou fort.
    vec2 pts[64];
    float peak = 0.05;
    for (int i = 0; i < 64; i++) {
        int j = i * 2;
        int k = j + 37;
        if (k > 127) k -= 128;
        float x = (u_wave[max(j - 1, 0)] + 2.0 * u_wave[j] + u_wave[min(j + 1, 127)]) * 0.25;
        float y = (u_wave[max(k - 1, 0)] + 2.0 * u_wave[k] + u_wave[min(k + 1, 127)]) * 0.25;
        pts[i] = vec2(x, y);
        peak = max(peak, max(abs(x), abs(y)));
    }
    float gain = detail(0.72, 0.95, 1.25) / peak;
    float best = 1e9;
    for (int i = 1; i < 64; i++) {
        vec2 a = m * pts[i - 1] * gain;
        vec2 ab = m * pts[i] * gain - a;
        float h = clamp(dot(q - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
        best = min(best, length(q - a - ab * h));
    }
    float d = best * R;
    float a = 0.55 + 0.8 * u_energy + 0.6 * u_kick;
    float glow = H * (0.012 + 0.02 * u_energy);
    return acc + u_colors[0] * a * 0.45 * exp(-d * d / (glow * glow))
         + mix(u_colors[0], vec3(1.0), 0.55) * a * exp(-d * d / (1.1 * u_ppp * 1.1 * u_ppp));
}

// ---------- 38. Rétro : soleil rayé, montagnes du spectre et sol quadrillé qui défile ----------

vec3 retro(vec2 px, float W, float H) {
    float hy = H * 0.60;
    vec3 acc = u_colors[0] * exp(-abs(px.y - hy) / (H * 0.012)) * (0.5 + 0.4 * u_kick);
    if (px.y < hy) {
        vec2 sc = vec2(W * 0.5, hy - H * 0.20);
        float sr = H * (0.20 + 0.015 * u_bass + 0.025 * u_kick);
        float d = length(px - sc);
        float yy = (px.y - (sc.y - sr)) / (2.0 * sr);
        float sun = 1.0 - smoothstep(sr - u_ppp, sr + u_ppp, d);
        // Bandes du soleil, de plus en plus larges vers le bas, qui descendent lentement.
        float band_w = mix(0.0, 0.5, smoothstep(0.42, 1.0, yy));
        float stripes = step(band_w, fract(px.y / (H * 0.035) + u_time * 0.3));
        vec3 sunc = mix(u_colors[1], u_colors[0], clamp(yy, 0.0, 1.0));
        acc += mix(sunc, vec3(1.0), 0.2) * sun * stripes * (0.9 + 0.4 * u_energy);
        acc += u_colors[0] * exp(-pow(max(d - sr, 0.0) / (H * 0.10), 2.0)) * 0.22 * (1.0 - sun);
        // Montagnes : leur crête suit le spectre.
        float u = px.x / W;
        float ridge = hy - H * (0.015 + detail(0.03, 0.11, 0.24) * level(u) * envelope(u) + 0.03 * noise(vec2(u * 14.0, 1.0)));
        float inside = smoothstep(ridge - u_ppp, ridge + u_ppp, px.y);
        acc *= 1.0 - inside * 0.92;
        acc += u_colors[1] * exp(-pow((px.y - ridge) / (1.3 * u_ppp), 2.0)) * 0.9;
        acc += u_colors[2] * inside * 0.05;
    } else {
        float depth = (H * 0.5) / max(px.y - hy, 1.0);
        float gx = (px.x - W * 0.5) / W * depth * 7.0;
        float gz = depth * 1.4 + u_time * 1.4;
        float dx = 0.5 - abs(fract(gx) - 0.5);
        float dz = 0.5 - abs(fract(gz) - 0.5);
        float lx = 1.0 - smoothstep(0.0, fwidth(gx) * 1.5, dx);
        float lz = 1.0 - smoothstep(0.0, fwidth(gz) * 1.5, dz);
        float fade = smoothstep(0.0, H * 0.10, px.y - hy);
        float line = max(lx, lz) * (1.0 - smoothstep(0.0, 1.0, fwidth(gz) * 2.0));
        acc += mix(u_colors[0], u_colors[1], 0.35) * line * fade * (0.55 + 0.4 * u_energy + 0.5 * u_kick);
        acc += u_colors[2] * 0.06 * fade;
    }
    return acc;
}

// ---------- 39. Mosaïque : la pochette en carreaux qui pivotent et s'allument sur le rythme ----------

vec3 mosaic(vec2 px, float W, float H) {
    float cs = max(H * u_scale / detail(6.0, 13.0, 26.0), 8.0 * u_ppp);
    vec2 id = floor(px / cs);
    vec2 f = px / cs - id - 0.5;
    vec2 center = (id + 0.5) * cs;
    float h = hash(id);
    // Les carreaux pivotent à tour de rôle (vus de côté, ils s'amincissent).
    float flip = abs(cos(u_time * 0.5 + h * 6.28318 + id.x * 0.15));
    float lv = band(h);
    float size = 0.40 * (0.80 + 0.25 * lv);
    vec2 q = abs(f) - vec2(size * flip, size);
    float r = 0.08;
    float sd = length(max(q + r, 0.0)) - r + min(max(q.x, q.y), 0.0);
    float tile = 1.0 - smoothstep(-u_ppp / cs, u_ppp / cs, sd);
    vec2 uv = (center - vec2(W, H) * 0.5) / (H * 1.1) + 0.5;
    uv = 1.0 - abs(1.0 - mod(uv, 2.0));
    vec3 col = cover_color(uv);
    float l = dot(col, vec3(0.299, 0.587, 0.114));
    col = max(mix(vec3(l), col, 1.4), 0.0);
    float flash = u_kick * step(0.82, hash(id + floor(u_time * 2.0)));
    float shade = 0.45 + 0.55 * flip;
    return col * tile * shade * (0.45 + 0.6 * lv) + mix(col, vec3(1.0), 0.6) * tile * flash * 0.6;
}

// ---------- 40. Vitrail : des éclats de verre colorés que la musique éclaire par derrière ----------

vec3 stained(vec2 px, float W, float H) {
    vec2 p = px / (H * detail(0.38, 0.20, 0.10) * u_scale);
    vec2 i = floor(p);
    vec2 f = fract(p);
    float d1 = 8.0;
    float d2 = 8.0;
    vec2 id = vec2(0.0);
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 g = vec2(float(x), float(y));
            vec2 o = 0.5 + 0.35 * sin(u_time * 0.12 + 6.28318 * hash2(i + g));
            float d = length(g + o - f);
            if (d < d1) { d2 = d1; d1 = d; id = i + g; } else if (d < d2) { d2 = d; }
        }
    }
    float lead = smoothstep(0.04, 0.10, d2 - d1);
    float h = hash(id);
    float lv = band(h);
    vec3 glassc = mix(mix(u_colors[0], u_colors[1], h), u_colors[2], hash(id + 3.1) * 0.6);
    float light = 0.25 + 0.7 * lv + 0.5 * u_kick * step(0.7, hash(id + 7.0));
    float center = exp(-d1 * d1 * 2.2);
    float grain = 0.85 + 0.15 * noise(px / (3.0 * u_ppp));
    return glassc * lead * light * (0.30 + 0.70 * center) * grain * 0.8
         + mix(glassc, vec3(1.0), 0.5) * exp(-pow((d2 - d1 - 0.07) / 0.02, 2.0)) * 0.06;
}

// ---------- 41. Radar : un balayage qui révèle les fréquences comme des échos ----------

vec3 radar(vec2 px, float W, float H) {
    float R = u_focus.z * 2.1;
    vec2 d = px - u_focus.xy;
    float r = length(d) / R;
    float a = atan(d.y, d.x);
    float sweep = u_time * 1.3;
    float behind = mod(sweep - a, 6.28318);
    vec3 acc = vec3(0.0);
    if (r < 1.0) {
        acc += u_colors[0] * exp(-behind * 2.5) * (0.35 + 0.35 * u_energy) * (0.3 + 0.7 * r);
        float edge_line = exp(-pow(min(behind, 6.28318 - behind) * length(px - u_focus.xy) / (1.2 * u_ppp), 2.0));
        acc += mix(u_colors[0], vec3(1.0), 0.5) * edge_line * 0.8;
        float rings = exp(-pow((fract(r * 4.0 + 0.5) - 0.5) * R / 4.0 / (0.9 * u_ppp), 2.0));
        acc += u_colors[1] * rings * 0.25;
    }
    acc += u_colors[1] * exp(-pow((r - 1.0) * R / (1.2 * u_ppp), 2.0)) * 0.35;
    // Échos : seize fréquences à des places fixes, ravivées au passage du faisceau.
    int count = int(floor(detail(6.0, 16.0, 32.0)));
    for (int i = 0; i < 32; i++) {
        if (i >= count) break;
        float fi = float(i);
        float ba = fi / float(count) * 6.28318 + hash(vec2(fi, 4.0)) * 0.8;
        float br = 0.28 + 0.62 * hash(vec2(fi, 9.0));
        vec2 bp = u_focus.xy + vec2(cos(ba), sin(ba)) * br * R;
        float lv = u_levels[i - (i / 16) * 16];
        float since = mod(sweep - ba, 6.28318);
        float glow = exp(-since * 0.7) * (0.6 + 1.8 * lv);
        float dist = length(px - bp);
        float size = (3.0 + 8.0 * lv) * u_ppp;
        acc += mix(u_colors[0], vec3(1.0), 0.4) * glow * (exp(-pow(dist / size, 2.0)) + 0.25 * exp(-dist / (size * 4.0)));
    }
    return acc;
}

// ---------- 42. Hélice : une double hélice qui tourne, ses barreaux suivent le spectre ----------

vec3 helix(vec2 px, float W, float H) {
    float cy = H * 0.5;
    float A = H * (0.16 + 0.04 * u_bass + 0.03 * u_kick);
    float k = 6.28318 / (H * 0.55);
    float ph = px.x * k + u_time * 0.9;
    vec3 acc = vec3(0.0);
    for (int s = 0; s < 2; s++) {
        float sgn = s == 0 ? 1.0 : -1.0;
        float y = cy + sgn * A * sin(ph);
        float z = sgn * cos(ph);
        float slope = sgn * A * k * cos(ph);
        float dist = abs(px.y - y) / sqrt(1.0 + slope * slope);
        float w = (1.4 + 1.0 * (z * 0.5 + 0.5)) * u_ppp;
        float bright = 0.35 + 0.65 * (z * 0.5 + 0.5);
        vec3 col = s == 0 ? u_colors[0] : u_colors[1];
        acc += mix(col, vec3(1.0), 0.3) * exp(-pow(dist / w, 2.0)) * bright
             + col * exp(-pow(dist / (w * 6.0), 2.0)) * 0.15 * bright;
    }
    // Barreaux : un tous les 2,6 % de la largeur ; leur éclat suit le spectre.
    float sp = W / floor(detail(16.0, 38.0, 76.0));
    float xr = (floor(px.x / sp) + 0.5) * sp;
    float phr = xr * k + u_time * 0.9;
    float y1 = cy + A * sin(phr);
    float y2 = cy - A * sin(phr);
    float lv = level(xr / W);
    float inr = step(min(y1, y2), px.y) * step(px.y, max(y1, y2));
    float bar = exp(-pow((px.x - xr) / (1.1 * u_ppp), 2.0)) * inr;
    float depth = 0.4 + 0.6 * abs(sin(phr));
    acc += mix(u_colors[2], u_colors[0], (px.y - min(y1, y2)) / max(abs(y1 - y2), 1.0))
         * bar * (0.2 + 1.3 * lv + 0.4 * u_kick) * depth;
    return acc;
}

// ---------- 44. Constellation : des étoiles reliées quand elles se rapprochent ----------

vec3 constellation(vec2 px, float W, float H) {
    float cs = H * detail(0.28, 0.17, 0.09) * u_scale;
    vec2 p = px / cs;
    vec2 i = floor(p);
    vec2 pts[9];
    float lvs[9];
    int n = 0;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 g = i + vec2(float(x), float(y));
            vec2 h = hash2(g);
            pts[n] = g + 0.5 + 0.40 * vec2(sin(u_time * 0.25 * (0.5 + h.x) + h.y * 6.28318),
                                           cos(u_time * 0.22 * (0.5 + h.y) + h.x * 6.28318));
            lvs[n] = band(hash(g + 1.7));
            n++;
        }
    }
    vec3 acc = vec3(0.0);
    for (int a = 0; a < 9; a++) {
        float d = length(p - pts[a]) * cs;
        float size = (1.4 + 2.6 * lvs[a] + 1.2 * u_kick) * u_ppp;
        vec3 col = mix(u_colors[0], u_colors[1], fract(pts[a].x * 0.37));
        acc += mix(col, vec3(1.0), 0.5) * (exp(-pow(d / size, 2.0)) + 0.12 * exp(-d / (size * 4.0))) * (0.6 + lvs[a]);
        for (int b = a + 1; b < 9; b++) {
            vec2 ab = pts[b] - pts[a];
            float len = length(ab);
            if (len > 1.35) continue;
            float hh = clamp(dot(p - pts[a], ab) / dot(ab, ab), 0.0, 1.0);
            float dl = length(p - pts[a] - ab * hh) * cs;
            float strength = (1.0 - len / 1.35) * (0.25 + 0.6 * (lvs[a] + lvs[b]) * 0.5 + 0.3 * u_kick);
            acc += mix(u_colors[0], u_colors[1], 0.3) * exp(-pow(dl / (1.0 * u_ppp), 2.0)) * strength * 1.6;
        }
    }
    return acc;
}

// ---------- 46. Bulles : des bulles de savon qui montent, irisées, et brillent sur les coups ----------

vec3 bubbles(vec2 px, float W, float H) {
    vec3 acc = vec3(0.0);
    int count = int(floor(detail(6.0, 18.0, 36.0)));
    for (int i = 0; i < 36; i++) {
        if (i >= count) break;
        float fi = float(i);
        vec2 h = vec2(hash(vec2(fi, 1.1)), hash(vec2(fi, 8.3)));
        float r = H * (0.03 + 0.07 * hash(vec2(fi, 5.5))) * (1.0 + 0.15 * u_bass);
        float speed = 0.04 + 0.05 * h.y;
        float y = H + r - fract(u_time * speed + h.x) * (H + 2.0 * r);
        float x = W * h.x + sin(u_time * 0.6 + fi) * H * 0.04;
        vec2 d = px - vec2(x, y);
        float dist = length(d);
        float rim = exp(-pow((dist - r) / (1.2 * u_ppp + r * 0.04), 2.0));
        float inner = smoothstep(r, r * 0.6, dist) * 0.05;
        float ang = atan(d.y, d.x);
        vec3 film = 0.5 + 0.5 * cos(6.28318 * (ang / 6.28318 * 2.0 + u_time * 0.1 + fi * 0.3 + vec3(0.0, 0.33, 0.67)));
        vec3 col = mix(u_colors[i - (i / 3) * 3], film, 0.55);
        float lv = u_levels[i - (i / 16) * 16];
        vec2 spot = vec2(x, y) + vec2(-0.35, -0.4) * r;
        float spec = exp(-pow(length(px - spot) / (r * 0.18), 2.0));
        acc += col * (rim * (0.45 + 0.9 * lv + 0.6 * u_kick) + inner) + vec3(1.0) * spec * 0.35;
    }
    return acc;
}

// ---------- 47. Éclipse : la pochette en lune noire, la couronne du soleil autour ----------

vec3 eclipse(vec2 px, float W, float H) {
    vec2 d = px - u_focus.xy;
    float r = length(d);
    float a = atan(d.y, d.x);
    float r0 = u_focus.z * 1.12;
    float out_r = max(r - r0, 0.0) / H;
    float u = fract(a / 6.28318 + 0.25);
    float lv = level(u);
    // Filets de la couronne, qui s'étirent vers l'extérieur.
    float fil = fbm(vec2(a * 5.0, out_r * 6.0 - u_time * 0.25)) ;
    float fil2 = fbm(vec2(a * 11.0 + 3.0, out_r * 10.0 - u_time * 0.4));
    float reach = (0.10 + 0.20 * lv + 0.10 * u_kick) * detail(0.4, 1.0, 2.0);
    float corona = exp(-out_r / (reach * (0.5 + fil))) * (0.5 + 0.8 * fil2);
    vec3 acc = mix(u_colors[0], vec3(1.0), 0.25) * corona * step(r0, r) * (0.45 + 0.4 * u_energy);
    acc += u_colors[1] * exp(-out_r / 0.35) * 0.12;
    // Le bord éclairé, et un « diamant » qui en fait le tour.
    float rim = exp(-pow((r - r0) / (1.4 * u_ppp), 2.0));
    acc += mix(u_colors[1], vec3(1.0), 0.6) * rim * 0.9;
    float da = mod(a - u_time * 0.2 + 3.14159, 6.28318) - 3.14159;
    float diamond = exp(-pow(da * r0 / (H * 0.02), 2.0)) * exp(-pow((r - r0) / (H * 0.02), 2.0));
    acc += vec3(1.0) * diamond * (0.5 + 1.2 * u_kick);
    return acc;
}

// ---------- 48. Moiré : deux trames de cercles qui glissent l'une sur l'autre ----------

vec3 moire(vec2 px, float W, float H) {
    float sp = (detail(3.5, 6.0, 11.0) + 2.0 * u_bass) * u_ppp;
    vec2 c1 = vec2(W * (0.5 + 0.18 * sin(u_time * 0.21)), H * (0.5 + 0.15 * cos(u_time * 0.17)));
    vec2 c2 = vec2(W * (0.5 + 0.18 * sin(u_time * 0.19 + 2.0)), H * (0.5 + 0.15 * cos(u_time * 0.23 + 1.0)));
    float l1 = 0.5 + 0.5 * cos(length(px - c1) / sp * 6.28318);
    float l2 = 0.5 + 0.5 * cos(length(px - c2) / sp * 6.28318);
    float m = pow(l1 * l2, 3.0);
    // La figure de moiré elle-même (basse fréquence), qui porte la couleur.
    float beat = 0.5 + 0.5 * cos((length(px - c1) - length(px - c2)) / sp * 6.28318 * 0.5);
    float lv = band(beat);
    vec3 col = mix(u_colors[0], u_colors[1], beat);
    return col * m * (0.35 + 0.7 * lv + 0.4 * u_kick) + u_colors[2] * beat * 0.06;
}

// ---------- 49. Néon : des tubes de néon qui tournent autour de la pochette et grésillent ----------

float neon_tri(vec2 p, float r) {
    const float k = 1.7320508;
    p.x = abs(p.x) - r;
    p.y = p.y + r / k;
    if (p.x + k * p.y > 0.0) p = vec2(p.x - k * p.y, -k * p.x - p.y) / 2.0;
    p.x -= clamp(p.x, -2.0 * r, 0.0);
    return -length(p) * sign(p.y);
}
vec3 neon(vec2 px, float W, float H) {
    vec2 q = px - u_focus.xy;
    float s = u_focus.z;
    float pulse = 1.0 + 0.05 * u_bass + 0.06 * u_kick;
    float d0 = abs(length(q) - s * 1.32 * pulse);
    vec2 qt = rot(u_time * 0.15) * q;
    float d1 = abs(neon_tri(vec2(qt.x, -qt.y), s * 1.75 * pulse));
    vec2 qb = rot(-u_time * 0.11 + 0.4) * q;
    vec2 bq = abs(qb) - vec2(s * 1.9 * pulse);
    float d2 = abs(length(max(bq, 0.0)) + min(max(bq.x, bq.y), 0.0));
    vec3 acc = vec3(0.0);
    for (int i = 0; i < 3; i++) {
        float d = i == 0 ? d0 : (i == 1 ? d1 : d2);
        float lv = band(float(i) / 2.0);
        // Grésillement : de temps en temps, un tube vacille.
        float tick = floor(u_time * 9.0);
        float flicker = step(0.06, hash(vec2(tick, float(i)))) * (0.75 + 0.25 * hash(vec2(tick, float(i) + 9.0)));
        vec3 col = u_colors[i];
        float core = exp(-pow(d / (1.6 * u_ppp), 2.0));
        float halo = exp(-d / (H * detail(0.006, 0.018, 0.04)));
        acc += (mix(col, vec3(1.0), 0.55) * core + col * halo * 0.35) * flicker * (0.45 + 1.0 * lv);
    }
    return acc;
}

// ================= Effets retravaillés =================

// ---------- 0. Aurore : rideaux de lumière aux rayons verticaux, comme une aurore boréale ----------

vec3 aurora(vec2 px, float W, float H, float u) {
    float env = envelope(u);
    float lv = level(u);
    float fine = detail(0.45, 1.0, 2.4);
    vec3 acc = vec3(0.0);
    for (int k = 0; k < 3; k++) {
        float fk = float(k);
        float freq = k == 0 ? 1.1 : (k == 1 ? 1.7 : 0.8);
        float speed = k == 0 ? 0.35 : (k == 1 ? 0.5 : 0.25);
        // L'ourlet du rideau ondule ; le rideau monte d'autant plus haut que sa fréquence est forte.
        float hem = H * (u_shape.x - 0.045 * fk) + wave(u, freq, speed, fk * 2.1) * H * 0.085 * u_shape.y * max(env, 0.25);
        float hgt = H * u_shape.y * (0.20 + 0.85 * lv + 0.18 * u_kick) * max(env, 0.12) * (1.0 - 0.16 * fk);
        float dy = hem - px.y;  // > 0 au-dessus de l'ourlet
        float body = smoothstep(-1.5 * u_ppp, 1.5 * u_ppp, dy) * exp(-max(dy, 0.0) / max(hgt, 1.0));
        // Rayons : stries verticales qui glissent le long du rideau.
        float sx = px.x / (7.0 * u_ppp) * fine;
        float rays_ = 0.35 + 0.65 * noise(vec2(sx + u_time * (0.5 + 0.25 * fk), fk * 7.3 + dy / (H * 0.9)));
        rays_ *= 0.65 + 0.35 * noise(vec2(sx * 0.21 - u_time * 0.18, fk + 3.0));
        vec3 col = mix(u_colors[k], u_colors[2], 0.65 * smoothstep(0.0, hgt * 1.7, dy));
        float a = (0.30 + 0.80 * lv + 0.50 * u_kick) * max(env, 0.2) * (1.0 - 0.18 * fk);
        acc += col * body * rays_ * a;
        acc += mix(col, vec3(1.0), 0.5) * exp(-pow(dy / (1.7 * u_ppp), 2.0)) * a * 0.75;
    }
    // Lueur au sol, sous les rideaux.
    float base = H * u_shape.x;
    acc += u_colors[2] * exp(-pow((px.y - base - 0.05 * H) / (0.06 * H), 2.0)) * env * (0.10 + 0.25 * u_bass + 0.3 * u_kick);
    return acc;
}

// ---------- 3. Spectre : colonnes arrondies, coiffe lumineuse, halo et reflet ----------

vec3 spectrum_bars(vec2 px, float W, float H, float u) {
    float n = floor(detail(20.0, 48.0, 110.0));
    float cw = W / n;
    float cell = clamp(floor(px.x / cw), 0.0, n - 1.0);
    float cu = (cell + 0.5) / n;
    float lv = level(cu) * envelope(cu);
    float base = H * u_shape.x;
    float h = lv * H * 0.55 * u_shape.y + 3.0 * u_ppp;
    float bw = cw * 0.66;
    float rr = min(bw * 0.5, 5.0 * u_ppp);
    float qx = px.x - (cell + 0.5) * cw;
    // Boîte arrondie en haut, coupée net à la ligne de base.
    vec2 q = vec2(qx, px.y - (base - h * 0.5 + rr * 0.5));
    vec2 dq = abs(q) - vec2(bw * 0.5, (h + rr) * 0.5) + rr;
    float sd = length(max(dq, 0.0)) + min(max(dq.x, dq.y), 0.0) - rr;
    float body = (1.0 - smoothstep(-u_ppp, u_ppp, sd)) * step(px.y, base);
    float yy = clamp((base - px.y) / max(h, 1.0), 0.0, 1.0);
    vec3 col = mix(u_colors[0], u_colors[1], yy);
    col = mix(col, vec3(1.0), smoothstep(0.7, 1.0, yy) * 0.35);
    vec3 acc = col * body * (0.50 + 0.30 * yy + 0.5 * u_kick);
    // Coiffe : le haut de chaque colonne brille.
    float cap = exp(-pow((px.y - (base - h)) / (1.4 * u_ppp), 2.0)) * (1.0 - smoothstep(bw * 0.5 - rr, bw * 0.5, abs(qx)));
    acc += mix(u_colors[1], vec3(1.0), 0.6) * cap * 0.6;
    // Halo derrière, qui suit la courbe lissée du spectre.
    float dy = base - px.y;
    float smooth_h = level(u) * envelope(u) * H * 0.55 * u_shape.y;
    acc += u_colors[0] * exp(-pow(max(dy - smooth_h, 0.0) / (H * 0.07), 2.0)) * step(0.0, dy) * (0.05 + 0.10 * level(u));
    // Reflet, de la forme de la colonne, qui s'efface vite.
    float below = px.y - base;
    float refl = step(0.0, below) * (1.0 - smoothstep(bw * 0.5 - u_ppp, bw * 0.5 + u_ppp, abs(qx)))
               * smoothstep(h * 0.55, 0.0, below) * exp(-below / (H * 0.05));
    acc += mix(u_colors[0], u_colors[1], 0.2) * refl * 0.22;
    return acc;
}

// ---------- 5. Lignes : relief de crêtes empilées, chaque ligne cachant celles de derrière ----------

vec3 lines(vec2 px, float W, float H, float u) {
    vec3 acc = vec3(0.0);
    float n = floor(detail(10.0, 20.0, 34.0));
    // Les crêtes se concentrent au centre, comme sur la célèbre pochette.
    float centre = pow(max(1.0 - abs(u * 2.0 - 1.0) / 0.78, 0.0), 1.6);
    for (int i = 0; i < 34; i++) {
        if (float(i) >= n) break;
        float fi = float(i) / (n - 1.0);
        float y_base = H * (0.24 + 0.68 * fi);
        float fl = float(i);
        float ridge = noise(vec2(u * 11.0 + fl * 3.1, u_time * 0.45 + fl * 1.7));
        ridge = ridge * ridge * (0.6 + 0.8 * noise(vec2(u * 31.0 - fl, u_time * 0.8)));
        float bump = centre * (0.25 + 1.3 * level(u)) * ridge * (1.0 + 0.5 * u_kick);
        float y = y_base - H * 0.20 * u_shape.y * bump;
        // Ce qui est sous la ligne est caché : les lignes de derrière ne la traversent pas.
        acc *= 1.0 - 0.93 * smoothstep(-u_ppp, u_ppp, px.y - y);
        float d = abs(px.y - y);
        float core = (0.8 + 0.5 * fi) * u_ppp;
        vec3 col = mix(mix(u_colors[2], u_colors[0], 0.35 + 0.65 * fi), u_colors[1], clamp(bump * 1.4, 0.0, 1.0));
        acc += col * (0.35 + 0.65 * fi) * (exp(-(d * d) / (core * core)) + 0.10 * exp(-d / (5.0 * u_ppp)));
    }
    return acc;
}

// ---------- 7. Faisceaux : projecteurs de scène, brume dans les faisceaux, tache au sol ----------

vec3 beams(vec2 px, float W, float H) {
    vec3 acc = vec3(0.0);
    float n = floor(detail(3.0, 6.0, 10.0));
    for (int k = 0; k < 10; k++) {
        if (float(k) >= n) break;
        float fk = float(k);
        vec2 o = vec2(W * (fk + 0.5) / n, 0.07 * H);
        float a = sin(u_time * (0.22 + 0.05 * hash(vec2(fk, 1.0))) + fk * 1.7) * 0.55;
        vec2 dir = vec2(sin(a), cos(a));
        vec2 rel = px - o;
        float along = dot(rel, dir);
        float across = abs(rel.x * dir.y - rel.y * dir.x);
        float width = 0.012 * H + along * 0.085;
        float lvk = u_levels[k + k / 2];
        float power = 0.35 + 1.1 * lvk + 0.5 * u_kick;
        // Bord doux et cœur plus vif ; la brume fait vivre le faisceau.
        float t = across / max(width, 1.0);
        float shape = exp(-t * t * 1.6) + 0.5 * exp(-t * t * 14.0);
        float haze = 0.55 + 0.45 * fbm(vec2(along / (H * 0.22) - u_time * 0.25, across / (H * 0.10) + fk * 5.0));
        float reach = smoothstep(0.0, 0.03 * H, along) * exp(-along / (1.25 * H));
        vec3 col = u_colors[k - (k / 3) * 3];
        acc += col * shape * haze * reach * power * 0.9;
        // La lampe, en haut, et la tache de lumière au sol.
        acc += mix(col, vec3(1.0), 0.6) * exp(-dot(rel, rel) / pow(0.022 * H, 2.0)) * (0.6 + power);
        float hit_x = o.x + tan(a) * (H - o.y);
        vec2 fd = (px - vec2(hit_x, H)) / vec2(0.16 * H, 0.035 * H);
        acc += col * exp(-dot(fd, fd)) * power * 0.35;
    }
    return acc;
}

// ---------- 9. Rayons : grand soleil levant, deux couronnes de rayons qui suivent le spectre ----------

vec3 rays(vec2 px, float W, float H) {
    vec2 c = vec2(0.5 * W, H * 1.02);
    vec2 d = px - c;
    float r = length(d) / H;
    float ang = atan(d.x, -d.y);  // 0 vers le haut
    float u = clamp(0.5 + ang / 3.14159, 0.0, 1.0);
    float n = floor(detail(18.0, 44.0, 90.0));
    vec3 acc = vec3(0.0);
    for (int layer = 0; layer < 2; layer++) {
        float nn = layer == 0 ? n : n * 2.0;
        float su = u + (layer == 0 ? 0.004 : -0.006) * sin(u_time * 0.3);
        float lv = level(floor(su * nn) / nn + 0.5 / nn);
        float edge = abs(fract(su * nn) - 0.5) * 2.0;
        float ray = 1.0 - smoothstep(layer == 0 ? 0.22 : 0.10, layer == 0 ? 0.62 : 0.34, edge);
        // Les rayons vont loin : jusqu'en haut de la page sur les fréquences fortes.
        float reach = (layer == 0 ? 0.50 : 0.32) + (layer == 0 ? 1.15 : 0.75) * lv + 0.12 * u_kick;
        float body = ray * smoothstep(reach, reach * 0.25, r) * smoothstep(0.10, 0.20, r);
        vec3 col = mix(u_colors[layer], u_colors[1 - layer], clamp(r / max(reach, 0.01), 0.0, 1.0));
        acc += col * body * (layer == 0 ? 0.42 : 0.30) * (0.8 + 0.6 * u_kick);
    }
    // Le soleil : disque, couronne et grande lueur.
    float sun_r = 0.13 + 0.015 * u_bass + 0.02 * u_kick;
    acc += mix(u_colors[1], vec3(1.0), 0.5) * (1.0 - smoothstep(sun_r - 0.004, sun_r + 0.004, r)) * (0.75 + 0.5 * u_kick);
    acc += u_colors[1] * exp(-pow(max(r - sun_r, 0.0) / 0.07, 2.0)) * 0.45;
    acc += u_colors[2] * exp(-r * r * 2.2) * (0.14 + 0.2 * u_bass);
    return acc;
}

// ---------- 13. Oscilloscope : la forme d'onde sur un écran à réticule, avec sa rémanence ----------

float wave_at(float u) {
    float x = clamp(u, 0.0, 1.0) * 127.0;
    int i = int(floor(x));
    return mix(u_wave[i], u_wave[min(i + 1, 127)], fract(x));
}
vec3 scope(vec2 px, float W, float H, float u) {
    float base = H * u_shape.x;
    float amp = H * 0.24 * u_shape.y * detail(0.45, 1.0, 1.9);
    float env = envelope(u);
    float w0 = wave_at(u) * env;
    float y0 = base - w0 * amp;
    float y1 = base - wave_at(u + 1.0 / W) * amp * envelope(u + 1.0 / W);
    float slope = y1 - y0;
    float d = abs(px.y - y0) / sqrt(1.0 + slope * slope);
    float a = (0.55 + 0.8 * u_energy + 0.6 * u_kick) * max(env, 0.15);
    vec3 col = u_colors[0];
    vec3 acc = vec3(0.0);
    // Réticule : quadrillage fin et axe central, comme sur l'écran d'un appareil de mesure.
    float gs = H * 0.07;
    vec2 g = abs(fract(vec2(px.x, px.y - base) / gs + 0.5) - 0.5) * gs;
    float in_screen = smoothstep(amp * 1.9, amp * 1.2, abs(px.y - base)) * env;
    acc += u_colors[2] * (exp(-pow(min(g.x, g.y) / (0.7 * u_ppp), 2.0)) * 0.10
         + exp(-pow((px.y - base) / (0.8 * u_ppp), 2.0)) * 0.22) * in_screen;
    // Remplissage entre la trace et l'axe : la « matière » du son.
    float between = step(min(y0, base), px.y) * step(px.y, max(y0, base));
    acc += col * between * 0.16 * a * (0.4 + 0.6 * abs(px.y - base) / max(abs(y0 - base), 1.0));
    // La trace : lueur large, halo, cœur presque blanc.
    float glow = H * (0.018 + 0.03 * u_energy);
    acc += col * a * 0.45 * exp(-d * d / (glow * glow));
    acc += mix(col, vec3(1.0), 0.6) * a * exp(-d * d / (1.5 * u_ppp * 1.5 * u_ppp));
    // Rémanence : la même trace, pâle, en miroir sous l'axe.
    float ym = base + w0 * amp * 0.55;
    acc += u_colors[1] * a * 0.16 * exp(-pow((px.y - ym) / (2.2 * u_ppp), 2.0));
    return acc;
}

// ---------- 16. Horizon : route de lumière entre deux reliefs, soleil rayé au fond ----------

vec3 horizon(vec2 px, float W, float H, float u) {
    float hy = H * 0.50;
    vec3 acc = vec3(0.0);
    // Soleil : demi-disque rayé posé sur l'horizon.
    vec2 sc = vec2(0.5 * W, hy);
    float sr = H * (0.17 + 0.02 * u_bass + 0.02 * u_kick);
    float sd = length(px - sc);
    if (px.y <= hy) {
        float sun = 1.0 - smoothstep(sr - u_ppp, sr + u_ppp, sd);
        float yy = (hy - px.y) / sr;
        float stripes = step(mix(0.55, 0.0, yy), fract((hy - px.y) / (H * 0.028) - u_time * 0.25));
        acc += mix(u_colors[0], u_colors[1], yy) * sun * stripes * (0.75 + 0.4 * u_energy);
        acc += u_colors[0] * exp(-pow(max(sd - sr, 0.0) / (H * 0.12), 2.0)) * 0.20 * (1.0 - sun);
    }
    acc += u_colors[1] * exp(-abs(px.y - hy) / (1.2 * u_ppp)) * 0.5;
    if (px.y > hy - 1.0) {
        float relief = detail(0.45, 1.0, 1.9);
        float speed = 0.5;
        float scroll = fract(u_time * speed);
        float row0 = floor(u_time * speed);
        // Une vallée plate au milieu (la route), le relief sur les côtés.
        float side = smoothstep(0.10, 0.42, abs(u - 0.5));
        // Du fond vers l'avant : chaque rangée cache celles de derrière.
        for (int j = 0; j < 18; j++) {
            int i = 17 - j;
            float z = float(i) + 1.0 - scroll;
            float id = float(i) + row0;  // la rangée garde son relief en avançant
            float terrain = (0.35 + 1.1 * level(u)) * noise(vec2(u * 7.0, id * 0.9)) + 0.25 * noise(vec2(u * 23.0, id * 1.7));
            float y = hy + (H - hy) / z - side * terrain * relief * H * 0.26 / sqrt(z) * (1.0 + 0.35 * u_kick);
            acc *= 1.0 - 0.85 * smoothstep(-u_ppp, u_ppp, px.y - y) * smoothstep(0.0, 1.0, z);
            float d = abs(px.y - y);
            float core = (0.7 + 1.3 / z) * u_ppp;
            float fade = smoothstep(18.0, 5.0, z) * smoothstep(0.0, 1.0, z);
            vec3 col = mix(u_colors[0], u_colors[2], clamp(z / 16.0, 0.0, 1.0));
            acc += col * fade * (exp(-d * d / (core * core)) + 0.12 * exp(-d / (6.0 * u_ppp)));
        }
        // Lignes de fuite, seulement sur la route.
        float k = (px.x - 0.5 * W) / max(px.y - hy, 1.0);
        float g = abs(fract(k * 0.8 + 0.5) - 0.5) / 0.8 * (px.y - hy);
        float vfade = smoothstep(hy, H, px.y) * (1.0 - side);
        acc += u_colors[1] * exp(-g * g / (u_ppp * u_ppp)) * vfade * 0.45;
    }
    return acc;
}

// ---------- 20. Pluie de lumière : gouttes qui tombent, leur traînée sort de l'écran avec elles ----------

vec3 light_rain(vec2 px, float W, float H) {
    vec3 acc = vec3(0.0);
    float cols = max(floor(W / (detail(26.0, 14.0, 7.0) * u_ppp)), 8.0);
    for (int layer = 0; layer < 2; layer++) {
        float fl = float(layer);
        float n = cols * (layer == 0 ? 1.0 : 0.6);
        float x = px.x / W * n;
        float ci = floor(x);
        float dx = abs(fract(x) - 0.5) * W / n;
        float width = (0.9 + 0.6 * fl) * u_ppp;
        float line = exp(-dx * dx / (width * width));
        float lv = level((ci + 0.5) / n);
        vec3 col = mix(u_colors[layer], vec3(1.0), 0.15);
        // Deux gouttes par colonne. La longueur de la traînée est fixe (c'est son éclat qui suit
        // la musique) et la course va assez loin sous l'écran pour qu'elle en sorte en entier.
        for (int j = 0; j < 2; j++) {
            float seed = hash(vec2(ci, fl * 13.0 + float(j) * 5.0));
            float len = 0.16 + 0.14 * seed;
            float speed = (0.10 + 0.20 * seed) * (1.0 + 0.5 * fl);
            float head = fract(u_time * speed + seed * 7.0 + float(j) * 0.5) * (1.12 + 5.0 * len) - 0.10;
            float d = head - px.y / H;  // > 0 au-dessus de la tête de la goutte
            float trail = step(0.0, d) * exp(-d / len);
            float tip = exp(-pow(d * H / (4.0 * u_ppp), 2.0));
            acc += col * line * (trail * (0.25 + 1.0 * lv) + tip * 1.2) * (layer == 0 ? 1.0 : 0.5) * (0.7 + 0.5 * u_kick);
            // Éclat au sol quand la goutte touche le bas.
            float splash = exp(-pow((head - 1.0) / 0.03, 2.0)) * step(1.0, head + 0.03);
            vec2 sp = vec2((px.x - (ci + 0.5) * W / n) / (10.0 * u_ppp), (px.y - H) / (3.0 * u_ppp));
            acc += col * splash * exp(-dot(sp, sp)) * (0.5 + lv);
        }
    }
    return acc;
}

// ---------- 21. Kaléidoscope : miroirs autour de la pochette, facettes et filets de lumière ----------

vec3 kaleidoscope(vec2 px, float W, float H) {
    vec2 p = (px - u_focus.xy) / H;
    float r = length(p);
    float segs = floor(detail(4.0, 8.0, 16.0));
    float seg = 6.28318 / segs;
    float a = mod(atan(p.y, p.x) + u_time * 0.05, seg);
    a = abs(a - seg * 0.5);
    vec2 q = vec2(cos(a), sin(a)) * r / u_scale;
    float t = u_time * 0.15;
    float n = fbm(q * 4.0 + vec2(t, -t * 0.7) + 1.5 * fbm(q * 3.0 - t));
    // Facettes : un réseau de cellules pris dans les miroirs, dont les arêtes brillent.
    float e = cell_edge(q * 7.0 + vec2(t * 0.6, 0.0), u_time * 0.3);
    float facet = exp(-e / 0.05);
    float lines_ = exp(-pow((fract(n * 6.0) - 0.5) / 0.08, 2.0));
    vec3 col = mix(u_colors[2], u_colors[0], smoothstep(0.3, 0.7, n));
    col = mix(col, u_colors[1], smoothstep(0.6, 0.9, n));
    float lv = level(fract(r * 1.6));
    float pulse = 0.6 + 0.4 * u_bass + 0.7 * u_kick;
    // Arêtes des miroirs, très fines.
    float mirror = exp(-pow(a * r * H / (1.0 * u_ppp), 2.0)) + exp(-pow((seg * 0.5 - a) * r * H / (1.0 * u_ppp), 2.0));
    return (col * (0.20 + 0.8 * n * n)
          + mix(col, vec3(1.0), 0.4) * lines_ * 0.30 * (0.5 + lv)
          + mix(u_colors[1], vec3(1.0), 0.5) * facet * 0.28 * (0.4 + 1.2 * lv)
          + u_colors[1] * mirror * 0.10) * pulse * smoothstep(1.7, 0.25, r);
}

// ---------- 22. Rosace : trois couronnes de pétales autour de la pochette (graves, médiums, aigus) ----------

vec3 rosette(vec2 px, float W, float H) {
    vec2 d = px - u_focus.xy;
    float r = length(d);
    float a = atan(d.y, d.x);
    float R = u_focus.z;
    vec3 acc = u_colors[2] * exp(-pow(r / (R * 2.8), 2.0)) * (0.10 + 0.22 * u_bass + 0.2 * u_kick);
    float petals = floor(detail(3.0, 6.0, 12.0));
    for (int k = 0; k < 3; k++) {
        float fk = float(k);
        float n = petals * (k == 0 ? 1.0 : (k == 1 ? 2.0 : 4.0));
        float lv = k == 0 ? max(band(0.06), u_kick * 0.8) : (k == 1 ? band(0.45) : band(0.85));
        float turn = u_time * (k == 1 ? -0.09 : 0.06) * (1.0 + 0.4 * fk);
        float c = abs(cos((a + turn) * n * 0.5));
        float shape = pow(c, 0.7);
        float r_in = R * (1.10 - 0.02 * fk);
        float r_out = R * ((k == 0 ? 1.50 : (k == 1 ? 1.34 : 1.20)) + (k == 0 ? 0.95 : (k == 1 ? 0.75 : 0.50)) * lv);
        float edge = r_in + (r_out - r_in) * shape;
        // Pente du contour, pour un trait d'épaisseur régulière.
        float de = (r_out - r_in) * 0.7 * pow(max(c, 0.02), -0.3) * abs(sin((a + turn) * n * 0.5)) * n * 0.5 / max(r, 1.0);
        float sd = (r - edge) / sqrt(1.0 + de * de);
        vec3 col = mix(u_colors[k], vec3(1.0), 0.25);
        float outline = exp(-pow(sd / (1.5 * u_ppp), 2.0));
        float fill = step(sd, 0.0) * step(r_in, r) * (0.06 + 0.10 * (r - r_in) / max(r_out - r_in, 1.0));
        acc += col * (outline * (0.45 + 0.8 * lv) + fill * (0.5 + 1.2 * lv));
        acc += u_colors[k] * exp(-pow(max(sd, 0.0) / (R * 0.10), 2.0)) * step(0.0, sd) * 0.10 * lv;
    }
    acc += mix(u_colors[1], vec3(1.0), 0.4) * exp(-pow((r - R * 1.08) / (1.5 * u_ppp), 2.0)) * 0.5;
    return acc;
}

// ---------- 24. Galaxie : bras d'étoiles, bandes de poussière, nébuleuses et bulbe central ----------

vec3 galaxy(vec2 px, float W, float H) {
    vec2 p = (px - vec2(0.5 * W, 0.5 * H)) / H;
    p = rot(0.35) * p * 0.62;
    p.y *= 1.9;  // vue inclinée
    float r = length(p);
    float a = atan(p.y, p.x);
    float t = u_time * 0.06;
    float wind = detail(2.2, 4.0, 7.5);
    // Deux bras en spirale logarithmique, épaissis et froissés par du bruit.
    float spiral = 2.0 * (a + t * 3.0) - log(r + 0.015) * wind;
    float wobble = fbm(vec2(a * 3.0 + t, r * 9.0)) - 0.5;
    float arm = 0.5 + 0.5 * cos(spiral + wobble * 2.2);
    float falloff = exp(-r * 3.2);
    float disc = falloff * (0.18 + 0.82 * pow(arm, 2.5));
    // Bandes de poussière : sombres, sur le bord intérieur des bras.
    float dust = fbm(vec2(spiral * 0.8, r * 14.0 - t * 2.0));
    float lane = smoothstep(0.35, 0.75, dust) * pow(0.5 + 0.5 * cos(spiral + 1.1), 3.0);
    vec3 col = mix(mix(u_colors[1], vec3(1.0, 0.94, 0.86), 0.5), u_colors[0], clamp(r * 2.4, 0.0, 1.0));
    vec3 acc = col * disc * (1.0 - 0.75 * lane) * (0.75 + 0.5 * u_bass + 0.4 * u_kick);
    // Nébuleuses : nœuds colorés le long des bras, qui pulsent avec leurs fréquences.
    float knots = smoothstep(0.62, 0.9, fbm(vec2(spiral * 1.3 + 4.0, r * 22.0))) * pow(arm, 4.0);
    acc += mix(u_colors[2], u_colors[1], 0.5) * knots * falloff * 2.2 * (0.4 + 1.4 * band(clamp(r * 2.0, 0.0, 1.0)));
    // Étoiles : trois tailles de grille ; plus nombreuses et plus vives dans les bras.
    vec2 sp = rot(t * 3.0) * p;
    for (int s = 0; s < 3; s++) {
        float sc = s == 0 ? 26.0 : (s == 1 ? 55.0 : 110.0);
        vec2 gq = sp * sc;
        vec2 gi = floor(gq);
        vec2 gf = fract(gq) - 0.5 - (hash2(gi) - 0.5) * 0.6;
        float hsh = hash(gi + float(s) * 17.0);
        float dens = 0.10 + 0.9 * disc * 2.5 + 0.25 * exp(-r * 1.2);
        float star = step(1.0 - 0.30 * clamp(dens, 0.0, 1.0), hsh) * exp(-dot(gf, gf) * (s == 0 ? 60.0 : 28.0));
        float tw = 0.6 + 0.4 * sin(u_time * (1.5 + 3.0 * hsh) + hsh * 40.0);
        acc += mix(vec3(1.0), col, 0.35) * star * tw * (s == 0 ? 0.9 : (s == 1 ? 0.55 : 0.35)) * (0.6 + 0.8 * u_energy);
    }
    // Bulbe central : cœur chaud, légèrement allongé (une barre).
    vec2 bp = rot(t * 3.0 + 0.6) * p;
    float bulge = exp(-(bp.x * bp.x * 40.0 + bp.y * bp.y * 110.0)) + 0.8 * exp(-r * r * 220.0);
    acc += mix(u_colors[1], vec3(1.0, 0.95, 0.88), 0.7) * bulge * (0.7 + 0.8 * u_kick);
    return acc;
}

// ---------- 27. Diodes : vumètre à segments, comme la façade d'une chaîne hi-fi ----------

vec3 leds(vec2 px, float W, float H, float u) {
    float cols = floor(detail(14.0, 30.0, 60.0));
    float cw = W / cols;
    float seg_h = 6.0 * u_ppp;
    float pitch = seg_h + 3.0 * u_ppp;
    float base = H * u_shape.x;
    float col_i = clamp(floor(px.x / cw), 0.0, cols - 1.0);
    float row = floor((base - px.y) / pitch);
    float cu = (col_i + 0.5) / cols;
    float lv = level(cu) * envelope(cu);
    float most = max(floor(H * 0.55 * u_shape.y / pitch), 3.0);
    float lit = floor(lv * most + 0.5 * u_kick);
    // Segment : un petit rectangle arrondi, plus large que haut.
    vec2 q = vec2(px.x - (col_i + 0.5) * cw, (base - px.y) - (row + 0.5) * pitch);
    vec2 half_ = vec2(cw * 0.5 - max(1.5 * u_ppp, cw * 0.09), seg_h * 0.5);
    float rr = min(2.0 * u_ppp, half_.y);
    vec2 dq = abs(q) - half_ + rr;
    float sd = length(max(dq, 0.0)) + min(max(dq.x, dq.y), 0.0) - rr;
    float seg = 1.0 - smoothstep(-0.7 * u_ppp, 0.7 * u_ppp, sd);
    float inside = step(0.0, row) * step(row, most - 1.0);
    float on = inside * step(row, lit - 1.0);
    float peak = inside * step(lit - 1.5, row) * step(row, lit - 0.5);
    float height = row / most;
    // Couleur par zone, comme un vrai vumètre : calme, fort, puis rouge-blanc tout en haut.
    vec3 col = mix(u_colors[0], u_colors[1], smoothstep(0.35, 0.7, height));
    col = mix(col, vec3(1.0, 0.92, 0.85), smoothstep(0.78, 0.95, height) * 0.7);
    vec3 acc = col * seg * (on * (0.85 + 0.5 * u_kick) + peak * 0.5 + inside * 0.045);
    // Halo des segments allumés (la lumière déborde un peu).
    acc += col * on * exp(-pow(max(sd, 0.0) / (2.5 * u_ppp), 2.0)) * 0.16;
    // Reflet sur la façade, sous la ligne de base.
    float below = px.y - base;
    acc += u_colors[0] * step(0.0, below) * (1.0 - smoothstep(half_.x - u_ppp, half_.x + u_ppp, abs(q.x)))
         * exp(-below / (H * 0.03)) * lv * 0.18;
    return acc;
}

// ---------- 43. Pendules : une rangée de pendules reliés par un fil de lumière, qui ondule et se défait ----------

float pendulum_y(float i, float n, float H) {
    float lv = level((i + 0.5) / n);
    float amp = H * (0.16 + 0.14 * lv);
    float freq = 0.30 * (1.0 + i / n * 0.7);
    return H * 0.56 + amp * sin(u_time * freq * 6.28318 * 0.5);
}
vec3 pendulums(vec2 px, float W, float H) {
    float n = floor(detail(9.0, 18.0, 36.0));
    float cw = W / n;
    float fx = px.x / cw - 0.5;
    float ci = floor(fx);
    vec3 acc = vec3(0.0);
    // Le fil : une courbe douce qui passe par toutes les boules.
    float i0 = clamp(ci, 0.0, n - 1.0);
    float i1 = clamp(ci + 1.0, 0.0, n - 1.0);
    float s = smoothstep(0.0, 1.0, fract(fx));
    float ya = pendulum_y(i0, n, H);
    float yb = pendulum_y(i1, n, H);
    float yl = mix(ya, yb, s);
    float slope = (yb - ya) / cw * 1.5 * (1.0 - abs(s * 2.0 - 1.0));
    float dl = abs(px.y - yl) / sqrt(1.0 + slope * slope);
    vec3 lcol = mix(u_colors[0], u_colors[1], px.x / W);
    acc += lcol * (exp(-pow(dl / (1.3 * u_ppp), 2.0)) * 0.55 + exp(-dl / (H * 0.03)) * 0.10) * (0.7 + 0.5 * u_energy);
    // Les boules, leur tige (jusqu'en haut) et leur halo.
    for (int o = 0; o <= 1; o++) {
        float i = ci + float(o);
        if (i < 0.0 || i >= n) continue;
        float cx = (i + 0.5) * cw;
        float y = pendulum_y(i, n, H);
        float lv = level((i + 0.5) / n);
        vec3 col = mix(u_colors[0], u_colors[1], i / n);
        float rod = exp(-pow((px.x - cx) / (0.7 * u_ppp), 2.0)) * step(px.y, y) * 0.22 * smoothstep(0.0, H * 0.2, px.y);
        float rad = (7.0 + 6.0 * lv + 2.5 * u_kick) * u_ppp;
        float d = length(px - vec2(cx, y));
        float ball = 1.0 - smoothstep(rad - u_ppp, rad + u_ppp, d);
        // Petit reflet en haut à gauche : la boule a du volume.
        float shine = exp(-pow(length(px - vec2(cx - rad * 0.3, y - rad * 0.35)) / (rad * 0.35), 2.0));
        acc += col * rod
             + mix(col, vec3(1.0), 0.25) * ball * (0.75 + 0.5 * lv)
             + vec3(1.0) * ball * shine * 0.5
             + col * exp(-d / (rad * 2.2)) * 0.30 * (0.5 + lv);
    }
    return acc;
}

// ---------- 45. Dunes : crêtes de sable douces sous un soleil bas, ridées par le vent ----------

float dune_ridge(float x, float fl, float H, float W) {
    float lambda = W * (0.70 - 0.09 * fl);
    float th = x / lambda * 6.28318 + fl * 2.3 + u_time * 0.02 * (1.0 + fl);
    float h = 0.5 + 0.5 * sin(th + 0.7 * sin(th * 0.47 + fl));
    h = pow(h, 1.5) + 0.12 * sin(th * 2.7 + fl * 5.0);
    return h;
}
vec3 dunes(vec2 px, float W, float H) {
    float u = px.x / W;
    float relief = detail(0.5, 1.0, 1.8);
    // Ciel : le soleil bas et sa lueur.
    vec2 sun = vec2(W * 0.80, H * 0.34);
    float sd = length(px - sun);
    float sr = H * 0.075;
    vec3 acc = mix(u_colors[1], vec3(1.0), 0.5) * (1.0 - smoothstep(sr - u_ppp, sr + u_ppp, sd)) * (0.8 + 0.3 * u_kick);
    acc += u_colors[0] * exp(-pow(max(sd - sr, 0.0) / (H * 0.22), 2.0)) * (0.22 + 0.15 * u_bass);
    acc += u_colors[2] * exp(-pow((px.y - H * 0.45) / (H * 0.20), 2.0)) * 0.10;
    for (int l = 0; l < 5; l++) {
        float fl = float(l);
        float depth = fl / 4.0;
        float amp = H * (0.045 + 0.030 * fl) * relief * (1.0 + (l >= 3 ? (0.9 * level(u) * envelope(u) + 0.2 * u_kick) : 0.0));
        float ridge = H * (0.50 + 0.105 * fl) - dune_ridge(px.x, fl, H, W) * amp;
        float below = px.y - ridge;
        float inside = smoothstep(-u_ppp, u_ppp, below);
        // Le sable : éclairé près de la crête, de plus en plus sombre en descendant.
        vec3 sand = mix(u_colors[2], u_colors[0], 0.25 + 0.5 * depth);
        float shade = 0.16 + 0.80 * exp(-max(below, 0.0) / (H * (0.06 + 0.07 * depth)));
        // Rides du vent : de fines lignes qui suivent la forme de la dune.
        float ripples_ = 0.85 + 0.15 * sin(below / ((2.2 + 2.0 * depth) * u_ppp) + px.x / (H * 0.05));
        vec3 body = sand * shade * ripples_ * (0.35 + 0.65 * depth);
        // Brume entre les plans : les dunes lointaines se fondent dans le ciel.
        body = mix(body, u_colors[0] * 0.10, (1.0 - depth) * 0.55);
        acc = mix(acc, body, inside);
        // Liseré de lumière sur la crête, côté soleil.
        float toward = 0.5 + 0.5 * clamp((sun.x - px.x) / (W * 0.5), -1.0, 1.0);
        acc += mix(u_colors[1], vec3(1.0), 0.4) * exp(-pow(below / ((1.0 + depth) * u_ppp), 2.0)) * (0.12 + 0.30 * depth) * (0.5 + 0.5 * toward);
    }
    return acc;
}

void main() {
    vec2 p = gl_FragCoord.xy;
    vec2 size = u_rect.zw - u_rect.xy;
    float W = size.x;
    float H = size.y;
    vec2 px = vec2(p.x - u_rect.x, u_rect.w - p.y);  // pixels, y vers le bas
    float u = px.x / W;
    float v = px.y / H;
    // Zoom de l'effet autour de son centre ; les fondus, eux, restent ceux de la page.
    vec2 zp = u_zoom.yz + (px - u_zoom.yz) / max(u_zoom.x, 0.05);
    float zu = zp.x / W;
    float zv = zp.y / H;

    // Rectangle arrondi.
    vec2 c = (u_rect.xy + u_rect.zw) * 0.5;
    vec2 q = abs(p - c) - size * 0.5 + vec2(u_radius);
    float dist = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - u_radius;
    float mask = clamp(0.5 - dist, 0.0, 1.0);

    vec3 acc;
    if (u_mode == 1) acc = caustics(zp, H, zv);
    else if (u_mode == 2) acc = nebula(zp, H);
    else if (u_mode == 3) acc = spectrum_bars(zp, W, H, zu);
    else if (u_mode == 4) acc = ripples(zp, W, H, zu);
    else if (u_mode == 5) acc = lines(zp, W, H, zu);
    else if (u_mode == 6) acc = plasma(zp, H);
    else if (u_mode == 7) acc = beams(zp, W, H);
    else if (u_mode == 8) acc = stars(zp, W, H);
    else if (u_mode == 9) acc = rays(zp, W, H);
    else if (u_mode == 10) acc = liquid(zp, W, H);
    else if (u_mode == 11) acc = bokeh(zp, W, H);
    else if (u_mode == 12) acc = ring(zp, W, H);
    else if (u_mode == 13) acc = scope(zp, W, H, zu);
    else if (u_mode == 14) acc = smoke(zp, H);
    else if (u_mode == 15) acc = fireflies(zp, W, H);
    else if (u_mode == 16) acc = horizon(zp, W, H, zu);
    else if (u_mode == 17) acc = storm(zp, W, H);
    else if (u_mode == 18) acc = tunnel(zp, W, H);
    else if (u_mode == 19) acc = spiral(zp, W, H);
    else if (u_mode == 20) acc = light_rain(zp, W, H);
    else if (u_mode == 21) acc = kaleidoscope(zp, W, H);
    else if (u_mode == 22) acc = rosette(zp, W, H);
    else if (u_mode == 23) acc = flames(zp, W, H, zu);
    else if (u_mode == 24) acc = galaxy(zp, W, H);
    else if (u_mode == 25) acc = crystals(zp, W, H);
    else if (u_mode == 26) acc = lava(zp, W, H);
    else if (u_mode == 27) acc = leds(zp, W, H, zu);
    else if (u_mode == 28) acc = vinyl(zp, W, H);
    else if (u_mode == 29) acc = ferrofluid(zp, W, H);
    else if (u_mode == 30) acc = hologram(zp, W, H);
    else if (u_mode == 31) acc = silk(zp, H);
    else if (u_mode == 32) acc = iridescence(zp, H);
    else if (u_mode == 33) acc = topography(zp, W, H);
    else if (u_mode == 34) acc = glass(zp, W, H, zu);
    else if (u_mode == 35) acc = flux(zp, H);
    else if (u_mode == 36) acc = halftone(zp, W, H);
    else if (u_mode == 37) acc = lissajous(zp, W, H);
    else if (u_mode == 38) acc = retro(zp, W, H);
    else if (u_mode == 39) acc = mosaic(zp, W, H);
    else if (u_mode == 40) acc = stained(zp, W, H);
    else if (u_mode == 41) acc = radar(zp, W, H);
    else if (u_mode == 42) acc = helix(zp, W, H);
    else if (u_mode == 43) acc = pendulums(zp, W, H);
    else if (u_mode == 44) acc = constellation(zp, W, H);
    else if (u_mode == 45) acc = dunes(zp, W, H);
    else if (u_mode == 46) acc = bubbles(zp, W, H);
    else if (u_mode == 47) acc = eclipse(zp, W, H);
    else if (u_mode == 48) acc = moire(zp, W, H);
    else if (u_mode == 49) acc = neon(zp, W, H);
    else acc = aurora(zp, W, H, zu);

    // Fondus doux : en haut, en bas, et côté texte pour le bandeau.
    float fade = smoothstep(0.0, u_fade.x, v) * (1.0 - smoothstep(1.0 - u_fade.y, 1.0, v));
    fade *= mix(1.0, smoothstep(0.30, 0.95, u), u_left);
    acc *= fade * u_intensity;

    // Saturation douce : les zones intenses éclatent sans brûler ; léger tramage contre les bandes.
    vec3 color = 1.0 - exp(-acc * 1.6);
    color += (fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453) - 0.5) / 255.0;
    color = max(color, 0.0);
    float alpha = max(max(color.r, color.g), color.b);
    out_color = vec4(color, alpha) * mask;  // alpha prémultiplié, comme egui
}
"#;

unsafe fn create(gl: &glow::Context) -> Option<Resources> {
    let version = egui_glow::ShaderVersion::get(gl);
    if !version.is_new_shader_interface() {
        return None;
    }
    let header = version.version_declaration();
    let precision = if version.is_embedded() {
        "precision highp float;\nprecision highp int;\n"
    } else {
        ""
    };
    let vertex = format!(
        "{header}{precision}
        void main() {{
            vec2 p = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1)) * 2.0 - 1.0;
            gl_Position = vec4(p, 0.0, 1.0);
        }}"
    );
    let fragment = format!("{header}{precision}{FRAGMENT}");
    unsafe {
        let program = gl.create_program().ok()?;
        let mut shaders = Vec::new();
        for (kind, source) in [(glow::VERTEX_SHADER, &vertex), (glow::FRAGMENT_SHADER, &fragment)] {
            let shader = gl.create_shader(kind).ok()?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                eprintln!(
                    "SterMusic, shader des vagues : {}",
                    gl.get_shader_info_log(shader)
                );
                return None;
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            eprintln!(
                "SterMusic, shader des vagues : {}",
                gl.get_program_info_log(program)
            );
            return None;
        }
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        let loc = UNIFORMS
            .iter()
            .map(|n| gl.get_uniform_location(program, n))
            .collect();
        // Même règle qu'egui_glow pour savoir si ses textures sont en sRGB.
        let srgb_textures = version == egui_glow::ShaderVersion::Es300
            || gl.supported_extensions().iter().any(|e| e.contains("sRGB"));
        Some(Resources {
            program,
            vao: gl.create_vertex_array().ok()?,
            loc,
            srgb_textures,
        })
    }
}

unsafe fn draw(
    gl: &glow::Context,
    res: &Resources,
    info: &egui::PaintCallbackInfo,
    w: &Waves,
    texture: Option<glow::Texture>,
) {
    let vp = info.viewport_in_pixels();
    let ppp = info.pixels_per_point;
    let l = |name: &str| res.loc[UNIFORMS.iter().position(|n| *n == name).unwrap()].as_ref();
    unsafe {
        gl.use_program(Some(res.program));
        gl.bind_vertex_array(Some(res.vao));
        let (x, y) = (vp.left_px as f32, vp.from_bottom_px as f32);
        gl.uniform_4_f32(l("u_rect"), x, y, x + vp.width_px as f32, y + vp.height_px as f32);
        gl.uniform_1_f32(l("u_radius"), w.radius * ppp);
        gl.uniform_1_f32(l("u_time"), w.time);
        gl.uniform_1_f32_slice(l("u_levels"), &w.levels);
        let colors: Vec<f32> = w.colors.iter().flatten().copied().collect();
        gl.uniform_3_f32_slice(l("u_colors"), &colors);
        gl.uniform_1_f32(l("u_kick"), w.kick);
        gl.uniform_1_f32(l("u_bass"), w.bass);
        gl.uniform_1_f32(l("u_intensity"), w.intensity);
        gl.uniform_4_f32(l("u_shape"), w.base, w.amp, 0.0, 0.0);
        gl.uniform_2_f32(l("u_fade"), w.fade_top, w.fade_bottom);
        gl.uniform_1_f32(l("u_left"), w.left);
        gl.uniform_1_f32(l("u_ppp"), ppp);
        gl.uniform_1_i32(l("u_mode"), w.mode);
        gl.uniform_1_f32(l("u_energy"), w.energy);
        gl.uniform_1_f32(l("u_scale"), w.scale.max(0.1));
        gl.uniform_1_f32_slice(l("u_wave"), &w.wave);
        gl.uniform_3_f32(l("u_focus"), w.focus[0] * ppp, w.focus[1] * ppp, w.focus[2] * ppp);
        gl.uniform_1_f32(l("u_detail"), w.detail.clamp(0.0, 1.0));
        gl.uniform_3_f32(l("u_zoom"), w.zoom.max(0.05), w.zoom_center[0] * ppp, w.zoom_center[1] * ppp);
        gl.uniform_1_i32(l("u_has_tex"), texture.is_some() as i32);
        gl.uniform_1_i32(l("u_srgb"), res.srgb_textures as i32);
        if let Some(tex) = texture {
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.uniform_1_i32(l("u_tex"), 0);
        }
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        gl.bind_vertex_array(None);
        if texture.is_some() {
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        gl.use_program(None);
    }
}
