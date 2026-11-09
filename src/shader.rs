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
}

struct Resources {
    program: glow::Program,
    vao: glow::VertexArray,
    loc: Vec<Option<glow::UniformLocation>>,
}

const UNIFORMS: [&str; 13] = [
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
    "u_unused",
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
            unsafe { draw(painter.gl(), res, &info, &waves) };
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
uniform float u_intensity;
uniform vec4 u_shape;       // base, amplitude, —, —
uniform vec2 u_fade;        // fondu haut, fondu bas
uniform float u_left;
uniform float u_ppp;
out vec4 out_color;

float level(float u) {
    float d = clamp(abs(u * 2.0 - 1.0), 0.0, 1.0) * 15.0;
    int i = int(floor(d));
    return mix(u_levels[i], u_levels[min(i + 1, 15)], fract(d));
}

float wave(float u, float freq, float speed, float phase) {
    return sin(u * freq * 6.28318 + u_time * speed + phase)
         + 0.35 * sin(u * freq * 2.3 * 6.28318 - u_time * speed * 1.4 + phase);
}

float envelope(float u) {
    return pow(sin(3.14159 * clamp(u, 0.0, 1.0)), 1.2);
}

// Centre d'un ruban (en pixels depuis le haut) à la position u.
float ribbon_y(float u, float H, float freq, float speed, float phase, float k) {
    float lv = level(u);
    float amp = H * u_shape.y * (0.16 + 0.22 * lv + 0.08 * u_kick) * envelope(u);
    return H * u_shape.x + wave(u, freq, speed, phase) * amp - k * 3.0 * u_ppp;
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

    float env = envelope(u);
    float lv = level(u);
    float base = H * u_shape.x;
    vec3 acc = vec3(0.0);

    // Grande lueur de fond qui gonfle sur les coups de grosse caisse.
    vec2 g = vec2((px.x - 0.5 * W) / (W * (0.30 + 0.06 * u_kick)),
                  (px.y - base - 0.08 * H) / (H * (0.30 + 0.14 * u_kick)));
    acc += u_colors[2] * (0.10 + 0.20 * u_bass + 0.50 * u_kick) * exp(-dot(g, g));

    // Colonnes de lumière très douces (le spectre), sous les rubans.
    float top = base - lv * env * H * 0.42;
    float column = smoothstep(top - 0.12 * H, base, px.y) * (1.0 - smoothstep(base, base + 0.10 * H, px.y));
    acc += u_colors[0] * 0.10 * column * lv;

    // Trois rubans : lueur large + fil lumineux net, à épaisseur constante même dans les pentes.
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
        Some(Resources {
            program,
            vao: gl.create_vertex_array().ok()?,
            loc,
        })
    }
}

unsafe fn draw(gl: &glow::Context, res: &Resources, info: &egui::PaintCallbackInfo, w: &Waves) {
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
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        gl.bind_vertex_array(None);
        gl.use_program(None);
    }
}
