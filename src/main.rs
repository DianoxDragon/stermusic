mod anim;
mod app;
mod backdrop;
mod covers;
mod dnd;
mod dsp;
mod icons;
mod ipc;
mod kwin;
mod library;
mod mpris;
mod musicbrainz;
mod player;
mod shader;
mod tags;
mod tagtools;
mod theme;
mod viz;
mod widgets;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // `stermusic ~/Musique` ajoute ce dossier à la bibliothèque ;
    // `stermusic --tags fichiers…` ouvre l'éditeur de tags (dans la fenêtre déjà ouverte s'il y en a une).
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let (cli_dir, tag_files) = match args.first().and_then(|a| a.to_str()) {
        Some("--tags") => {
            let files: Vec<PathBuf> = args[1..]
                .iter()
                .map(|p| std::path::absolute(p).unwrap_or_else(|_| p.clone()))
                .collect();
            if ipc::send_to_running(&files) {
                return Ok(());
            }
            (None, files)
        }
        _ => (args.first().cloned(), Vec::new()),
    };

    // Le son passe par le greffon ALSA de PipeWire, qui ne nomme pas le flux et n'indique pas le
    // processus : Plasma mettait alors plusieurs secondes à relier le son à la fenêtre (icône de
    // haut-parleur de la barre des tâches). On lui donne ces informations (avant tout thread).
    if std::env::var_os("PIPEWIRE_PROPS").is_none() {
        std::env::set_var(
            "PIPEWIRE_PROPS",
            format!(
                "{{ application.name=SterMusic application.id=stermusic application.icon-name=stermusic \
                 media.role=Music application.process.id={} }}",
                std::process::id()
            ),
        );
    }

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("SterMusic")
        .with_app_id("stermusic")
        .with_inner_size([1240.0, 800.0])
        .with_min_inner_size([760.0, 520.0])
        // Barre de titre dessinée par l'application, fondue dans l'interface.
        .with_decorations(false)
        .with_transparent(true)
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
        Box::new(move |cc| Ok(Box::new(app::SterMusic::new(cc, cli_dir, tag_files)))),
    )
}
