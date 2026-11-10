//! Globe terrestre dessiné par la carte graphique : l'image satellite (Blue Marble avec relief,
//! NASA GIBS, domaine public) projetée sur une sphère vue de face, avec un liseré d'atmosphère.
//! En zoomant, une image plus détaillée de la région affichée est téléchargée et fondue
//! par-dessus. Les stations sont placées avec [`View::project`].

use std::f32::consts::{FRAC_PI_2, PI};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, pos2, Pos2, Rect};
use eframe::egui_glow;
use eframe::glow::{self, HasContext};

/// Image équirectangulaire de la Terre (4096 × 2048, longitude −180° à gauche).
pub const EARTH_JPG: &[u8] = include_bytes!("../assets/earth.jpg");

/// Orientation et taille du globe à l'écran.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Longitude et latitude du point au centre du globe (radians).
    pub lon: f32,
    pub lat: f32,
    pub center: Pos2,
    /// Rayon du globe (points).
    pub radius: f32,
}

impl View {
    /// Position à l'écran d'un point (degrés), et sa profondeur (1 au centre, 0 sur le bord) ;
    /// `None` s'il est sur la face cachée.
    pub fn project(&self, lat_deg: f32, lon_deg: f32) -> Option<(Pos2, f32)> {
        let (phi, lam) = (lat_deg.to_radians(), lon_deg.to_radians());
        let x1 = phi.cos() * (lam - self.lon).sin();
        let y1 = phi.sin();
        let z1 = phi.cos() * (lam - self.lon).cos();
        let y2 = y1 * self.lat.cos() - z1 * self.lat.sin();
        let z2 = y1 * self.lat.sin() + z1 * self.lat.cos();
        (z2 > 0.0).then(|| {
            (
                pos2(self.center.x + self.radius * x1, self.center.y - self.radius * y2),
                z2,
            )
        })
    }

    /// Point du globe (latitude, longitude en degrés) sous une position de l'écran.
    pub fn unproject(&self, p: Pos2) -> Option<(f32, f32)> {
        let x2 = (p.x - self.center.x) / self.radius;
        let y2 = (self.center.y - p.y) / self.radius;
        let r2 = x2 * x2 + y2 * y2;
        if r2 > 1.0 {
            return None;
        }
        let z2 = (1.0 - r2).sqrt();
        let y1 = y2 * self.lat.cos() + z2 * self.lat.sin();
        let z1 = -y2 * self.lat.sin() + z2 * self.lat.cos();
        let phi = y1.clamp(-1.0, 1.0).asin();
        let lam = self.lon + x2.atan2(z1);
        Some((phi.to_degrees(), wrap_deg(lam.to_degrees())))
    }

    /// Fait tourner le globe d'un glissé de souris (points) : le point saisi suit le pointeur.
    pub fn drag(&mut self, delta: egui::Vec2) {
        self.lon -= delta.x / self.radius / self.lat.cos().max(0.2);
        self.lat = (self.lat + delta.y / self.radius).clamp(-MAX_LAT, MAX_LAT);
        self.lon = wrap_rad(self.lon);
    }
}

pub fn wrap_rad(a: f32) -> f32 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

fn wrap_deg(a: f32) -> f32 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

/// Latitude du centre, bornée pour ne pas basculer par-dessus les pôles.
pub const MAX_LAT: f32 = FRAC_PI_2 * 0.92;

/// 0 : inconnu, 1 : fonctionne, 2 : refusé par le pilote.
static STATE: AtomicU8 = AtomicU8::new(0);
static RESOURCES: Mutex<Option<Option<Resources>>> = Mutex::new(None);

struct Resources {
    program: glow::Program,
    vao: glow::VertexArray,
    loc: Vec<Option<glow::UniformLocation>>,
    srgb_textures: bool,
}

const UNIFORMS: [&str; 10] = [
    "u_rect",
    "u_globe",
    "u_rot",
    "u_tex",
    "u_srgb",
    "u_glow",
    "u_corner",
    "u_detail",
    "u_detail_box",
    "u_detail_alpha",
];

