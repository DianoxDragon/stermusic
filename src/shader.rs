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

pub const STYLES: [(&str, bool); 18] = [
    ("Aurore", false),
    ("Caustiques", true),
    ("Nébuleuse", true),
    ("Spectre", false),
    ("Ondes", true),
    ("Lignes", false),
    ("Plasma", true),
    ("Faisceaux", true),
    ("Étoiles", true),
    ("Rayons", true),
    ("Pochette liquide", true),
    ("Bokeh", true),
    ("Anneau", true),
    ("Oscilloscope", false),
    ("Fumée", true),
    ("Lucioles", true),
    ("Horizon", false),
    ("Orage", true),
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
