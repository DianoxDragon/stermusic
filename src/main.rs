mod anim;
mod app;
mod backdrop;
mod cast;
mod compat;
mod convert;
mod covers;
mod dnd;
mod dsp;
mod ffdecode;
mod globe;
mod history;
mod i18n;
mod i18n_en;
#[cfg(test)]
mod i18n_keys;
mod icons;
mod ipc;
mod kwin;
mod library;
mod loudness;
mod lyrics;
mod media;
mod mpris;
mod notify;
mod outputs;
mod musicbrainz;
mod player;
mod podcasts;
mod radio;
mod scrobble;
mod servers;
mod shader;
mod tags;
mod tagtools;
mod theme;
mod tray;
mod viz;
mod watch;
mod widgets;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // `stermusic ~/Musique` ajoute ce dossier à la bibliothèque ; `stermusic morceau.mp3…` joue
    // ces morceaux (double-clic dans Dolphin) ; `stermusic --tags fichiers…` ouvre l'éditeur de
    // tags. S'il y a déjà une fenêtre SterMusic, c'est elle qui s'en charge.
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let absolute = |p: &PathBuf| std::path::absolute(p).unwrap_or_else(|_| p.clone());
    let mut launch = app::Launch::default();
    if args.first().and_then(|a| a.to_str()) == Some("--tags") {
        launch.tag_files = args[1..].iter().map(absolute).collect();
        if ipc::send_tags(&launch.tag_files) {
            return Ok(());
        }
    } else {
        for p in args.iter().map(absolute) {
            if p.is_dir() {
                launch.music_dir.get_or_insert(p);
            } else if library::is_audio_file(&p) {
                launch.play_files.push(p);
            }
        }
        if !launch.play_files.is_empty() && ipc::send_play(&launch.play_files) {
            return Ok(());
        }
        // Relancé depuis le menu alors qu'il tourne déjà : on montre la fenêtre ouverte.
        if args.is_empty() && ipc::send_show() {
            return Ok(());
        }
    }

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
        // Pas de synchronisation verticale : sous Wayland, une fenêtre réduite (ou cachée dans
        // la zone de notification) ne reçoit plus d'images du compositeur, et la présentation
        // d'une image bloquerait toute l'application, lecture comprise (plus de morceau
        // suivant). La cadence est tenue par `app::pace` à la fréquence de l'écran.
        vsync: false,
        ..Default::default()
    };

    eframe::run_native(
        "SterMusic",
        options,
        Box::new(move |cc| Ok(Box::new(app::SterMusic::new(cc, launch)))),
    )
}