/// Image détaillée d'une région, posée sur la Terre de base quand on zoome.
#[derive(Clone, Copy, Debug)]
pub struct Detail {
    pub texture: egui::TextureId,
    /// Latitude et longitude minimales, puis maximales (degrés).
    pub bounds: [f32; 4],
    /// Opacité (fondu d'apparition).
    pub alpha: f32,
}

/// Ce qu'il faut pour dessiner une image du globe.
#[derive(Clone, Copy)]
pub struct Frame {
    pub view: View,
    /// Image de la Terre entière.
    pub earth: egui::TextureId,
    pub detail: Option<Detail>,
    /// Couleur de l'atmosphère.
    pub glow: egui::Color32,
    /// Rayon du coin haut gauche de la zone (points).
    pub corner: f32,
}

/// Faux si le pilote a refusé le shader (le globe est alors dessiné sans l'image).
pub fn available() -> bool {
    STATE.load(Ordering::Relaxed) != 2
}

/// Dessine le globe dans `rect`.
pub fn paint(painter: &egui::Painter, rect: Rect, frame: Frame) {
    let callback = egui_glow::CallbackFn::new(move |info, painter| {
        let mut guard = RESOURCES.lock().unwrap();
        let resources = guard.get_or_insert_with(|| {
            let r = unsafe { create(painter.gl()) };
            STATE.store(if r.is_some() { 1 } else { 2 }, Ordering::Relaxed);
            r
        });
        let (Some(res), Some(earth)) = (resources.as_ref(), painter.texture(frame.earth)) else {
            return;
        };
        let detail = frame
            .detail
            .and_then(|d| painter.texture(d.texture).map(|t| (t, d)));
        unsafe { draw(painter.gl(), res, &info, rect, &frame, earth, detail) };
    });
    painter.add(egui::Shape::Callback(egui::PaintCallback {
        rect,
        callback: Arc::new(callback),
    }));
}

const FRAGMENT: &str = r#"
uniform vec4 u_rect;    // viewport : x0, y0 (bas gauche), largeur, hauteur en pixels
uniform vec3 u_globe;   // centre x, y (pixels depuis le bas gauche du viewport), rayon
uniform vec2 u_rot;     // longitude, latitude du centre (radians)
uniform sampler2D u_tex;
uniform int u_srgb;
uniform vec3 u_glow;    // couleur de l'atmosphère
uniform float u_corner; // rayon du coin haut gauche (pixels), celui de la zone de contenu
uniform sampler2D u_detail;     // image détaillée d'une région
uniform vec4 u_detail_box;      // latitude min, longitude min, latitude max, longitude max (degrés)
uniform float u_detail_alpha;   // 0 : pas d'image détaillée
out vec4 out_color;

const float PI = 3.14159265;

vec3 srgb_from_linear(vec3 c) {
    return mix(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, vec3(lessThan(c, vec3(0.0031308))));
}

float corner_mask(vec2 p) {
    vec2 q = vec2(p.x, u_rect.w - p.y);  // distance au coin haut gauche
    if (q.x >= u_corner || q.y >= u_corner) return 1.0;
    float d = length(vec2(u_corner) - q) - u_corner;
    return clamp(0.5 - d, 0.0, 1.0);
}

