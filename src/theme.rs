//! Charte visuelle, reprise de SterCode : fonds bleu nuit doux, calques translucides,
//! dégradé de marque, coins arrondis à la Fluent. Les thèmes changent le fond et le dégradé.
//!
//! Style « Breeze » (Kubuntu) : couleurs et accent de KDE (`kdeglobals`), police Noto Sans,
//! icônes du thème Breeze, boutons de fenêtre ronds.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Rect, Shadow, Stroke,
    TextStyle,
};

pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub bg: Color32,
    pub accent: Color32,
    pub accent_2: Color32,
    /// Accent éclairci, lisible sur fond sombre.
    pub accent_light: Color32,
}

const fn theme(
    id: &'static str,
    name: &'static str,
    bg: u32,
    accent: u32,
    accent_2: u32,
    accent_light: u32,
) -> Theme {
    const fn rgb(hex: u32) -> Color32 {
        Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
    }
    Theme {
        id,
        name,
        bg: rgb(bg),
        accent: rgb(accent),
        accent_2: rgb(accent_2),
        accent_light: rgb(accent_light),
    }
}

/// Les mêmes seize thèmes que SterCode.
pub const THEMES: [Theme; 16] = [
    theme(
        "stercode",
        crate::i18n::n_("SterMusic"),
        0x14151D,
        0x5468FF,
        0x9A5CF5,
        0x8E9BFF,
    ),
    theme(
        "ocean",
        crate::i18n::n_("Océan"),
        0x0F171C,
        0x2BB3C0,
        0x3D7BFF,
        0x7AD7E0,
    ),
    theme(
        "foret",
        crate::i18n::n_("Forêt"),
        0x111814,
        0x3FB27F,
        0x9BCB3C,
        0x86D9AE,
    ),
    theme(
        "couchant",
        crate::i18n::n_("Couchant"),
        0x1A1214,
        0xFF7A45,
        0xFF4F8B,
        0xFFAA85,
    ),
    theme(
        "sakura",
        crate::i18n::n_("Sakura"),
        0x1A1219,
        0xF06BA8,
        0xB86BF0,
        0xF7A6CB,
    ),
    theme(
        "lavande",
        crate::i18n::n_("Lavande"),
        0x15131E,
        0x9D7BFF,
        0xE07BFF,
        0xC3B0FF,
    ),
    theme(
        "ambre",
        crate::i18n::n_("Ambre"),
        0x18150F,
        0xF2A93B,
        0xF26B3B,
        0xF7CB86,
    ),
    theme(
        "rubis",
        crate::i18n::n_("Rubis"),
        0x1A1113,
        0xE5484D,
        0xF27B3B,
        0xF28A8D,
    ),
    theme(
        "menthe",
        crate::i18n::n_("Menthe"),
        0x0F1918,
        0x3DD6B5,
        0x3DA0D6,
        0x8AE8D4,
    ),
    theme(
        "aurore",
        crate::i18n::n_("Aurore"),
        0x10161C,
        0x5EC4E8,
        0xA3E37A,
        0x9EDDF2,
    ),
    theme(
        "graphite",
        crate::i18n::n_("Graphite"),
        0x141516,
        0xA0A8B8,
        0x6E7686,
        0xC8CEDA,
    ),
    theme(
        "cosmos",
        crate::i18n::n_("Cosmos"),
        0x10101C,
        0x6C5CE7,
        0x00CEC9,
        0xA99FF5,
    ),
    theme(
        "citron",
        crate::i18n::n_("Citron vert"),
        0x14170F,
        0xB5D93C,
        0x5CC97A,
        0xD3EC85,
    ),
    theme(
        "corail",
        crate::i18n::n_("Corail"),
        0x1B1312,
        0xFF6F61,
        0xFFB25B,
        0xFFA69C,
    ),
    theme(
        "glacier",
        crate::i18n::n_("Glacier"),
        0x121620,
        0x7FB8FF,
        0xB8E0FF,
        0xB5D6FF,
    ),
    theme(
        "amethyste",
        crate::i18n::n_("Améthyste"),
        0x17121F,
        0xB15CF5,
        0x5C7CF5,
        0xD2A3FA,
    ),
];

