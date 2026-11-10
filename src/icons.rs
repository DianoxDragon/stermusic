//! Icônes, teintées à l'affichage : celles du thème Breeze de KDE quand le style Breeze est
//! actif (cohérence avec Kubuntu), sinon les icônes Lucide intégrées (licence ISC), comme dans
//! SterCode. Une icône absente du système retombe sur Lucide.
//!
//! Chaque icône est rendue à la taille exacte où elle s'affiche, alignée sur les pixels :
//! le cache de textures d'egui ne tient pas compte de la taille demandée, d'où la taille
//! glissée dans l'URI.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, ImageSource, Rect};

macro_rules! icons {
    ($($name:ident => $file:literal),* $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Icon { $($name),* }

        impl Icon {
            fn file(self) -> (&'static str, &'static [u8]) {
                match self {
                    $(Icon::$name => ($file, include_bytes!(concat!("../assets/icons/", $file)))),*
                }
            }
        }
    };
}

icons! {
    Play => "play-filled.svg",
    Pause => "pause-filled.svg",
    PlayOutline => "play.svg",
    SkipBack => "skip-back.svg",
    SkipForward => "skip-forward.svg",
    Shuffle => "shuffle.svg",
    Repeat => "repeat.svg",
    Repeat1 => "repeat-1.svg",
    Volume0 => "volume.svg",
    Volume1 => "volume-1.svg",
    Volume2 => "volume-2.svg",
    VolumeX => "volume-x.svg",
    Music => "music.svg",
    Disc => "disc-3.svg",
    Mic => "mic-vocal.svg",
    ListMusic => "list-music.svg",
    Home => "house.svg",
    Library => "library.svg",
    Heart => "heart.svg",
    HeartFilled => "heart-filled.svg",
    Maximize => "maximize-2.svg",
    ListPlus => "list-plus.svg",
    ListEnd => "list-end.svg",
    ListStart => "list-start.svg",
    Menu => "menu.svg",
    ArrowLeft => "arrow-left.svg",
    Search => "search.svg",
    Settings => "settings.svg",
    Close => "x.svg",
    WinMinimize => "minus.svg",
    WinMaximize => "square.svg",
    WinRestore => "copy.svg",
    FolderOpen => "folder-open.svg",
    FolderPlus => "folder-plus.svg",
    Plus => "plus.svg",
    Refresh => "refresh-cw.svg",
    Trash => "trash-2.svg",
    ChevronRight => "chevron-right.svg",
    Clock => "clock.svg",
    Pencil => "pencil.svg",
    Palette => "palette.svg",
    Info => "info.svg",
    MiniPlayer => "picture-in-picture-2.svg",
    Sparkles => "sparkles.svg",
    Grip => "grip-vertical.svg",
    Download => "download.svg",
    Upload => "upload.svg",
    AudioLines => "audio-lines.svg",
    Equalizer => "sliders-vertical.svg",
    Tag => "tag.svg",
    Image => "image.svg",
    Undo => "undo-2.svg",
    Globe => "globe.svg",
    Save => "save.svg",
    Replace => "replace.svg",
    FilePen => "file-pen.svg",
    Stop => "square-stop.svg",
    Images => "images.svg",
    Help => "circle-help.svg",
    Chart => "chart-column.svg",
    Flame => "flame.svg",
    History => "history.svg",
    CalendarPlus => "calendar-plus.svg",
    Star => "star.svg",
    Zap => "zap.svg",
    Moon => "moon.svg",
    Sun => "sun.svg",
    Coffee => "coffee.svg",
    EyeOff => "eye-off.svg",
    Radio => "radio.svg",
    RadioTower => "radio-tower.svg",
    Speaker => "speaker.svg",
    Timer => "timer.svg",
    Bell => "bell.svg",
    Calendar => "calendar.svg",
    Feather => "feather.svg",
    Shapes => "shapes.svg",
    Link => "link.svg",
    MediaStop => "stop-filled.svg",
}