void main() {
    vec2 p = gl_FragCoord.xy - u_rect.xy;
    float mask = corner_mask(p);
    vec2 d = (p - u_globe.xy) / u_globe.z;
    float r = length(d);
    float aa = 1.5 / u_globe.z;
    // Atmosphère : halo autour du disque.
    float halo = exp(-max(r - 1.0, 0.0) * 18.0) * smoothstep(1.0 - aa, 1.0 + aa, r);
    vec4 outside = vec4(u_glow * halo * 0.55, halo * 0.55);
    if (r > 1.0 + aa) {
        out_color = outside * mask;
        return;
    }
    float z = sqrt(max(1.0 - r * r, 0.0));
    float sl = sin(u_rot.y), cl = cos(u_rot.y);
    float y1 = d.y * cl + z * sl;
    float z1 = -d.y * sl + z * cl;
    float phi = asin(clamp(y1, -1.0, 1.0));
    float lam = u_rot.x + atan(d.x, z1);
    vec2 uv = vec2(fract(lam / (2.0 * PI) + 0.5), 0.5 - phi / PI);
    // Dérivées calculées sans le saut de longitude à ±180° (sinon une ligne floue apparaît).
    vec2 dx = dFdx(uv), dy = dFdy(uv);
    dx.x -= sign(dx.x) * step(0.5, abs(dx.x));
    dy.x -= sign(dy.x) * step(0.5, abs(dy.x));
    vec3 col = textureGrad(u_tex, uv, dx, dy).rgb;
    // Image détaillée de la région affichée, fondue sur ses bords dans la Terre de base.
    if (u_detail_alpha > 0.0) {
        float lat = degrees(phi);
        float lon = mod(degrees(lam) + 180.0, 360.0) - 180.0;
        vec2 duv = vec2((lon - u_detail_box.y) / (u_detail_box.w - u_detail_box.y),
                        (u_detail_box.z - lat) / (u_detail_box.z - u_detail_box.x));
        if (duv.x > 0.0 && duv.x < 1.0 && duv.y > 0.0 && duv.y < 1.0) {
            vec2 edge = min(duv, 1.0 - duv);
            float feather = smoothstep(0.0, 0.06, min(edge.x, edge.y));
            vec3 dcol = textureGrad(u_detail, duv, dFdx(duv), dFdy(duv)).rgb;
            col = mix(col, dcol, feather * u_detail_alpha);
        }
    }
    if (u_srgb == 1) col = srgb_from_linear(col);
    // Aux pôles, l'image se resserre en un point (traînées en étoile) : on la fond dans la
    // teinte moyenne de ses bords (océan Arctique en haut, banquise en bas).
    vec3 pole = phi > 0.0 ? vec3(6.0, 17.0, 46.0) / 255.0 : vec3(241.0, 241.0, 241.0) / 255.0;
    col = mix(col, pole, smoothstep(1.32, 1.48, abs(phi)));
    // Lumière douce venue du haut à gauche, assombrissement vers le bord, liseré d'atmosphère.
    vec3 n = vec3(d, z);
    float light = 0.55 + 0.55 * max(dot(n, normalize(vec3(-0.35, 0.45, 0.85))), 0.0);
    col *= light;
    float rim = pow(1.0 - z, 3.0);
    col = mix(col, u_glow, rim * 0.6);
    float edge = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, r);
    vec4 inside = vec4(col, 1.0) * edge;
    out_color = (inside + outside * (1.0 - inside.a)) * mask;
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
                eprintln!("SterMusic, shader du globe : {}", gl.get_shader_info_log(shader));
                return None;
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            eprintln!(
                "SterMusic, shader du globe : {}",
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
    rect: Rect,
    frame: &Frame,
    earth: glow::Texture,
    detail: Option<(glow::Texture, Detail)>,
) {
    let vp = info.viewport_in_pixels();
    let ppp = info.pixels_per_point;
    let l = |name: &str| res.loc[UNIFORMS.iter().position(|n| *n == name).unwrap()].as_ref();
    let view = frame.view;
    let glow_color = egui::Rgba::from(frame.glow);
    // Centre du globe en pixels, depuis le coin bas gauche du rectangle de dessin.
    let cx = (view.center.x - rect.left()) * ppp;
    let cy = (rect.bottom() - view.center.y) * ppp;
    unsafe {
        gl.use_program(Some(res.program));
        gl.bind_vertex_array(Some(res.vao));
        gl.uniform_4_f32(
            l("u_rect"),
            vp.left_px as f32,
            vp.from_bottom_px as f32,
            vp.width_px as f32,
            vp.height_px as f32,
        );
        gl.uniform_3_f32(l("u_globe"), cx, cy, view.radius * ppp);
        gl.uniform_2_f32(l("u_rot"), view.lon, view.lat);
        gl.uniform_1_i32(l("u_srgb"), res.srgb_textures as i32);
        gl.uniform_3_f32(l("u_glow"), glow_color.r(), glow_color.g(), glow_color.b());
        gl.uniform_1_f32(l("u_corner"), frame.corner * ppp);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(earth));
        gl.uniform_1_i32(l("u_tex"), 0);
        match detail {
            Some((tex, d)) => {
                gl.active_texture(glow::TEXTURE1);
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gl.uniform_1_i32(l("u_detail"), 1);
                let [a, b, c, e] = d.bounds;
                gl.uniform_4_f32(l("u_detail_box"), a, b, c, e);
                gl.uniform_1_f32(l("u_detail_alpha"), d.alpha);
            }
            None => gl.uniform_1_f32(l("u_detail_alpha"), 0.0),
        }
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        gl.bind_vertex_array(None);
        if detail.is_some() {
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
        }
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.use_program(None);
    }
}