/// Thème en cours. Tous les thèmes vivent jusqu'à la fin du programme (constantes, thème du
/// système, favoris), d'où un simple pointeur : `current()` est appelé des centaines de fois par image.
static CURRENT: AtomicPtr<Theme> = AtomicPtr::new(std::ptr::null_mut());
static BREEZE: AtomicBool = AtomicBool::new(false);
/// Thème adaptatif : couleurs qui remplacent celles du thème choisi (fond, accent, second
/// accent, accent clair), tirées de la pochette du morceau en cours.
static ADAPTIVE_ON: AtomicBool = AtomicBool::new(false);
static ADAPTIVE: [AtomicU32; 4] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
static RADIUS: AtomicU8 = AtomicU8::new(0);

/// Thème « système » : couleurs de KDE (`~/.config/kdeglobals`) ou accent de GNOME. L'identifiant
/// est resté « kde » (réglages des versions précédentes).
pub const KDE_ID: &str = "kde";
const KDE_NAME: &str = "KDE (système)";
const GNOME_NAME: &str = "GNOME (système)";
static SYSTEM: OnceLock<Option<Theme>> = OnceLock::new();

/// Le thème du bureau (KDE, sinon GNOME), s'il y en a un.
pub fn system_theme() -> Option<&'static Theme> {
    SYSTEM.get_or_init(|| load_kde().or_else(load_gnome)).as_ref()
}

/// Le thème de KDE, s'il y a une configuration KDE.
pub fn kde_theme() -> Option<&'static Theme> {
    system_theme().filter(|t| t.name == KDE_NAME)
}

/// Nom court du thème du système, sous sa vignette.
pub fn system_label() -> &'static str {
    match system_theme() {
        Some(t) if t.name == GNOME_NAME => "GNOME",
        _ => "KDE",
    }
}

/// Thème enregistré en favori : une copie figée des couleurs d'un thème (en général celui du
/// bureau, qui change avec les réglages de KDE ou de GNOME).
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedTheme {
    pub id: String,
    pub name: String,
    pub bg: [u8; 3],
    pub accent: [u8; 3],
    pub accent_2: [u8; 3],
    pub accent_light: [u8; 3],
}

impl SavedTheme {
    /// Copie du thème en cours, sous ce nom.
    pub fn from_current(name: String) -> Self {
        let t = current();
        let rgb = |c: Color32| [c.r(), c.g(), c.b()];
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        Self {
            id: format!("fav-{now:x}"),
            name,
            bg: rgb(t.bg),
            accent: rgb(t.accent),
            accent_2: rgb(t.accent_2),
            accent_light: rgb(t.accent_light),
        }
    }
}

/// Favoris chargés (copies `'static`, voir `CURRENT`).
static SAVED: Mutex<Vec<&'static Theme>> = Mutex::new(Vec::new());

/// Remplace la liste des favoris. Les anciennes copies restent en mémoire (quelques octets par
/// modification) : le thème en cours peut encore les désigner.
pub fn set_saved(themes: &[SavedTheme]) {
    let rgb = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
    let leaked = themes
        .iter()
        .map(|s| {
            &*Box::leak(Box::new(Theme {
                id: Box::leak(s.id.clone().into_boxed_str()),
                name: Box::leak(s.name.clone().into_boxed_str()),
                bg: rgb(s.bg),
                accent: rgb(s.accent),
                accent_2: rgb(s.accent_2),
                accent_light: rgb(s.accent_light),
            }))
        })
        .collect();
    *SAVED.lock().unwrap() = leaked;
}

/// Les favoris, dans l'ordre où ils ont été enregistrés.
pub fn saved() -> Vec<&'static Theme> {
    SAVED.lock().unwrap().clone()
}

