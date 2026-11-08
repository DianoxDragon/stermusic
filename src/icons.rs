//! Icônes Lucide (licence ISC), comme dans SterCode, teintées à l'affichage.
//!
//! Chaque icône est rendue à la taille exacte où elle s'affiche, alignée sur les pixels :
//! le cache de textures d'egui ne tient pas compte de la taille demandée, d'où la taille
//! glissée dans l'URI.

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
    Minimize2 => "minimize-2.svg",
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
}

/// Rectangle aligné sur la grille des pixels physiques.
fn snap(ctx: &egui::Context, rect: Rect) -> (Rect, u32) {
    let ppp = ctx.pixels_per_point();
    let px = (rect.width() * ppp).round().max(1.0);
    let min =
        ((rect.center().to_vec2() * ppp - egui::Vec2::splat(px / 2.0)).round() / ppp).to_pos2();
    (
        Rect::from_min_size(min, egui::Vec2::splat(px / ppp)),
        px as u32,
    )
}

fn image(icon: Icon, px: u32) -> egui::Image<'static> {
    let (file, bytes) = icon.file();
    egui::Image::new(ImageSource::Bytes {
        uri: format!("bytes://icons/{px}/{file}").into(),
        bytes: bytes.into(),
    })
}

/// Dessine l'icône dans `rect`, teintée de `color`.
pub fn paint(painter: &egui::Painter, icon: Icon, rect: Rect, color: Color32) {
    let (rect, px) = snap(painter.ctx(), rect);
    let image = image(icon, px).fit_to_exact_size(rect.size());
    if let Ok(egui::load::TexturePoll::Ready { texture }) =
        image.load_for_size(painter.ctx(), rect.size())
    {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(texture.id, rect, uv, color);
    }
}

/// Dessine l'icône centrée sur `center`, de côté `size`.
pub fn paint_centered(
    painter: &egui::Painter,
    icon: Icon,
    center: egui::Pos2,
    size: f32,
    color: Color32,
) {
    paint(
        painter,
        icon,
        Rect::from_center_size(center, egui::Vec2::splat(size)),
        color,
    );
}
