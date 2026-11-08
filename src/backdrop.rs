//! Fond « Mica », comme dans SterCode : le fond d'écran du bureau, réduit, flouté et fortement
//! teinté vers la couleur du thème. Lit le fond d'écran de KDE Plasma, puis celui de GNOME.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, Rect};

use crate::{anim, theme};

/// Part de la teinte du thème dans le fond (1 = aucune couleur du fond d'écran).
const TINT: f32 = 0.78;

type Slot = Arc<Mutex<Option<Option<egui::ColorImage>>>>;

#[derive(Default)]
pub struct Backdrop {
    job: Option<Slot>,
    texture: Option<egui::TextureHandle>,
    /// Thème pour lequel la teinte a été calculée.
    theme: Option<&'static str>,
}

impl Backdrop {
    /// Dessine le fond de la fenêtre (à appeler avant les panneaux).
    pub fn paint(&mut self, ctx: &egui::Context, enabled: bool) {
        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.rect_filled(screen, 0.0, theme::bg());

        let theme_id = theme::current().id;
        if enabled && self.theme != Some(theme_id) && self.job.is_none() {
            self.theme = Some(theme_id);
            let slot: Slot = Arc::new(Mutex::new(None));
            self.job = Some(slot.clone());
            let ctx = ctx.clone();
            let bg = theme::bg();
            std::thread::spawn(move || {
                *slot.lock().unwrap() = Some(prepare(bg));
                ctx.request_repaint();
            });
        }
        let ready = self.job.as_ref().and_then(|job| job.lock().unwrap().take());
        if let Some(image) = ready {
            self.texture = image.map(|i| ctx.load_texture("mica", i, egui::TextureOptions::LINEAR));
            self.job = None;
        }

        let t = anim::toggle(
            ctx,
            egui::Id::new("mica_fade"),
            enabled && self.texture.is_some(),
            anim::SLOW,
        );
        if let (Some(texture), true) = (&self.texture, t > 0.0) {
            paint_fill(&painter, texture, screen, Color32::WHITE.gamma_multiply(t));
        }
    }
}

/// Dessine une texture qui couvre tout `rect` sans se déformer (recadrée au centre).
pub fn paint_fill(
    painter: &egui::Painter,
    texture: &egui::TextureHandle,
    rect: Rect,
    tint: Color32,
) {
    let [w, h] = texture.size();
    let (tex_ratio, rect_ratio) = (
        w as f32 / h.max(1) as f32,
        rect.width() / rect.height().max(1.0),
    );
    let uv = if tex_ratio > rect_ratio {
        let k = rect_ratio / tex_ratio;
        Rect::from_min_max(
            egui::pos2((1.0 - k) / 2.0, 0.0),
            egui::pos2((1.0 + k) / 2.0, 1.0),
        )
    } else {
        let k = tex_ratio / rect_ratio;
        Rect::from_min_max(
            egui::pos2(0.0, (1.0 - k) / 2.0),
            egui::pos2(1.0, (1.0 + k) / 2.0),
        )
    };
    painter.image(texture.id(), rect, uv, tint);
}

fn prepare(bg: Color32) -> Option<egui::ColorImage> {
    let wall = image::open(wallpaper_path()?).ok()?;
    let small = wall
        .resize(240, 240, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let blurred = image::imageops::blur(&small, 10.0);
    let pixels = blurred
        .pixels()
        .map(|p| {
            let mix = |c: u8, t: u8| (c as f32 * (1.0 - TINT) + t as f32 * TINT).round() as u8;
            Color32::from_rgb(mix(p[0], bg.r()), mix(p[1], bg.g()), mix(p[2], bg.b()))
        })
        .collect();
    Some(egui::ColorImage {
        size: [blurred.width() as usize, blurred.height() as usize],
        pixels,
    })
}

fn wallpaper_path() -> Option<PathBuf> {
    kde_wallpaper().or_else(gnome_wallpaper)
}

/// KDE Plasma : `Image=` dans la configuration du bureau (fichier, ou paquet de fond d'écran).
fn kde_wallpaper() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let text =
        std::fs::read_to_string(config.join("plasma-org.kde.plasma.desktop-appletsrc")).ok()?;
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("Image="))
        .find_map(|v| resolve(&percent_decode(v.trim().trim_start_matches("file://"))))
}

/// GNOME (Ubuntu, Fedora…) : `gsettings`, variante sombre d'abord.
fn gnome_wallpaper() -> Option<PathBuf> {
    ["picture-uri-dark", "picture-uri"].iter().find_map(|key| {
        let out = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.background", key])
            .output()
            .ok()?;
        let uri = String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_matches('\'')
            .to_owned();
        resolve(&percent_decode(uri.strip_prefix("file://")?))
    })
}

fn is_image(p: &Path) -> bool {
    p.extension().is_some_and(|e| {
        matches!(
            e.to_string_lossy().to_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "bmp"
        )
    })
}

/// Un fichier image, ou un paquet de fond d'écran KDE (`contents/images/1920x1080.jpg`…) : on
/// prend alors la plus grande image.
fn resolve(path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path);
    if path.is_file() && is_image(&path) {
        return Some(path);
    }
    if path.is_dir() {
        let images = path.join("contents/images");
        let area = |p: &Path| -> u64 {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            stem.split_once('x')
                .and_then(|(w, h)| Some(w.parse::<u64>().ok()? * h.parse::<u64>().ok()?))
                .unwrap_or(0)
        };
        return std::fs::read_dir(images)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| is_image(p))
            .max_by_key(|p| area(p));
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