/// Un nom pour enregistrer le thème en cours : « KDE du 9 novembre ».
pub fn suggested_name() -> String {
    let t = current();
    let base = if t.id == KDE_ID {
        system_label()
    } else {
        crate::i18n::tr(t.name)
    };
    let (_, month, day) = today();
    let month = (month as usize).saturating_sub(1).min(11);
    match crate::i18n::lang() {
        crate::i18n::Lang::Fr => {
            const MONTHS: [&str; 12] = [
                "janvier",
                "février",
                "mars",
                "avril",
                "mai",
                "juin",
                "juillet",
                "août",
                "septembre",
                "octobre",
                "novembre",
                "décembre",
            ];
            format!("{base} du {day} {}", MONTHS[month])
        }
        crate::i18n::Lang::En => {
            const MONTHS: [&str; 12] = [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ];
            format!("{base}, {} {day}", MONTHS[month])
        }
    }
}

/// Date du jour, à l'heure locale (commande `date`), sinon en temps universel.
fn today() -> (i64, u32, u32) {
    let local = std::process::Command::new("date")
        .arg("+%Y %m %d")
        .output()
        .ok()
        .and_then(|o| {
            let text = String::from_utf8(o.stdout).ok()?;
            let mut parts = text.split_whitespace().map(|p| p.parse::<i64>().ok());
            Some((parts.next()??, parts.next()?? as u32, parts.next()?? as u32))
        });
    local.unwrap_or_else(utc_today)
}

/// Date du jour en temps universel (année, mois, jour), sans dépendance : calendrier grégorien
/// depuis l'époque Unix.
fn utc_today() -> (i64, u32, u32) {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64 / 86_400)
        .unwrap_or(0);
    // Algorithme de Howard Hinnant (« civil_from_days »).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// Thème du système sous GNOME : son accent (GNOME 47 et suivants) sur le fond sombre d'Adwaita.
fn load_gnome() -> Option<Theme> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_uppercase();
    if !desktop.contains("GNOME") && !desktop.contains("UNITY") {
        return None;
    }
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "accent-color"])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout)
        .trim()
        .trim_matches('\'')
        .to_owned();
    let hex = match name.as_str() {
        "teal" => 0x2190A4,
        "green" => 0x3A944A,
        "yellow" => 0xC88800,
        "orange" => 0xED5B00,
        "red" => 0xE62D42,
        "pink" => 0xD56199,
        "purple" => 0x9141AC,
        "slate" => 0x6F8396,
        _ => 0x3584E4,
    };
    let accent = Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
    Some(Theme {
        id: KDE_ID,
        name: GNOME_NAME,
        bg: Color32::from_rgb(0x22, 0x22, 0x26),
        accent,
        accent_2: mix(accent, Color32::WHITE, 0.16),
        accent_light: mix(accent, Color32::WHITE, 0.45),
    })
}

/// Vrai sous KDE Plasma (Kubuntu…).
pub fn is_kde_desktop() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("KDE"))
}

fn load_kde() -> Option<Theme> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let text = std::fs::read_to_string(config.join("kdeglobals")).ok()?;
    let mut group = String::new();
    let mut values = std::collections::HashMap::new();
    for line in text.lines().map(str::trim) {
        if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = g.to_owned();
        } else if let Some((k, v)) = line.split_once('=') {
            values.insert(format!("{group}/{}", k.trim()), v.trim().to_owned());
        }
    }
    let color = |key: &str| -> Option<Color32> {
        let v = values.get(key)?;
        let c: Vec<u8> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (c.len() >= 3).then(|| Color32::from_rgb(c[0], c[1], c[2]))
    };
    // Breeze sombre par défaut, si le jeu de couleurs ne précise rien.
    let bg = color("Colors:Window/BackgroundNormal").unwrap_or(Color32::from_rgb(0x20, 0x23, 0x26));
    let accent = color("General/AccentColor")
        .or_else(|| color("Colors:Window/DecorationFocus"))
        .or_else(|| color("Colors:Selection/BackgroundNormal"))
        .unwrap_or(Color32::from_rgb(0x3D, 0xAE, 0xE9));
    // Fond clair (jeu de couleurs clair) : SterMusic reste sombre, on assombrit.
    let bg = if bg.r() as u32 + bg.g() as u32 + bg.b() as u32 > 3 * 110 {
        mix(bg, Color32::BLACK, 0.85)
    } else {
        bg
    };
    Some(Theme {
        id: KDE_ID,
        name: KDE_NAME,
        bg,
        accent,
        // Breeze est plat : un « dégradé » à peine marqué.
        accent_2: mix(accent, Color32::WHITE, 0.16),
        accent_light: mix(accent, Color32::WHITE, 0.45),
    })
}

