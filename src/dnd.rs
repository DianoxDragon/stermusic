//! File drag and drop under Wayland.
//!
//! winit (0.30) only passes dropped files on under X11: under Wayland, nothing reaches egui.
//! And KWin only sends drag events to the application's first `wl_data_device`, which is the
//! one of egui's clipboard (created before the SterMusic window): so it is this clipboard, in a
//! modified copy (`vendor/smithay-clipboard`), that accepts the files. Here we fetch what it
//! received and slip it into egui's input ([`Dnd::feed`]): the rest of the application cannot
//! tell the difference from X11.

use std::path::PathBuf;

use eframe::egui;

pub struct Dnd;

impl Dnd {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        smithay_clipboard::dnd::set_waker(move || ctx.request_repaint());
        Self
    }

    /// Adds the hovered or dropped files to egui's input.
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

/// "file:///home/me/My%20music/a.mp3" → path; other addresses are ignored.
fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        // "file://host/path": only the path matters (the host is almost always empty).
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