/// Rectangle aligné sur la grille des pixels physiques.
fn snap(ctx: &egui::Context, rect: Rect) -> (Rect, u32) {
    let ppp = ctx.pixels_per_point();
    let px = (rect.width() * ppp).round().max(1.0);
    let min = ((rect.center().to_vec2() * ppp - egui::Vec2::splat(px / 2.0)).round() / ppp).to_pos2();
    (Rect::from_min_size(min, egui::Vec2::splat(px / ppp)), px as u32)
}

/// Nom de l'icône équivalente dans le thème Breeze (freedesktop).
fn breeze_name(icon: Icon) -> Option<&'static str> {
    use Icon::*;
    Some(match icon {
        Play | PlayOutline => "media-playback-start",
        Pause => "media-playback-pause",
        SkipBack => "media-skip-backward",
        SkipForward => "media-skip-forward",
        Shuffle => "media-playlist-shuffle",
        Repeat => "media-playlist-repeat",
        Repeat1 => "media-playlist-repeat-song",
        Volume0 => "audio-volume-low",
        Volume1 => "audio-volume-medium",
        Volume2 => "audio-volume-high",
        VolumeX => "audio-volume-muted",
        Disc => "media-optical-audio",
        Mic => "view-media-artist",
        ListMusic => "view-media-playlist",
        Home => "go-home",
        Library => "library-music",
        Heart => "love",
        Maximize => "view-fullscreen",
        ListPlus => "media-playlist-append",
        ListEnd => "media-playlist-append",
        ListStart => "go-next",
        Menu => "application-menu",
        ArrowLeft => "go-previous",
        Search => "edit-find",
        Settings => "configure",
        Close => "window-close",
        FolderOpen => "folder-open",
        FolderPlus => "folder-new",
        Plus => "list-add",
        Refresh => "view-refresh",
        Trash => "edit-delete",
        ChevronRight => "arrow-right",
        Clock => "chronometer",
        Pencil => "document-edit",
        Palette => "color-management",
        Info => "help-about",
        MiniPlayer => "window-new",
        Download => "document-import",
        Upload => "document-export",
        AudioLines => "view-media-visualization",
        Equalizer => "view-media-equalizer",
        Tag => "tag",
        Image => "insert-image",
        Undo => "edit-undo",
        Globe => "globe",
        Save => "document-save",
        Replace => "edit-find-replace",
        FilePen => "edit-rename",
        Stop => "process-stop",
        Images => "view-preview",
        Help => "help-contextual",
        Chart => "view-statistics",
        Radio => "radio",
        Speaker => "audio-speakers-symbolic",
        Timer => "chronometer",
        Bell => "notifications",
        Calendar => "view-calendar",
        MediaStop => "media-playback-stop",
        _ => return None,
    })
}