/// Style Breeze (Kubuntu) activé ?
pub fn breeze() -> bool {
    BREEZE.load(Ordering::Relaxed)
}

/// Active ou non le style Breeze (police, icônes, boutons de fenêtre).
pub fn set_breeze(ctx: &egui::Context, on: bool) {
    if BREEZE.swap(on, Ordering::Relaxed) != on {
        install_fonts(ctx);
    }
}

/// Rayon des coins de la fenêtre (0 quand elle est agrandie).
pub fn window_radius() -> u8 {
    RADIUS.load(Ordering::Relaxed)
}

pub fn set_window_radius(r: u8) {
    RADIUS.store(r, Ordering::Relaxed);
}

pub fn current() -> &'static Theme {
    let p = CURRENT.load(Ordering::Relaxed);
    if p.is_null() {
        &THEMES[0]
    } else {
        // SAFETY: `CURRENT` ne reçoit que des références `'static` (voir `set`).
        unsafe { &*p }
    }
}

/// Change de thème (identifiant inconnu : le premier) et met à jour les couleurs d'egui.
pub fn set(ctx: &egui::Context, id: &str) {
    let theme: &'static Theme = if id == KDE_ID {
        system_theme().unwrap_or(&THEMES[0])
    } else if let Some(t) = saved().into_iter().find(|t| t.id == id) {
        t
    } else {
        THEMES.iter().find(|t| t.id == id).unwrap_or(&THEMES[0])
    };
    CURRENT.store(theme as *const Theme as *mut Theme, Ordering::Relaxed);
    ctx.style_mut_of(egui::Theme::Dark, style);
}

/// Remplace (ou rend, avec `None`) les couleurs du thème choisi par celles-ci : fond, accent,
/// second accent, accent clair. Les couleurs d'egui ne sont refaites que si quelque chose change.
pub fn set_adaptive(ctx: &egui::Context, colors: Option<[Color32; 4]>) {
    let pack = |c: Color32| u32::from_be_bytes([c.r(), c.g(), c.b(), 255]);
    let was = ADAPTIVE_ON.load(Ordering::Relaxed);
    let mut changed = was != colors.is_some();
    if let Some(colors) = colors {
        for (slot, c) in ADAPTIVE.iter().zip(colors) {
            changed |= slot.swap(pack(c), Ordering::Relaxed) != pack(c);
        }
    }
    ADAPTIVE_ON.store(colors.is_some(), Ordering::Relaxed);
    if changed {
        ctx.style_mut_of(egui::Theme::Dark, style);
    }
}

/// Couleur `i` du thème adaptatif, s'il est en place.
fn adaptive(i: usize) -> Option<Color32> {
    if !ADAPTIVE_ON.load(Ordering::Relaxed) {
        return None;
    }
    let [r, g, b, _] = ADAPTIVE[i].load(Ordering::Relaxed).to_be_bytes();
    Some(Color32::from_rgb(r, g, b))
}

fn lighten(c: Color32, r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(
        c.r().saturating_add(r),
        c.g().saturating_add(g),
        c.b().saturating_add(b),
    )
}

/// Fond de la fenêtre.
pub fn bg() -> Color32 {
    adaptive(0).unwrap_or(current().bg)
}

/// Menus et fenêtres flottantes.
pub fn glass() -> Color32 {
    lighten(bg(), 10, 10, 14)
}

