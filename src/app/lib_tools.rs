//! Library maintenance tools, available from the tag editor and the settings: albums without
//! art, ReplayGain analysis, duplicates. Each opens in its own window; the long work runs in
//! the background and goes on if the window is closed.

use std::path::PathBuf;

use eframe::egui::{self, vec2, RichText, Sense};

use super::cover_tool::CoverTool;
use super::dup_tool::DupTool;
use super::rg_tool::RgTool;
use super::SterMusic;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::{theme, widgets};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LibTool {
    Covers,
    ReplayGain,
    Duplicates,
}

impl LibTool {
    pub const ALL: [LibTool; 3] = [LibTool::Covers, LibTool::ReplayGain, LibTool::Duplicates];

    pub fn icon(self) -> Icon {
        match self {
            LibTool::Covers => Icon::Images,
            LibTool::ReplayGain => Icon::Volume2,
            LibTool::Duplicates => Icon::WinRestore,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LibTool::Covers => tr("Albums sans pochette"),
            LibTool::ReplayGain => tr("ReplayGain"),
            LibTool::Duplicates => tr("Doublons"),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            LibTool::Covers => tr("Repère les albums sans pochette et va chercher leurs images en ligne"),
            LibTool::ReplayGain => {
                tr("Mesure le volume des morceaux et des albums, pour un nivellement exact")
            }
            LibTool::Duplicates => tr("Repère les morceaux présents en plusieurs exemplaires"),
        }
    }
}

#[derive(Default)]
pub struct LibTools {
    /// Open window.
    pub(super) open: Option<LibTool>,
    pub(super) covers: CoverTool,
    pub(super) rg: RgTool,
    pub(super) dups: DupTool,
}

impl LibTools {
    /// A job is running in the background (sidebar indicator).
    pub fn busy(&self) -> bool {
        self.rg.running() || self.covers.busy() || self.dups.busy()
    }

    /// A tool is writing tags: the editor waits its turn.
    pub fn writing(&self) -> bool {
        self.rg.running()
    }

    /// What is running, for the sidebar tooltip.
    pub fn status(&self) -> Option<&'static str> {
        if self.rg.running() {
            Some(tr("Analyse ReplayGain en cours…"))
        } else if self.covers.busy() {
            Some(tr("Recherche de pochettes en cours…"))
        } else if self.dups.busy() {
            Some(tr("Recherche de doublons en cours…"))
        } else {
            None
        }
    }

    /// On exit: nothing must start writing to a file any more.
    pub fn stop(&mut self) {
        self.rg.stop_and_wait();
        self.covers.stop();
    }
}

/// The three tools, as menu items; returns the one that was picked.
pub(super) fn menu(ui: &mut egui::Ui) -> Option<LibTool> {
    let mut chosen = None;
    ui.set_min_width(250.0);
    for tool in LibTool::ALL {
        if widgets::menu_item(ui, Some(tool.icon()), &format!("{}…", tool.label()))
            .on_hover_text(tool.hint())
            .clicked()
        {
            chosen = Some(tool);
        }
    }
    chosen
}

pub(super) fn modal_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::popup(&ctx.style())
        .inner_margin(24.0)
        .corner_radius(12)
}

/// Title of a tool window: icon, name and a discreet detail.
pub(super) fn title(ui: &mut egui::Ui, icon: Icon, text: &str, detail: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
        icons::paint_centered(ui.painter(), icon, r.center(), 20.0, theme::accent_light());
        ui.label(RichText::new(text).font(theme::semibold(20.0)));
        if !detail.is_empty() {
            ui.label(RichText::new(detail).color(theme::TEXT_3));
        }
    });
}

impl SterMusic {
    pub(super) fn open_lib_tool(&mut self, tool: LibTool, ctx: &egui::Context) {
        if self.lib.is_empty() && tool != LibTool::ReplayGain {
            self.toast(tr(
                "La bibliothèque est vide : ajoute d'abord un dossier de musique",
            ));
            return;
        }
        match tool {
            LibTool::Covers => self.start_cover_scan(ctx),
            LibTool::Duplicates => self.start_dup_scan(ctx),
            LibTool::ReplayGain => self.lib_tools.rg.opened(None),
        }
        self.lib_tools.open = Some(tool);
    }

    /// ReplayGain of a few specific tracks (menu of an album, of a selection).
    pub(super) fn open_replaygain_for(&mut self, paths: Vec<PathBuf>, label: String) {
        let paths: Vec<PathBuf> = paths.into_iter().filter(|p| p.is_file()).collect();
        if paths.is_empty() {
            return;
        }
        self.lib_tools.rg.opened(Some((paths, label)));
        self.lib_tools.open = Some(LibTool::ReplayGain);
    }

    pub(super) fn poll_lib_tools(&mut self, ctx: &egui::Context) {
        self.poll_replaygain();
        self.lib_tools.covers.poll();
        self.poll_duplicates(ctx);
        if self.lib_tools.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }

    pub(super) fn lib_tool_dialogs(&mut self, ctx: &egui::Context) {
        match self.lib_tools.open {
            Some(LibTool::Covers) => self.cover_dialog(ctx),
            Some(LibTool::ReplayGain) => self.replaygain_dialog(ctx),
            Some(LibTool::Duplicates) => self.duplicates_dialog(ctx),
            None => {}
        }
    }
}
