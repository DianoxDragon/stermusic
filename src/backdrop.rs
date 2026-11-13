//! Window background, with rounded corners:
//! - "Mica", as on Windows 11: the desktop wallpaper, heavily blurred, darkened and tinted
//!   towards the theme color, with subtle grain. Under KDE, each screen has its own wallpaper
//!   (including a frame taken from a video wallpaper), and the window shows the part that is
//!   really behind it, as it is moved (position provided by the KWin script);
//! - "Transparent": the window lets the desktop show through, blurred by KWin (Blur / Better
//!   Blur effect);
//! - solid.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use eframe::egui::{self, Color32, CornerRadius, Rect};

use crate::kwin::Geometry;
use crate::{anim, theme};

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Solid,
    /// Strength from 0 (subtle) to 1 (very colorful).
    Mica(f32),
    /// Opacity of the veil from 0 (very transparent) to 1 (almost opaque).
    Transparent(f32),
}

/// Prepared wallpapers: one per screen (output name, e.g. "DP-1"), and a default one.
#[derive(Default)]
struct Prepared {
    by_output: HashMap<String, egui::ColorImage>,
    default: Option<egui::ColorImage>,
}

type Slot = Arc<Mutex<Option<Prepared>>>;

#[derive(Default)]
pub struct Backdrop {
    job: Option<Slot>,
    started: bool,
    by_output: HashMap<String, egui::TextureHandle>,
    default: Option<egui::TextureHandle>,
    noise: Option<egui::TextureHandle>,
    /// Watching the desktop configuration (wallpaper change).
    last_check: Option<Instant>,
    config_mtime: Option<SystemTime>,
    /// Screen the window was on, and its last position on it (cross-fade when changing screens).
    last: Option<Geometry>,
    previous: Option<Geometry>,
    /// Number of the screen change in progress (identifies its animation).
    switches: u64,
}