pub fn accent() -> Color32 {
    adaptive(1).unwrap_or(current().accent)
}

pub fn accent_2() -> Color32 {
    adaptive(2).unwrap_or(current().accent_2)
}

pub fn accent_light() -> Color32 {
    adaptive(3).unwrap_or(current().accent_light)
}

pub const TEXT: Color32 = Color32::from_rgb(0xEB, 0xEE, 0xFA);
pub const TEXT_2: Color32 = Color32::from_rgb(0xAC, 0xAF, 0xBC);
/// Icônes au repos : un peu plus soutenu que `TEXT_2` (les icônes fines paraissaient pâles).
pub fn icon_idle() -> Color32 {
    mix(TEXT_2, TEXT, 0.35)
}

pub const TEXT_3: Color32 = Color32::from_rgb(0x75, 0x78, 0x86);
pub const DANGER: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
/// Volume amplifié (au-dessus de 100 %).
pub const BOOST: Color32 = Color32::from_rgb(0xFF, 0xA0, 0x3C);

/// Couleur translucide mélangée comme en CSS (dans l'espace sRGB).
pub fn rgba(r: u8, g: u8, b: u8, alpha: u8) -> Color32 {
    let m = |c: u8| ((c as u16 * alpha as u16 + 127) / 255) as u8;
    Color32::from_rgba_premultiplied(m(r), m(g), m(b), alpha)
}

/// Blanc translucide (calques, bordures, survols).
pub fn white(alpha: u8) -> Color32 {
    rgba(0xFF, 0xFF, 0xFF, alpha)
}

/// Couleur avec une opacité (en sRGB).
pub fn alpha(c: Color32, a: u8) -> Color32 {
    rgba(c.r(), c.g(), c.b(), a)
}

pub fn stroke() -> Color32 {
    white(0x12)
}

pub fn stroke_strong() -> Color32 {
    white(0x24)
}

/// Survol, teinté de l'accent.
pub fn hover() -> Color32 {
    let tint = mix(accent_light(), Color32::WHITE, 0.5);
    rgba(tint.r(), tint.g(), tint.b(), 0x12)
}

pub fn accent_soft() -> Color32 {
    alpha(accent(), 0x33)
}

pub fn selection() -> Color32 {
    alpha(accent(), 0x59)
}

/// Police « semi-gras » des titres (les grands titres restent en graisse normale avec Breeze).
pub fn semibold(size: f32) -> FontId {
    let family = if size >= 19.5 { "heading" } else { "semibold" };
    FontId::new(size, FontFamily::Name(family.into()))
}

/// Interpolation linéaire entre deux couleurs.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// Rectangle arrondi rempli du dégradé de marque (135°).
pub fn brand_fill(painter: &egui::Painter, rect: Rect, radius: impl Into<CornerRadius>, alpha: f32) {
    // egui ignore la texture des rectangles très fins : dégradé par sommets.
    if rect.height() < 4.0 || rect.width() < 4.0 {
        let (a, b) = (accent().gamma_multiply(alpha), accent_2().gamma_multiply(alpha));
        let mut mesh = egui::Mesh::default();
        let horizontal = rect.width() >= rect.height();
        for (corner, color) in [
            (rect.left_top(), a),
            (rect.right_top(), if horizontal { b } else { a }),
            (rect.right_bottom(), b),
            (rect.left_bottom(), if horizontal { a } else { b }),
        ] {
            mesh.colored_vertex(corner, color);
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
        return;
    }
    let texture = brand_texture(painter.ctx());
    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let shape = egui::epaint::RectShape::filled(rect, radius, Color32::WHITE.gamma_multiply(alpha))
        .with_texture(texture.id(), uv);
    painter.add(shape);
}

fn brand_texture(ctx: &egui::Context) -> egui::TextureHandle {
    // Une seule texture gardée, refaite quand les couleurs changent (thème adaptatif compris).
    let id = egui::Id::new("brand_gradient_texture");
    let key = (accent(), accent_2());
    if let Some((k, texture)) = ctx.data(|d| d.get_temp::<((Color32, Color32), egui::TextureHandle)>(id)) {
        if k == key {
            return texture;
        }
    }
    let size = 32;
    let mut pixels = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let t = (x + y) as f32 / (2 * (size - 1)) as f32;
            pixels.push(mix(accent(), accent_2(), t));
        }
    }
    let image = egui::ColorImage {
        size: [size, size],
        pixels,
    };
    let texture = ctx.load_texture("brand_gradient", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, (key, texture.clone())));
    texture
}