/// Résolution la plus fine de l'image détaillée de la NASA (pixels par degré, 500 m environ).
pub const DETAIL_MAX_PX_PER_DEG: f32 = 220.0;

/// Télécharge l'image satellite d'une région (NASA GIBS, Blue Marble avec relief, domaine
/// public) : `bounds` en degrés, comme [`Detail::bounds`] ; bloquant.
pub fn fetch_detail(bounds: [f32; 4], width: u32, height: u32) -> Option<egui::ColorImage> {
    let [lat0, lon0, lat1, lon1] = bounds;
    let url = format!(
        "https://gibs.earthdata.nasa.gov/wms/epsg4326/best/wms.cgi?SERVICE=WMS&REQUEST=GetMap\
         &VERSION=1.3.0&LAYERS=BlueMarble_ShadedRelief_Bathymetry&STYLES=&CRS=EPSG:4326\
         &BBOX={lat0:.4},{lon0:.4},{lat1:.4},{lon1:.4}&WIDTH={width}&HEIGHT={height}&FORMAT=image/jpeg"
    );
    let resp = ureq::AgentBuilder::new()
        .user_agent(concat!("SterMusic/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .get(&url)
        .call()
        .ok()?;
    if !resp.content_type().starts_with("image/") {
        return None;
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(resp.into_reader(), 16 << 20), &mut bytes).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [img.width() as usize, img.height() as usize],
        img.as_raw(),
    ))
}

/// Image de la Terre décodée, prête à devenir une texture (à faire hors du thread de l'interface).
pub fn decode_earth() -> Option<egui::ColorImage> {
    let img = image::load_from_memory_with_format(EARTH_JPG, image::ImageFormat::Jpeg)
        .ok()?
        .to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [img.width() as usize, img.height() as usize],
        img.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_round_trip() {
        let view = View {
            lon: 2.35f32.to_radians(),
            lat: 48.85f32.to_radians(),
            center: pos2(400.0, 300.0),
            radius: 250.0,
        };
        // Le point au centre de la vue tombe au centre du globe.
        let (p, z) = view.project(48.85, 2.35).unwrap();
        assert!((p - view.center).length() < 0.01 && (z - 1.0).abs() < 1e-4);
        // Aller-retour écran → globe → écran.
        for (lat, lon) in [(51.5, -0.12), (40.4, -3.7), (59.3, 18.0), (36.8, 10.2)] {
            let (p, _) = view.project(lat, lon).unwrap();
            let (la, lo) = view.unproject(p).unwrap();
            assert!(
                (la - lat).abs() < 0.01 && (lo - lon).abs() < 0.01,
                "{lat},{lon} → {la},{lo}"
            );
        }
        // Les antipodes sont sur la face cachée.
        assert!(view.project(-48.85, -177.65).is_none());
        // Nord en haut : Londres est au-dessus de Paris.
        assert!(view.project(51.5, -0.12).unwrap().0.y < view.center.y);
    }

    #[test]
    fn earth_image() {
        let img = decode_earth().unwrap();
        assert_eq!(img.size, [4096, 2048]);
    }
}
