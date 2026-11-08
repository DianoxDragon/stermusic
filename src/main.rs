mod anim;
mod app;
mod backdrop;
mod covers;
mod icons;
mod library;
mod mpris;
mod player;
mod theme;
mod widgets;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // `stermusic ~/Musique` ajoute ce dossier à la bibliothèque.
    let cli_dir = std::env::args_os().nth(1).map(PathBuf::from);

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("SterMusic")
        .with_app_id("stermusic")
        .with_inner_size([1240.0, 800.0])
        .with_min_inner_size([760.0, 520.0])
        // Barre de titre dessinée par l'application, fondue dans l'interface.
        .with_decorations(false)
        .with_transparent(false)
        .with_drag_and_drop(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "SterMusic",
        options,
        Box::new(move |cc| Ok(Box::new(app::SterMusic::new(cc, cli_dir)))),
    )
}