/// Deux couleurs stables tirées d'un texte (pochettes de remplacement, avatars d'artistes).
pub fn seeded_colors(seed: &str) -> (Color32, Color32) {
    let mut h: u32 = 2166136261;
    for b in seed.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    let t = (h % 1000) as f32 / 1000.0;
    let base = mix(accent(), accent_2(), t);
    let dark = mix(base, bg(), 0.55);
    let other = mix(mix(accent_2(), accent(), t), bg(), 0.25);
    (dark, other)
}

pub fn install(ctx: &egui::Context, theme_id: &str, breeze: bool) {
    BREEZE.store(breeze, Ordering::Relaxed);
    install_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    set(ctx, theme_id);
}

fn style(style: &mut egui::Style) {
    style.animation_time = 0.18;
    style.scroll_animation = egui::style::ScrollAnimation {
        points_per_second: 2600.0,
        duration: egui::Rangef::new(0.12, 0.3),
    };
    style.text_styles = [
        (TextStyle::Heading, semibold(28.0)),
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Small, FontId::proportional(12.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();

    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 6.0);
    s.button_padding = egui::vec2(10.0, 5.0);
    s.menu_margin = Margin::same(6);
    s.window_margin = Margin::same(16);
    s.interact_size.y = 26.0;
    s.scroll = egui::style::ScrollStyle::floating();
    s.scroll.bar_width = 8.0;
    s.scroll.floating_allocated_width = 0.0;

    let v = &mut style.visuals;
    v.dark_mode = true;
    // Les panneaux sont transparents : le fond (Mica) est peint par l'application.
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = glass();
    v.extreme_bg_color = white(0x0F);
    v.faint_bg_color = white(0x06);
    v.code_bg_color = white(0x0F);
    v.hyperlink_color = accent_light();
    v.window_stroke = Stroke::new(1.0_f32, white(0x1C));
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(130),
    };
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.selection.bg_fill = selection();
    v.selection.stroke = Stroke::new(1.0_f32, TEXT);
    v.text_cursor.stroke = Stroke::new(2.0_f32, accent_light());
    v.slider_trailing_fill = true;
    v.striped = false;

    let radius = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = bg();
    w.noninteractive.weak_bg_fill = bg();
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, stroke());
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = white(0x0F);
    w.inactive.weak_bg_fill = white(0x0F);
    w.inactive.bg_stroke = Stroke::new(1.0_f32, stroke());
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = white(0x17);
    w.hovered.weak_bg_fill = white(0x17);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, stroke_strong());
    w.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = white(0x0D);
    w.active.weak_bg_fill = white(0x0D);
    w.active.bg_stroke = Stroke::new(1.0_f32, stroke_strong());
    w.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = hover();
    w.open.weak_bg_fill = hover();
    w.open.bg_stroke = Stroke::NONE;
    w.open.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    w.open.corner_radius = radius;
}

