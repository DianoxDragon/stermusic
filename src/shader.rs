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
}

/// Nombre de points de la forme d'onde envoyés au shader.
pub const WAVE: usize = 128;

/// Les visualiseurs : nom, et vrai s'il remplit toute la page (sinon, une bande en bas).
/// Choix « Automatique » : un visualiseur au hasard à chaque morceau (et toutes les 45 s).
pub const AUTO: usize = STYLES.len();

pub const STYLES: [(&str, bool); 28] = [
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
];

struct Resources {
    program: glow::Program,
    vao: glow::VertexArray,
    loc: Vec<Option<glow::UniformLocation>>,
    /// Textures d'egui stockées en sRGB (leur lecture renvoie alors des couleurs linéaires).
    srgb_textures: bool,
}

const UNIFORMS: [&str; 20] = [
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

// ---------- 0. Aurore : rubans de lumière ----------

float ribbon_y(float u, float H, float freq, float speed, float phase, float k) {
    float amp = H * u_shape.y * (0.16 + 0.22 * level(u) + 0.08 * u_kick) * envelope(u);
    return H * u_shape.x + wave(u, freq, speed, phase) * amp - k * 3.0 * u_ppp;
}
vec3 aurora(vec2 px, float W, float H, float u) {
    float env = envelope(u);
    float lv = level(u);
    float base = H * u_shape.x;
    vec3 acc = vec3(0.0);
    vec2 g = vec2((px.x - 0.5 * W) / (W * (0.30 + 0.06 * u_kick)),
                  (px.y - base - 0.08 * H) / (H * (0.30 + 0.14 * u_kick)));
    acc += u_colors[2] * (0.10 + 0.20 * u_bass + 0.50 * u_kick) * exp(-dot(g, g));
    float top = base - lv * env * H * 0.42;
    float column = smoothstep(top - 0.12 * H, base, px.y) * (1.0 - smoothstep(base, base + 0.10 * H, px.y));
    acc += u_colors[0] * 0.10 * column * lv;
    for (int k = 0; k < 3; k++) {
        float freq = k == 0 ? 1.3 : (k == 1 ? 1.9 : 0.9);
        float speed = k == 0 ? 0.55 : (k == 1 ? 0.8 : 0.4);
        float phase = k == 0 ? 0.0 : (k == 1 ? 2.1 : 4.0);
        float weight = k == 0 ? 1.0 : (k == 1 ? 0.75 : 0.6);
        vec3 col = u_colors[k];
        float y0 = ribbon_y(u, H, freq, speed, phase, float(k));
        float y1 = ribbon_y(u + 1.0 / W, H, freq, speed, phase, float(k));
        float slope = y1 - y0;
        float d = abs(px.y - y0) / sqrt(1.0 + slope * slope);
        float sigma = H * (0.025 + 0.09 * lv + 0.04 * u_kick) * max(env, 0.15);
        float a = (0.35 + 0.8 * lv + 0.9 * u_kick) * env * weight;
        acc += col * a * 0.6 * exp(-(d * d) / (sigma * sigma));
        float core = (1.0 + 0.7 * lv) * u_ppp;
        acc += mix(col, vec3(1.0), 0.45) * a * 0.7 * exp(-(d * d) / (core * core));
    }
    return acc;
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
    vec2 p = px / (H * 0.42 * u_scale);
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
    float n = fbm(p + vec2(t, -t * 0.6) + 1.5 * fbm(p * 0.7 - vec2(t * 0.8, t * 0.3)));
    vec3 col = mix(u_colors[2], u_colors[0], smoothstep(0.30, 0.75, n));
    col = mix(col, u_colors[1], smoothstep(0.62, 0.92, n));
    return col * (0.22 + 1.2 * n * n) * (0.75 + 0.45 * u_bass + 0.6 * u_kick);
}

// ---------- 3. Spectre : colonnes de lumière et leur reflet ----------

vec3 spectrum_bars(vec2 px, float W, float H, float u) {
    float n = 56.0;
    float cell = floor(u * n);
    float cu = (cell + 0.5) / n;
    float lv = level(cu) * envelope(cu);
    float bx = abs(fract(u * n) - 0.5) * 2.0;
    float base = H * u_shape.x;
    float h = lv * H * 0.55 * u_shape.y + 2.0 * u_ppp;
    float dy = base - px.y;  // > 0 au-dessus de la ligne de base
    float body = 1.0 - smoothstep(0.45, 0.75, bx);
    float up = smoothstep(h + 0.02 * H, h - 0.01 * H, dy) * step(0.0, dy);
    float down = smoothstep(h * 0.45, 0.0, -dy) * step(dy, 0.0) * 0.30;
    vec3 col = mix(u_colors[0], mix(u_colors[1], vec3(1.0), 0.3), clamp(dy / max(h, 1.0), 0.0, 1.0));
    float glow = exp(-pow(dy / (h + 0.05 * H) - 0.4, 2.0) * 3.0) * lv * 0.25;
    return col * (body * (up + down) * (0.55 + 0.6 * u_kick) + glow);
}

// ---------- 4. Ondes : anneaux qui partent du bas sur le rythme ----------

vec3 ripples(vec2 px, float W, float H, float u) {
    vec2 c = vec2(0.5 * W, H * 1.02);
    float r = length(px - c) / H;
    vec3 acc = vec3(0.0);
    for (int k = 0; k < 6; k++) {
        float rk = fract(u_time * 0.10 + float(k) / 6.0) * 1.5;
        float width = 0.010 + 0.025 * level(u) + 0.015 * u_kick;
        float ring = exp(-pow((r - rk) / width, 2.0)) * (1.0 - rk / 1.5);
        acc += mix(u_colors[k - (k / 3) * 3], vec3(1.0), 0.2) * ring * (0.35 + 0.7 * u_energy);
    }
    acc += u_colors[2] * exp(-r * r * 6.0) * (0.15 + 0.6 * u_kick);
    return acc;
}

// ---------- 5. Lignes : relief de lignes empilées ----------

vec3 lines(vec2 px, float W, float H, float u) {
    vec3 acc = vec3(0.0);
    for (int i = 0; i < 16; i++) {
        float fi = float(i) / 15.0;
        float y_base = H * (0.30 + 0.62 * fi);
        float bump = level(u) * envelope(u) * (0.6 + 0.8 * noise(vec2(u * 9.0 + float(i) * 3.1, u_time * 0.4 + float(i))));
        float y = y_base - H * 0.16 * u_shape.y * bump;
        float d = abs(px.y - y);
        float core = 0.9 * u_ppp;
        float depth = 0.35 + 0.65 * fi;
        vec3 col = mix(u_colors[2], mix(u_colors[0], u_colors[1], bump), fi);
        acc += col * depth * (exp(-(d * d) / (core * core)) * 0.9 + exp(-(d * d) / (36.0 * u_ppp * u_ppp)) * 0.12) * (0.6 + 0.6 * u_kick);
    }
    return acc;
}

// ---------- 6. Plasma : couleurs douces qui se mélangent ----------

vec3 plasma(vec2 px, float H) {
    vec2 p = px / (H * u_scale);
    float t = u_time * 0.35;
    float v = sin(p.x * 2.6 + t) + sin(p.y * 3.1 - t * 1.2) + sin((p.x + p.y) * 2.0 + t * 0.7)
            + sin(length(p - vec2(1.0, 0.6)) * 4.5 - t);
    v = v * 0.25 + 0.5;
    vec3 col = mix(u_colors[2], u_colors[0], smoothstep(0.2, 0.6, v));
    col = mix(col, u_colors[1], smoothstep(0.6, 0.95, v));
    return col * (0.38 + 0.30 * u_bass + 0.45 * u_kick);
}

// ---------- 7. Faisceaux : lumières de scène qui balaient ----------

vec3 beams(vec2 px, float W, float H) {
    vec3 acc = vec3(0.0);
    for (int k = 0; k < 6; k++) {
        float fk = float(k);
        vec2 o = vec2(W * (0.08 + 0.168 * fk), -0.05 * H);
        float a = sin(u_time * (0.25 + 0.05 * fk) + fk * 1.7) * 0.45;
        vec2 dir = vec2(sin(a), cos(a));
        vec2 rel = px - o;
        float along = dot(rel, dir);
        float across = abs(rel.x * dir.y - rel.y * dir.x);
        float width = 0.04 * H + along * 0.10;
        float lvk = u_levels[k * 2];
        float beam = exp(-pow(across / width, 2.0)) * smoothstep(0.0, 0.2 * H, along) * exp(-along / (1.4 * H));
        acc += u_colors[k - (k / 3) * 3] * beam * (0.18 + 0.7 * lvk + 0.4 * u_kick);
    }
    return acc;
}

// ---------- 8. Étoiles : un champ d'étoiles qui file vers soi ----------

vec3 stars(vec2 px, float W, float H) {
    vec2 c = vec2(0.5 * W, 0.55 * H);
    vec2 p = (px - c) / H;
    vec3 acc = vec3(0.0);
    for (int i = 0; i < 3; i++) {
        float z = fract(u_time * 0.06 + float(i) / 3.0);
        float zoom = mix(10.0, 1.2, z);
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

// ---------- 9. Rayons : soleil levant dont les rayons suivent le spectre ----------

vec3 rays(vec2 px, float W, float H) {
    vec2 c = vec2(0.5 * W, H * 1.05);
    vec2 d = px - c;
    float r = length(d) / H;
    float ang = atan(d.x, -d.y);  // 0 vers le haut
    float u = clamp(0.5 + ang / 3.14159, 0.0, 1.0);
    float n = 40.0;
    float ray = 1.0 - smoothstep(0.15, 0.5, abs(fract(u * n) - 0.5) * 2.0);
    float lv = level(floor(u * n) / n + 0.5 / n);
    float reach = 0.35 + 0.9 * lv;
    float body = ray * smoothstep(reach, reach * 0.3, r) * smoothstep(0.08, 0.2, r);
    vec3 col = mix(u_colors[0], u_colors[1], r / max(reach, 0.01));
    vec3 sun = mix(u_colors[1], vec3(1.0), 0.4) * exp(-r * r * 30.0) * (0.6 + 0.8 * u_kick);
    return col * body * (0.35 + 0.5 * u_kick) + sun + u_colors[2] * exp(-r * r * 4.0) * 0.12;
}

// ---------- 10. Pochette liquide : la pochette floutée qui coule lentement ----------

vec3 srgb_from_linear(vec3 c) {
    return mix(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, vec3(lessThan(c, vec3(0.0031308))));
}
vec3 liquid(vec2 px, float W, float H) {
    float t = u_time * 0.07;
    vec2 q = (px - vec2(W, H) * 0.5) / H;
    float a = t * 0.5;
    q = mat2(cos(a), -sin(a), sin(a), cos(a)) * q / (1.4 * u_scale);
    vec2 warp = vec2(fbm(q * 2.0 + vec2(t, -t)), fbm(q * 2.0 + vec2(-t, t) + 5.3)) - 0.5;
    q += warp * (0.55 + 0.30 * u_bass + 0.25 * u_kick);
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
    for (int i = 0; i < 22; i++) {
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
    float n = 120.0;
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

// ---------- 13. Oscilloscope : la forme d'onde, en fil de lumière ----------

float wave_at(float u) {
    float x = clamp(u, 0.0, 1.0) * 127.0;
    int i = int(floor(x));
    return mix(u_wave[i], u_wave[min(i + 1, 127)], fract(x));
}
vec3 scope(vec2 px, float W, float H, float u) {
    float base = H * u_shape.x;
    float amp = H * 0.24 * u_shape.y;
    float y0 = base - wave_at(u) * amp * envelope(u);
    float y1 = base - wave_at(u + 1.0 / W) * amp * envelope(u + 1.0 / W);
    float slope = y1 - y0;
    float d = abs(px.y - y0) / sqrt(1.0 + slope * slope);
    float a = (0.55 + 0.8 * u_energy + 0.6 * u_kick) * envelope(u);
    float glow = H * (0.02 + 0.03 * u_energy);
    float core = 1.1 * u_ppp;
    vec3 col = u_colors[0];
    // Reflet très doux sous la ligne.
    float yr = base + (base - y0) * 0.35 + H * 0.05;
    float dr = abs(px.y - yr);
    return col * a * 0.5 * exp(-d * d / (glow * glow))
         + mix(col, vec3(1.0), 0.5) * a * exp(-d * d / (core * core))
         + u_colors[2] * a * 0.15 * exp(-dr * dr / (glow * glow * 4.0));
}

// ---------- 14. Fumée : volutes d'encre colorée ----------

vec3 smoke(vec2 px, float H) {
    vec2 p = px / (H * u_scale) * 1.6;
    float t = u_time * 0.10;
    vec2 q = vec2(fbm(p + vec2(0.0, t)), fbm(p + vec2(5.2, -t)));
    vec2 r = vec2(fbm(p + 3.0 * q + vec2(1.7, 9.2) + t * 0.7), fbm(p + 3.0 * q + vec2(8.3, 2.8) - t * 0.5));
    float f = fbm(p + 2.5 * r);
    vec3 col = mix(u_colors[2], u_colors[0], clamp(f * f * 2.0, 0.0, 1.0));
    col = mix(col, u_colors[1], clamp(length(q) * 0.8, 0.0, 1.0) * 0.6);
    return col * pow(f, 2.2) * 0.95 * (0.7 + 0.5 * u_bass + 0.9 * u_kick);
}

// ---------- 15. Lucioles : petites lumières et leur traînée ----------

vec3 fireflies(vec2 px, float W, float H) {
    vec3 acc = vec3(0.0);
    vec3 warm = mix(u_colors[1], vec3(1.0, 0.95, 0.75), 0.4);
    for (int i = 0; i < 26; i++) {
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

// ---------- 16. Horizon : paysage de lignes dont le relief suit le spectre ----------

vec3 horizon(vec2 px, float W, float H, float u) {
    float hy = H * 0.50;
    vec3 acc = vec3(0.0);
    vec2 sc = vec2(0.5 * W, hy);
    float sd = length((px - sc) / H);
    acc += mix(u_colors[0], u_colors[1], 0.5) * exp(-sd * sd * 16.0) * (0.45 + 0.6 * u_bass + 0.6 * u_kick);
    if (px.y > hy - 2.0) {
        float scroll = fract(u_time * 0.5);
        for (int i = 0; i < 16; i++) {
            float z = float(i) + 1.0 - scroll;
            float y = hy + (H - hy) / z;
            y -= level(u) * envelope(u) * H * 0.22 / sqrt(z) * (1.0 + 0.4 * u_kick);
            float d = abs(px.y - y);
            float core = (0.8 + 1.2 / z) * u_ppp;
            float fade = smoothstep(16.0, 4.0, z) * smoothstep(0.0, 1.0, z);
            vec3 col = mix(u_colors[0], u_colors[2], clamp(z / 16.0, 0.0, 1.0));
            acc += col * fade * (exp(-d * d / (core * core)) + 0.15 * exp(-d / (6.0 * u_ppp)));
        }
        // Lignes qui filent vers le point de fuite.
        float k = (px.x - 0.5 * W) / max(px.y - hy, 1.0);
        float g = abs(fract(k * 0.8 + 0.5) - 0.5) / 0.8 * (px.y - hy);
        float vfade = smoothstep(hy, H, px.y);
        acc += u_colors[2] * exp(-g * g / (u_ppp * u_ppp)) * vfade * 0.5;
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
    col += vec3(0.95, 0.95, 1.0) * bolt * smoothstep(0.35, 0.8, u_kick);
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
    float ds = abs(fract(u * 24.0) - 0.5) / 24.0 * 6.28318 * r * H;
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
    float band = 0.5 + 0.5 * cos(a * 5.0 + log(r + 1e-3) * 9.0 - u_time * 1.2);
    float line = pow(band, 8.0 - 4.0 * lv);
    vec3 col = mix(u_colors[0], u_colors[1], 0.5 + 0.5 * sin(a * 2.0 + u_time * 0.3));
    float fade = smoothstep(0.0, 0.08, r) * exp(-r * 1.6);
    float core = exp(-r * r * 40.0) * (0.3 + 1.0 * u_kick);
    return col * line * fade * (0.8 + 0.8 * u_energy + 0.6 * u_kick)
         + mix(u_colors[2], vec3(1.0), 0.3) * core;
}

// ---------- 20. Pluie de lumière : traits qui tombent, plus longs sur leurs fréquences ----------

vec3 light_rain(vec2 px, float W, float H) {
    vec3 acc = vec3(0.0);
    float cols = max(floor(W / (14.0 * u_ppp)), 8.0);
    for (int layer = 0; layer < 2; layer++) {
        float fl = float(layer);
        float n = cols * (layer == 0 ? 1.0 : 0.6);
        float x = px.x / W * n;
        float ci = floor(x);
        float seed = hash(vec2(ci, fl * 13.0));
        float lv = level((ci + 0.5) / n);
        float speed = (0.20 + 0.45 * seed) * (1.0 + 0.5 * fl);
        float head = fract(u_time * speed + seed * 7.0) * 1.3 - 0.15;
        float d = head - px.y / H;  // > 0 au-dessus de la tête de la goutte
        float len = 0.10 + 0.35 * lv + 0.10 * seed;
        float trail = step(0.0, d) * exp(-d / len);
        float dx = abs(fract(x) - 0.5) * W / n;
        float width = (0.9 + 0.6 * fl) * u_ppp;
        float line = exp(-dx * dx / (width * width));
        float tip = exp(-pow(d * H / (4.0 * u_ppp), 2.0));
        vec3 col = mix(u_colors[layer], vec3(1.0), 0.15);
        acc += col * line * (trail * (0.35 + 0.9 * lv) + tip * 1.2) * (layer == 0 ? 1.0 : 0.5)
             * (0.7 + 0.5 * u_kick);
    }
    return acc;
}

// ---------- 21. Kaléidoscope : motifs en miroir autour du centre ----------

vec3 kaleidoscope(vec2 px, float W, float H) {
    vec2 p = (px - vec2(0.5 * W, 0.5 * H)) / H;
    float r = length(p);
    float seg = 6.28318 / 8.0;
    float a = mod(atan(p.y, p.x) + u_time * 0.05, seg);
    a = abs(a - seg * 0.5);
    vec2 q = vec2(cos(a), sin(a)) * r / u_scale;
    float t = u_time * 0.15;
    float n = fbm(q * 4.0 + vec2(t, -t * 0.7) + 1.5 * fbm(q * 3.0 - t));
    float lines = exp(-pow((fract(n * 6.0) - 0.5) / 0.08, 2.0));
    vec3 col = mix(u_colors[2], u_colors[0], smoothstep(0.3, 0.7, n));
    col = mix(col, u_colors[1], smoothstep(0.6, 0.9, n));
    float pulse = 0.6 + 0.4 * u_bass + 0.7 * u_kick;
    return (col * (0.25 + 0.9 * n * n) + mix(col, vec3(1.0), 0.4) * lines * 0.35 * (0.5 + level(fract(r * 2.0))))
         * pulse * smoothstep(1.2, 0.2, r);
}

// ---------- 22. Rosace : le spectre en couronne, graves en haut ----------

vec3 rosette(vec2 px, float W, float H) {
    vec2 d = px - vec2(0.5 * W, 0.52 * H);
    float r = length(d) / H;
    float u = fract(atan(d.x, -d.y) / 6.28318 + 1.0);  // 0 en haut, sens horaire
    float n = 64.0;
    float cu = (floor(u * n) + 0.5) / n;
    float lv = level(fract(cu + 0.5));  // graves en haut, aigus en bas
    float r0 = 0.16 + 0.03 * u_bass + 0.03 * u_kick;
    float len = 0.02 + lv * 0.30;
    float cell = 6.28318 * max(r, 0.01) * H / n;  // largeur d'une case, en pixels
    float ang = (fract(u * n) - 0.5) * cell;
    float bar = exp(-pow(ang / (0.32 * cell), 4.0));
    float along = r - r0;
    float body = smoothstep(0.0, 0.004, along) * smoothstep(len, len - 0.02, along);
    vec3 col = mix(u_colors[0], mix(u_colors[1], vec3(1.0), 0.3), clamp(along / max(len, 0.01), 0.0, 1.0));
    float disc = smoothstep(r0, r0 - 0.005, r) * (0.10 + 0.25 * u_kick);
    float rim = exp(-pow((r - r0) * H / (1.5 * u_ppp), 2.0)) * 0.6;
    return col * bar * body * (0.6 + 0.6 * u_kick) + u_colors[2] * disc + mix(u_colors[1], vec3(1.0), 0.4) * rim;
}

// ---------- 23. Flammes : un feu qui monte plus haut sur les graves ----------

vec3 flames(vec2 px, float W, float H, float u) {
    vec2 p = vec2(px.x / H, (H - px.y) / H);  // y vers le haut, depuis le bas
    float t = u_time * 0.9;
    float lv = level(u) * envelope(u);
    float n = fbm(vec2(p.x * 3.0, p.y * 2.0 - t) + fbm(vec2(p.x * 1.5 + 3.0, p.y - t * 0.6)));
    float height = 0.22 + 0.45 * lv + 0.15 * u_bass + 0.15 * u_kick;
    float f = clamp(1.0 - p.y / height + (n - 0.5) * 1.1, 0.0, 1.0);
    f *= f;
    vec3 col = mix(u_colors[2] * 0.4, u_colors[0], smoothstep(0.0, 0.5, f));
    col = mix(col, mix(u_colors[1], vec3(1.0, 0.95, 0.85), 0.5), smoothstep(0.5, 1.0, f));
    return col * f * 1.3 * (0.7 + 0.5 * u_energy);
}

// ---------- 24. Galaxie : deux bras d'étoiles qui tournent lentement ----------

vec3 galaxy(vec2 px, float W, float H) {
    vec2 p = (px - vec2(0.5 * W, 0.5 * H)) / H;
    p.y *= 1.8;  // vue inclinée
    float r = length(p);
    float a = atan(p.y, p.x);
    float t = u_time * 0.08;
    float arm = sin(2.0 * a - log(r + 0.02) * 4.0 + t * 3.0);
    float dens = pow(0.5 + 0.5 * arm, 3.0) * exp(-r * 3.5);
    float dust = fbm(vec2(a * 2.0 + t, r * 8.0 - t));
    vec3 col = mix(u_colors[0], u_colors[1], clamp(r * 2.0, 0.0, 1.0));
    vec3 acc = col * dens * (0.6 + 0.8 * dust) * (0.9 + 0.6 * u_bass + 0.5 * u_kick);
    acc += mix(u_colors[2], vec3(1.0, 0.95, 0.9), 0.5) * exp(-r * r * 60.0) * (0.6 + 0.8 * u_kick);
    float rot = t * 2.0;
    vec2 q = mat2(cos(rot), -sin(rot), sin(rot), cos(rot)) * p * 18.0;
    vec2 g = floor(q);
    vec2 f = fract(q) - 0.5;
    float h = hash(g);
    float star = step(0.88, h) * exp(-dot(f, f) * 90.0) * (0.5 + 0.5 * sin(u_time * 2.0 + h * 40.0));
    acc += vec3(1.0) * star * 0.5 * exp(-r * 1.5) * (0.6 + u_energy);
    return acc;
}

// ---------- 25. Cristaux : facettes qui s'allument sur leurs fréquences ----------

vec3 crystals(vec2 px, float W, float H) {
    float size = H * 0.18 * u_scale;
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
    for (int i = 0; i < 9; i++) {
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

// ---------- 27. Diodes : égaliseur à points lumineux, comme une vieille chaîne hi-fi ----------

vec3 leds(vec2 px, float W, float H, float u) {
    float cell = 9.0 * u_ppp;
    float base = H * u_shape.x;
    // La grille part de la ligne de base (vers le haut) et est centrée en largeur : chaque point
    // tombe entier dans sa case, sans demi-points en haut, en bas ni sur les côtés.
    float cols = max(floor(W / cell), 1.0);
    float gx = (px.x - 0.5 * (W - cols * cell)) / cell;
    float gy = (base - px.y) / cell;  // 0 → 1 : la première rangée, juste au-dessus de la base
    float col_i = floor(gx);
    float row = floor(gy);
    vec2 f = vec2(fract(gx), fract(gy)) - 0.5;
    float cu = (col_i + 0.5) / cols;
    float lv = level(cu) * envelope(cu);
    float most = max(floor(H * 0.55 / cell), 1.0);
    float lit = floor(lv * most * u_shape.y + 0.5 * u_kick);
    float inside = step(0.0, row) * step(row, most - 1.0) * step(0.0, col_i) * step(col_i, cols - 1.0);
    float on = inside * step(row, lit - 1.0);
    float dot_ = smoothstep(0.42, 0.28, length(f));
    float height = row / most;
    vec3 col = mix(u_colors[0], u_colors[1], clamp(height * 1.6, 0.0, 1.0));
    col = mix(col, vec3(1.0, 0.95, 0.9), smoothstep(0.75, 1.0, height) * 0.5);
    return col * dot_ * (on * (0.9 + 0.5 * u_kick) + inside * 0.05);
}

void main() {
    vec2 p = gl_FragCoord.xy;
    vec2 size = u_rect.zw - u_rect.xy;
    float W = size.x;
    float H = size.y;
    vec2 px = vec2(p.x - u_rect.x, u_rect.w - p.y);  // pixels, y vers le bas
    float u = px.x / W;
    float v = px.y / H;

    // Rectangle arrondi.
    vec2 c = (u_rect.xy + u_rect.zw) * 0.5;
    vec2 q = abs(p - c) - size * 0.5 + vec2(u_radius);
    float dist = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - u_radius;
    float mask = clamp(0.5 - dist, 0.0, 1.0);

    vec3 acc;
    if (u_mode == 1) acc = caustics(px, H, v);
    else if (u_mode == 2) acc = nebula(px, H);
    else if (u_mode == 3) acc = spectrum_bars(px, W, H, u);
    else if (u_mode == 4) acc = ripples(px, W, H, u);
    else if (u_mode == 5) acc = lines(px, W, H, u);
    else if (u_mode == 6) acc = plasma(px, H);
    else if (u_mode == 7) acc = beams(px, W, H);
    else if (u_mode == 8) acc = stars(px, W, H);
    else if (u_mode == 9) acc = rays(px, W, H);
    else if (u_mode == 10) acc = liquid(px, W, H);
    else if (u_mode == 11) acc = bokeh(px, W, H);
    else if (u_mode == 12) acc = ring(px, W, H);
    else if (u_mode == 13) acc = scope(px, W, H, u);
    else if (u_mode == 14) acc = smoke(px, H);
    else if (u_mode == 15) acc = fireflies(px, W, H);
    else if (u_mode == 16) acc = horizon(px, W, H, u);
    else if (u_mode == 17) acc = storm(px, W, H);
    else if (u_mode == 18) acc = tunnel(px, W, H);
    else if (u_mode == 19) acc = spiral(px, W, H);
    else if (u_mode == 20) acc = light_rain(px, W, H);
    else if (u_mode == 21) acc = kaleidoscope(px, W, H);
    else if (u_mode == 22) acc = rosette(px, W, H);
    else if (u_mode == 23) acc = flames(px, W, H, u);
    else if (u_mode == 24) acc = galaxy(px, W, H);
    else if (u_mode == 25) acc = crystals(px, W, H);
    else if (u_mode == 26) acc = lava(px, W, H);
    else if (u_mode == 27) acc = leds(px, W, H, u);
    else acc = aurora(px, W, H, u);

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