impl Backdrop {
    /// Draws the window background (to be called before the panels).
    pub fn paint(&mut self, ctx: &egui::Context, mode: Mode, geometry: Option<&Geometry>) {
        let screen = ctx.screen_rect();
        let radius = CornerRadius::same(theme::window_radius());
        let painter = ctx.layer_painter(egui::LayerId::background());
        let bg = theme::bg();

        if let Mode::Transparent(opacity) = mode {
            let a = (110.0 + 120.0 * opacity.clamp(0.0, 1.0)) as u8;
            painter.rect_filled(screen, radius, theme::alpha(bg, a));
            self.paint_noise(ctx, &painter, screen, radius, 0x04);
            return;
        }
        painter.rect_filled(screen, radius, bg);

        let mica = matches!(mode, Mode::Mica(_));
        if mica {
            self.watch_config();
        }
        if mica && !self.started {
            self.started = true;
            let slot: Slot = Arc::new(Mutex::new(None));
            self.job = Some(slot.clone());
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                *slot.lock().unwrap() = Some(prepare_all());
                ctx.request_repaint();
            });
        }
        let ready = self.job.as_ref().and_then(|job| job.lock().unwrap().take());
        if let Some(prepared) = ready {
            let load = |name: &str, image| {
                ctx.load_texture(format!("mica:{name}"), image, egui::TextureOptions::LINEAR)
            };
            self.by_output = prepared
                .by_output
                .into_iter()
                .map(|(k, v)| (k.clone(), load(&k, v)))
                .collect();
            self.default = prepared.default.map(|i| load("default", i));
            self.job = None;
        }

        // The wallpaper of the screen the window is on, cut at its position; otherwise the default
        // one, covering the window.
        let placed = geometry.and_then(|g| self.by_output.get(&g.output).map(|t| (t, g)));
        let available = placed.is_some() || self.default.is_some();
        let t = anim::toggle(ctx, egui::Id::new("mica_fade"), mica && available, anim::SLOW);
        if t <= 0.0 {
            return;
        }
        let tint = Color32::WHITE.gamma_multiply(t);
        match placed {
            Some((texture, g)) => {
                // Screen change: the old background fades out while the new one appears.
                if self.last.as_ref().is_some_and(|l| l.output != g.output) {
                    self.previous = self.last.clone();
                    self.switches += 1;
                }
                self.last = Some(g.clone());
                let k = anim::value_from(
                    ctx,
                    egui::Id::new(("mica_switch", self.switches)),
                    if self.previous.is_some() { 0.0 } else { 1.0 },
                    1.0,
                    0.7,
                    anim::Easing::Out,
                );
                let old = self.previous.as_ref().filter(|_| k < 1.0);
                match old.and_then(|p| self.by_output.get(&p.output).map(|t| (t, p))) {
                    Some((old_texture, old_g)) => {
                        // The old screen, frozen at the last position, then the new one on top.
                        paint_window_slice(&painter, old_texture, screen, old_g, tint, radius);
                        paint_window_slice(&painter, texture, screen, g, tint.gamma_multiply(k), radius);
                    }
                    None => paint_window_slice(&painter, texture, screen, g, tint, radius),
                }
                if k >= 1.0 {
                    self.previous = None;
                }
            }
            None => match &self.default {
                Some(texture) => paint_fill(&painter, texture, screen, tint, radius),
                None => return,
            },
        }
        let strength = if let Mode::Mica(s) = mode {
            s.clamp(0.0, 1.0)
        } else {
            0.5
        };
        // Theme tint on top: the lower the strength, the more it covers.
        let veil = 0.90 - 0.55 * strength;
        painter.rect_filled(
            screen,
            radius,
            theme::alpha(bg, (255.0 * (1.0 - t * (1.0 - veil))) as u8),
        );
        self.paint_noise(ctx, &painter, screen, radius, (0x06 as f32 * t) as u8);
    }

    /// Starts the preparation again if the desktop configuration changed (new wallpaper).
    fn watch_config(&mut self) {
        if self.last_check.is_some_and(|t| t.elapsed().as_secs() < 4) {
            return;
        }
        self.last_check = Some(Instant::now());
        let mtime = kde_config_path()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok());
        if self.config_mtime.is_some() && mtime != self.config_mtime && self.job.is_none() {
            self.started = false;
        }
        self.config_mtime = mtime;
    }

    /// Very light grain, like the noise of Windows 11 materials.
    fn paint_noise(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        rect: Rect,
        radius: CornerRadius,
        alpha: u8,
    ) {
        if alpha == 0 {
            return;
        }
        let noise = self.noise.get_or_insert_with(|| {
            let size = 128;
            let mut rng = fastrand::Rng::with_seed(7);
            let pixels = (0..size * size).map(|_| Color32::from_gray(rng.u8(..))).collect();
            let options = egui::TextureOptions {
                wrap_mode: egui::TextureWrapMode::Repeat,
                ..egui::TextureOptions::NEAREST
            };
            ctx.load_texture(
                "mica_noise",
                egui::ColorImage {
                    size: [size, size],
                    pixels,
                },
                options,
            )
        });
        let uv = Rect::from_min_max(
            egui::pos2(0.0, 0.0),
            egui::pos2(rect.width() / 128.0, rect.height() / 128.0),
        );
        painter.add(
            // Opacity in sRGB: `from_white_alpha` (linear) would make the grain far too visible.
            egui::epaint::RectShape::filled(rect, radius, theme::white(alpha)).with_texture(noise.id(), uv),
        );
    }
}

/// The part of the wallpaper behind the window. The wallpaper covers the screen without being
/// distorted ("scaled and cropped", Plasma's default setting).
fn paint_window_slice(
    painter: &egui::Painter,
    texture: &egui::TextureHandle,
    screen: Rect,
    g: &Geometry,
    tint: Color32,
    radius: CornerRadius,
) {
    let [tw, th] = texture.size().map(|v| v as f32);
    let o = g.output_rect;
    let scale = (o.width() / tw).max(o.height() / th);
    let (iw, ih) = (tw * scale, th * scale);
    let image = Rect::from_min_size(
        egui::pos2(
            o.left() + (o.width() - iw) / 2.0,
            o.top() + (o.height() - ih) / 2.0,
        ),
        egui::vec2(iw, ih),
    );
    let w = g.window;
    let uv = Rect::from_min_max(
        egui::pos2((w.left() - image.left()) / iw, (w.top() - image.top()) / ih),
        egui::pos2((w.right() - image.left()) / iw, (w.bottom() - image.top()) / ih),
    );
    painter.add(egui::epaint::RectShape::filled(screen, radius, tint).with_texture(texture.id(), uv));
}