/// Polices habituelles des distributions (Ubuntu, Noto, DejaVu, Liberation), dans les dossiers
/// du système puis de l'utilisateur : interface, semi-gras, code.
fn system_fonts(breeze: bool) -> [Vec<PathBuf>; 4] {
    let mut dirs: Vec<PathBuf> = ["/usr/share/fonts", "/usr/local/share/fonts"]
        .iter()
        .map(PathBuf::from)
        .collect();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/fonts"));
        dirs.push(PathBuf::from(home).join(".fonts"));
    }
    let files = |names: &[&str]| -> Vec<PathBuf> {
        names
            .iter()
            .flat_map(|name| dirs.iter().map(move |d| d.join(name)))
            .collect()
    };
    if breeze {
        // Kubuntu : Noto Sans partout ; les grands titres en graisse normale, comme Kirigami.
        let regular = files(&[
            "truetype/noto/NotoSans-Regular.ttf",
            "noto/NotoSans-Regular.ttf",
            "truetype/dejavu/DejaVuSans.ttf",
        ]);
        return [
            regular.clone(),
            files(&[
                "truetype/noto/NotoSans-SemiBold.ttf",
                "noto/NotoSans-SemiBold.ttf",
                "truetype/noto/NotoSans-Medium.ttf",
                "truetype/noto/NotoSans-Bold.ttf",
                "noto/NotoSans-Bold.ttf",
            ]),
            regular,
            files(&[
                "truetype/noto/NotoSansMono-Regular.ttf",
                "truetype/hack/Hack-Regular.ttf",
                "truetype/dejavu/DejaVuSansMono.ttf",
            ]),
        ];
    }
    let semibold = files(&[
        "truetype/ubuntu/Ubuntu-M.ttf",
        "truetype/noto/NotoSans-SemiBold.ttf",
        "truetype/noto/NotoSans-Bold.ttf",
        "truetype/dejavu/DejaVuSans-Bold.ttf",
        "TTF/DejaVuSans-Bold.ttf",
    ]);
    [
        files(&[
            "truetype/ubuntu/Ubuntu-R.ttf",
            "truetype/noto/NotoSans-Regular.ttf",
            "noto/NotoSans-Regular.ttf",
            "truetype/dejavu/DejaVuSans.ttf",
            "TTF/DejaVuSans.ttf",
            "truetype/liberation/LiberationSans-Regular.ttf",
        ]),
        semibold.clone(),
        semibold,
        files(&[
            "truetype/ubuntu/UbuntuMono-R.ttf",
            "truetype/noto/NotoSansMono-Regular.ttf",
            "truetype/dejavu/DejaVuSansMono.ttf",
            "TTF/DejaVuSansMono.ttf",
        ]),
    ]
}

/// Charge les polices (texte, semi-gras, grands titres, code), en gardant celles d'egui en
/// secours (symboles, émojis).
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut load = |key: &str, files: &[PathBuf]| -> bool {
        for file in files {
            if let Ok(bytes) = std::fs::read(file) {
                fonts
                    .font_data
                    .insert(key.to_owned(), Arc::new(FontData::from_owned(bytes)));
                return true;
            }
        }
        false
    };
    let [ui, semibold, heading, mono] = system_fonts(breeze());
    let has_ui = load("ui", &ui);
    let has_semibold = load("ui-semibold", &semibold);
    let has_heading = load("ui-heading", &heading);
    let has_mono = load("mono", &mono);

    let proportional = fonts.families.get_mut(&FontFamily::Proportional).unwrap();
    if has_ui {
        proportional.insert(0, "ui".into());
    }
    let base = proportional.clone();
    for (family, key, ok) in [
        ("semibold", "ui-semibold", has_semibold),
        ("heading", "ui-heading", has_heading),
    ] {
        let mut list = base.clone();
        if ok {
            list.insert(0, key.into());
        }
        fonts.families.insert(FontFamily::Name(family.into()), list);
    }
    if has_mono {
        fonts
            .families
            .get_mut(&FontFamily::Monospace)
            .unwrap()
            .insert(0, "mono".into());
    }
    // Secours pour le japonais, le chinois et le coréen (titres d'animes, de jeux…) : Noto Sans
    // CJK comme Kubuntu, sinon Droid Sans Fallback.
    let cjk = [
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
    ];
    if let Some(bytes) = cjk.iter().find_map(|f| std::fs::read(f).ok()) {
        fonts
            .font_data
            .insert("cjk".into(), Arc::new(FontData::from_owned(bytes)));
        for list in fonts.families.values_mut() {
            list.push("cjk".into());
        }
    }
    ctx.set_fonts(fonts);
}