type Cache = Mutex<HashMap<(&'static str, u32), Option<Arc<[u8]>>>>;
static SYSTEM: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();

/// SVG de l'icône Breeze (sombre), recoloré en blanc pour être teinté ; `None` si absent.
fn system_svg(name: &'static str, px: u32) -> Option<Arc<[u8]>> {
    let cache = SYSTEM.get_or_init(Default::default);
    let size = if px <= 16 { 16 } else { 22 };
    let mut map = cache.lock().unwrap();
    map.entry((name, size))
        .or_insert_with(|| {
            let mut dirs: Vec<PathBuf> = Vec::new();
            if let Some(home) = std::env::var_os("HOME") {
                dirs.push(PathBuf::from(home).join(".local/share/icons/breeze-dark"));
            }
            dirs.push("/usr/share/icons/breeze-dark".into());
            let file = dirs.iter().find_map(|d| {
                ["actions", "status", "devices", "places", "apps"]
                    .iter()
                    .flat_map(|cat| {
                        [size, 22, 24, 16]
                            .map(|s| d.join(cat).join(s.to_string()).join(format!("{name}.svg")))
                    })
                    .find(|p| p.is_file())
            })?;
            let text = std::fs::read_to_string(file).ok()?;
            // Toutes les couleurs du jeu Breeze → blanc : la teinte est appliquée à l'affichage.
            let text = text
                .replace("currentColor", "#ffffff")
                .replace("#fcfcfc", "#ffffff")
                .replace("#232629", "#ffffff")
                .replace("stroke:none", "stroke:#ffffff");
            Some(Arc::from(embolden(&text).into_bytes()))
        })
        .clone()
}

/// Épaissit une icône Breeze : ses formes pleines n'ont que des traits d'un pixel, peu lisibles
/// sur fond sombre. Un fin contour de la même couleur les grossit uniformément.
fn embolden(svg: &str) -> String {
    const STYLE: &str = "<style type=\"text/css\">path, rect, circle, ellipse, polygon { stroke: #ffffff; stroke-width: 0.7; stroke-linejoin: round; }</style>";
    match svg
        .find("<svg")
        .and_then(|start| svg[start..].find('>').map(|end| start + end + 1))
    {
        Some(at) => format!("{}{STYLE}{}", &svg[..at], &svg[at..]),
        None => svg.to_owned(),
    }
}

fn image(icon: Icon, px: u32) -> egui::Image<'static> {
    if crate::theme::breeze() {
        if let Some(name) = breeze_name(icon) {
            if let Some(bytes) = system_svg(name, px) {
                return egui::Image::new(ImageSource::Bytes {
                    uri: format!("bytes://breeze/{px}/{name}.svg").into(),
                    bytes: egui::load::Bytes::Shared(bytes),
                });
            }
        }
    }
    lucide(icon, px)
}

fn lucide(icon: Icon, px: u32) -> egui::Image<'static> {
    let (file, bytes) = icon.file();
    egui::Image::new(ImageSource::Bytes {
        uri: format!("bytes://icons/{px}/{file}").into(),
        bytes: bytes.into(),
    })
}

/// Comme [`paint_centered`], mais toujours avec l'icône Lucide (même en style Breeze).
pub fn paint_lucide(painter: &egui::Painter, icon: Icon, center: egui::Pos2, size: f32, color: Color32) {
    let (rect, px) = snap(
        painter.ctx(),
        Rect::from_center_size(center, egui::Vec2::splat(size)),
    );
    let image = lucide(icon, px).fit_to_exact_size(rect.size());
    if let Ok(egui::load::TexturePoll::Ready { texture }) = image.load_for_size(painter.ctx(), rect.size()) {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(texture.id, rect, uv, color);
    }
}

/// Vrai si l'icône sera dessinée depuis le thème Breeze du système.
fn uses_breeze(icon: Icon) -> bool {
    crate::theme::breeze() && breeze_name(icon).is_some_and(|n| system_svg(n, 22).is_some())
}

/// Dessine l'icône dans `rect`, teintée de `color`.
pub fn paint(painter: &egui::Painter, icon: Icon, rect: Rect, color: Color32) {
    // Les icônes Breeze sont dessinées au pixel près pour 16 et 22 px : ailleurs, leurs traits
    // d'un pixel deviennent flous et pâles. On les cale sur la taille d'origine la plus proche.
    let rect = if uses_breeze(icon) && (12.0..=27.0).contains(&rect.width()) {
        let size = if rect.width() >= 19.0 { 22.0 } else { 16.0 };
        Rect::from_center_size(rect.center(), egui::Vec2::splat(size))
    } else {
        rect
    };
    let (rect, px) = snap(painter.ctx(), rect);
    let image = image(icon, px).fit_to_exact_size(rect.size());
    if let Ok(egui::load::TexturePoll::Ready { texture }) = image.load_for_size(painter.ctx(), rect.size()) {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(texture.id, rect, uv, color);
    }
}

/// Dessine l'icône centrée sur `center`, de côté `size`.
pub fn paint_centered(painter: &egui::Painter, icon: Icon, center: egui::Pos2, size: f32, color: Color32) {
    paint(
        painter,
        icon,
        Rect::from_center_size(center, egui::Vec2::splat(size)),
        color,
    );
}