/// Draws a texture covering the whole `rect` without distortion (cropped at the center), with rounded corners.
pub fn paint_fill(
    painter: &egui::Painter,
    texture: &egui::TextureHandle,
    rect: Rect,
    tint: Color32,
    radius: CornerRadius,
) {
    let [w, h] = texture.size();
    let (tex_ratio, rect_ratio) = (w as f32 / h.max(1) as f32, rect.width() / rect.height().max(1.0));
    let uv = if tex_ratio > rect_ratio {
        let k = rect_ratio / tex_ratio;
        Rect::from_min_max(egui::pos2((1.0 - k) / 2.0, 0.0), egui::pos2((1.0 + k) / 2.0, 1.0))
    } else {
        let k = tex_ratio / rect_ratio;
        Rect::from_min_max(egui::pos2(0.0, (1.0 - k) / 2.0), egui::pos2(1.0, (1.0 + k) / 2.0))
    };
    painter.add(egui::epaint::RectShape::filled(rect, radius, tint).with_texture(texture.id(), uv));
}

/// Prepares the wallpaper of each screen (and the default one).
fn prepare_all() -> Prepared {
    let (by_screen, default_path) = wallpapers();
    let mut prepared = Prepared::default();
    let mut cache: HashMap<PathBuf, egui::ColorImage> = HashMap::new();
    for (output, path) in by_screen {
        let image = match cache.get(&path) {
            Some(i) => Some(i.clone()),
            None => prepare(&path),
        };
        if let Some(image) = image {
            cache.insert(path, image.clone());
            prepared.by_output.insert(output, image);
        }
    }
    prepared.default = default_path.and_then(|p| cache.get(&p).cloned().or_else(|| prepare(&p)));
    prepared
}

/// The wallpaper, scaled down, heavily blurred, a little more saturated and darkened (the theme
/// tint is added when drawing, which allows changing theme or strength without recomputing).
fn prepare(path: &Path) -> Option<egui::ColorImage> {
    let wall = image::open(path).ok()?;
    // A window only shows part of the screen: keep a little more detail.
    let small = wall
        .resize(360, 360, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let blurred = image::imageops::blur(&image::imageops::blur(&small, 10.0), 10.0);
    let pixels = blurred
        .pixels()
        .map(|p| {
            let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
            let l = 0.299 * r + 0.587 * g + 0.114 * b;
            // Saturation ×1.3, then brightness brought into a dark range.
            let sat = |c: f32| (l + (c - l) * 1.3).clamp(0.0, 255.0) * 0.62;
            Color32::from_rgb(sat(r) as u8, sat(g) as u8, sat(b) as u8)
        })
        .collect();
    Some(egui::ColorImage {
        size: [blurred.width() as usize, blurred.height() as usize],
        pixels,
    })
}

/// Wallpapers per screen (KDE), and the one to use as a fallback (main screen, or GNOME).
fn wallpapers() -> (HashMap<String, PathBuf>, Option<PathBuf>) {
    let by_screen = kde_wallpapers();
    let outputs = kde_outputs();
    let mut by_output = HashMap::new();
    for (screen, path) in &by_screen {
        if let Some(name) = outputs.get(screen) {
            by_output.insert(name.clone(), path.clone());
        }
    }
    let default = by_screen
        .get(&0)
        .or_else(|| by_screen.values().next())
        .cloned()
        .or_else(gnome_wallpaper);
    (by_output, default)
}

fn kde_config_path() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    Some(config.join("plasma-org.kde.plasma.desktop-appletsrc"))
}

