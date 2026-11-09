//! Glisser-déposer de fichiers sous Wayland.
//!
//! winit (0.30) ne transmet les fichiers déposés que sous X11 : sous Wayland, rien n'arrive
//! jusqu'à egui. Et KWin n'envoie les événements de glisser qu'au premier `wl_data_device` de
//! l'application, qui est celui du presse-papiers d'egui (créé avant la fenêtre de SterMusic) :
//! c'est donc ce presse-papiers, dans une copie modifiée (`vendor/smithay-clipboard`), qui
//! accepte les fichiers. On vient chercher ici ce qu'il a reçu, et on le glisse dans les entrées
//! d'egui ([`Dnd::feed`]) : le reste de l'application ne voit pas la différence avec X11.

use std::path::PathBuf;

use eframe::egui;

pub struct Dnd;

impl Dnd {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        smithay_clipboard::dnd::set_waker(move || ctx.request_repaint());
        Self
    }

    /// Ajoute aux entrées d'egui les fichiers survolés ou déposés.
    pub fn feed(&self, raw: &mut egui::RawInput) {
        if smithay_clipboard::dnd::hovering() && raw.hovered_files.is_empty() {
            raw.hovered_files.push(egui::HoveredFile::default());
        }
        for list in smithay_clipboard::dnd::take_dropped() {
            for path in parse_uri_list(&list) {
                raw.dropped_files.push(egui::DroppedFile {
                    path: Some(path),
                    ..Default::default()
                });
            }
        }
    }
}

/// « file:///home/moi/Ma%20musique/a.mp3 » → chemin ; les autres adresses sont ignorées.
fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        // « file://hôte/chemin » : seul le chemin compte (l'hôte est presque toujours vide).
        .filter_map(|rest| rest.find('/').map(|i| &rest[i..]))
        .map(|path| {
            let bytes = path.as_bytes();
            let mut out = Vec::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                let hex = (bytes[i] == b'%')
                    .then(|| bytes.get(i + 1..i + 3))
                    .flatten()
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(
                out,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_lists() {
        let text = "# commentaire\r\nfile:///home/moi/Ma%20musique/%C3%A9t%C3%A9.m4a\r\n\
                    file://localhost/tmp/a%25b.flac\r\nhttps://exemple.fr/x.mp3\r\n";
        assert_eq!(
            parse_uri_list(text),
            vec![
                PathBuf::from("/home/moi/Ma musique/été.m4a"),
                PathBuf::from("/tmp/a%b.flac"),
            ]
        );
    }
}
