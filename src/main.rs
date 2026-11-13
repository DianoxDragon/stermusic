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
    // `stermusic ~/Music` adds this folder to the library; `stermusic track.mp3…` plays these
    // tracks (double click in Dolphin); `stermusic --tags files…` opens the tag editor. If a
    // SterMusic window is already open, that window takes care of it.
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
        // Launched again from the menu while it is already running: show the open window.
        if args.is_empty() && ipc::send_show() {
            return Ok(());
        }
    }

    // Sound goes through PipeWire's ALSA plugin, which neither names the stream nor reports the
    // process: Plasma then took several seconds to link the sound to the window (speaker icon in
    // the task bar). We give it this information (before any thread).
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
        // Title bar drawn by the application, blended into the interface.
        .with_decorations(false)
        .with_transparent(true)
        .with_drag_and_drop(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        // No vertical sync: under Wayland, a minimized window (or one hidden in the system tray) no
        // longer receives frames from the compositor, and presenting a frame would block the whole
        // application, playback included (no more next track). Pacing is handled by `app::pace` at
        // the screen's refresh rate.
        vsync: false,
        ..Default::default()
    };

    eframe::run_native(
        "SterMusic",
        options,
        Box::new(move |cc| Ok(Box::new(app::SterMusic::new(cc, launch)))),
    )
}