/// Plasma screen number → output name. Plasma numbers screens by priority
/// (`kscreen-doctor`: priority 1 = screen 0).
fn kde_outputs() -> HashMap<u32, String> {
    let Ok(out) = std::process::Command::new("kscreen-doctor").arg("-j").output() else {
        return HashMap::new();
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return HashMap::new();
    };
    json["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["enabled"].as_bool().unwrap_or(false))
        .filter_map(|o| {
            let priority = o["priority"].as_u64()? as u32;
            Some((priority.saturating_sub(1), o["name"].as_str()?.to_owned()))
        })
        .collect()
}

#[derive(Default)]
struct Containment {
    screen: Option<u32>,
    desktop: bool,
    plugin: String,
    image: Option<String>,
    video: Option<String>,
    video_pos_ms: u64,
}

/// KDE Plasma: wallpaper of each desktop, by screen number. "image" plugin (file or wallpaper
/// package) or video wallpaper (a frame is extracted from it).
fn kde_wallpapers() -> HashMap<u32, PathBuf> {
    let Some(text) = kde_config_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return HashMap::new();
    };
    let mut containments: HashMap<String, Containment> = HashMap::new();
    let mut group = String::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            group = line.to_owned();
            continue;
        }
        let Some(id) = group
            .strip_prefix("[Containments][")
            .and_then(|g| g.split(']').next())
        else {
            continue;
        };
        let top = group.matches('[').count() == 2;
        let c = containments.entry(id.to_owned()).or_default();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "lastScreen" if top => c.screen = value.parse().ok(),
            "formfactor" if top => c.desktop = value == "0",
            "wallpaperplugin" if top => c.plugin = value.to_owned(),
            "Image" if group.contains("[Wallpaper][org.kde.image]") => c.image = Some(value.to_owned()),
            "VideoUrls" if group.contains("smart.video.wallpaper") => {
                // [{"filename":"file:///…mp4","enabled":true,…}]
                c.video = serde_json::from_str::<serde_json::Value>(value)
                    .ok()
                    .and_then(|v| {
                        v.as_array()?
                            .iter()
                            .find(|e| e["enabled"].as_bool().unwrap_or(true))?["filename"]
                            .as_str()
                            .map(str::to_owned)
                    });
            }
            "VideoUrl" if group.contains("[Wallpaper]") && c.video.is_none() => {
                c.video = Some(value.to_owned())
            }
            "LastVideoPosition" => c.video_pos_ms = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    let mut out = HashMap::new();
    for c in containments.values().filter(|c| c.desktop) {
        let Some(screen) = c.screen else { continue };
        let video = c.video.as_deref().filter(|_| c.plugin != "org.kde.image");
        let path = match video {
            Some(v) => video_frame(&percent_decode(v.trim_start_matches("file://")), c.video_pos_ms),
            None => None,
        }
        .or_else(|| {
            c.image
                .as_deref()
                .and_then(|v| resolve(&percent_decode(v.trim().trim_start_matches("file://"))))
        });
        if let Some(path) = path {
            out.insert(screen, path);
        }
    }
    out
}

/// A frame of the video wallpaper (where the video was), extracted with ffmpeg, cached.
fn video_frame(video: &str, position_ms: u64) -> Option<PathBuf> {
    let video = Path::new(video);
    let modified = std::fs::metadata(video).and_then(|m| m.modified()).ok()?;
    let dir = crate::library::cache_dir().join("wallpapers");
    std::fs::create_dir_all(&dir).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&video, &mut h);
    let out = dir.join(format!("{:016x}.jpg", std::hash::Hasher::finish(&h)));
    let fresh = std::fs::metadata(&out)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t >= modified);
    if fresh {
        return Some(out);
    }
    for seconds in [position_ms as f64 / 1000.0, 1.0, 0.0] {
        let ok = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-ss", &format!("{seconds:.2}"), "-i"])
            .arg(video)
            .args(["-frames:v", "1", "-vf", "scale=640:-2"])
            .arg(&out)
            .status()
            .is_ok_and(|s| s.success());
        if ok && std::fs::metadata(&out).is_ok_and(|m| m.len() > 0) {
            return Some(out);
        }
    }
    None
}

/// GNOME (Ubuntu, Fedora…): `gsettings`, dark variant first.
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

/// An image file, or a KDE wallpaper package (`contents/images/1920x1080.jpg`…): the largest
/// image is then taken.
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
