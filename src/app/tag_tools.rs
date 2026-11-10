//! Outils de l'éditeur de tags qui s'ouvrent dans une fenêtre : rechercher-remplacer (avec les
//! expressions régulières et leur aide-mémoire), renommer les fichiers d'après les tags, et lire
//! les tags dans les noms de fichiers.
//!
//! Chaque outil montre un aperçu de ce qu'il va faire avant « Appliquer ». Seul le renommage
//! touche aux fichiers tout de suite ; les deux autres remplissent les champs de l'éditeur, et
//! rien n'est écrit avant « Enregistrer ».

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{self, pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, RichText, Sense};

use super::tag_editor::{file_name, Prefs, TagEditor};
use super::views::segment;
use super::SterMusic;
use crate::i18n::{n_, tr};
use crate::icons::{self, Icon};
use crate::library::{self, plural};
use crate::tags::Field;
use crate::tagtools::{self, NameParser, Replacer, PLACEHOLDERS};
use crate::trf;
use crate::{theme, widgets};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolKind {
    Replace,
    Rename,
    FromName,
}

/// Champs proposés au rechercher-remplacer (les champs numériques n'ont rien à y gagner).
const TEXT_FIELDS: [Field; 7] = [
    Field::Title,
    Field::Artist,
    Field::Album,
    Field::AlbumArtist,
    Field::Genre,
    Field::Composer,
    Field::Comment,
];

/// Motifs tout prêts, dans les deux sens.
const PATTERNS: [&str; 5] = [
    "%track% - %title%",
    "%track%. %title%",
    "%artist% - %title%",
    "%track% - %artist% - %title%",
    "%disc%-%track% - %title%",
];

/// Aide-mémoire des expressions régulières.
const REGEX_HELP: [(&str, &str); 12] = [
    (".", n_("n'importe quel caractère")),
    ("\\d   \\w   \\s", n_("chiffre · lettre ou chiffre · espace")),
    ("[abc]   [^abc]", n_("un de ces caractères · tout sauf eux")),
    ("[a-z]   [0-9]", n_("de « a » à « z » · de « 0 » à « 9 »")),
    ("a|b", n_("« a » ou « b »")),
    ("*   +   ?", n_("0 fois ou plus · 1 ou plus · 0 ou 1")),
    ("{2}   {2,}", n_("exactement 2 fois · 2 fois ou plus")),
    ("^   $", n_("début du texte · fin du texte")),
    ("( … )", n_("groupe, à reprendre avec $1, $2…")),
    ("\\.   \\(   \\[", n_("un vrai point, parenthèse, crochet")),
    (".*?", n_("le moins de caractères possible")),
    ("(?i)", n_("en tête : ignore les majuscules")),
];

/// Exemples prêts à l'emploi : (ce que ça fait, rechercher, remplacer par).
const REGEX_EXAMPLES: [(&str, &str, &str); 6] = [
    (
        n_("Retirer « (feat. …) »"),
        r"\s*[\(\[](feat|ft)\.?\s[^\)\]]*[\)\]]",
        "",
    ),
    (
        n_("Retirer « (Remastered 2011) »"),
        r"\s*[\(\[][^\)\]]*remaster[^\)\]]*[\)\]]",
        "",
    ),
    (n_("Retirer le numéro en tête"), r"^\d+\s*[-.)]?\s*", ""),
    (n_("« Nom, Prénom » → « Prénom Nom »"), r"^(.+), (.+)$", "$2 $1"),
    (n_("« Artiste - Titre » → « Titre »"), r"^.+? - ", ""),
    (n_("Plusieurs espaces → une seule"), r"\s{2,}", " "),
];

const WARN: Color32 = Color32::from_rgb(0xFF, 0xC0, 0x70);
const ERROR: Color32 = Color32::from_rgb(0xFF, 0x99, 0x99);
/// Lignes d'aperçu affichées (le décompte, lui, porte sur tous les fichiers).
const SHOWN: usize = 300;

struct Row {
    /// Petit libellé en tête de ligne (le champ touché).
    tag: &'static str,
    left: String,
    right: String,
    warn: bool,
    tip: String,
}

#[derive(Default)]
struct Preview {
    rows: Vec<Row>,
    /// Ce qui sera fait, ce qui n'a pas besoin de l'être, ce qui ne peut pas l'être.
    changes: usize,
    files: usize,
    unchanged: usize,
    problems: usize,
    error: Option<String>,
}

pub struct Tool {
    kind: ToolKind,
    /// Toute la liste plutôt que la sélection.
    all: bool,
    find: String,
    with: String,
    regex: bool,
    case: bool,
    /// Champ visé par le remplacement ; `None` : tous les champs de texte.
    field: Option<Field>,
    help: bool,
    pattern: String,
    /// Aperçu, recalculé quand la saisie change.
    key: String,
    preview: Preview,
    focused: bool,
}

/// Nouveau chemin de chaque fichier d'après le motif, ou pourquoi il ne peut pas être renommé.
fn rename_plan(
    ed: &TagEditor,
    targets: &[usize],
    pattern: &str,
) -> Vec<(usize, Result<PathBuf, &'static str>)> {
    let mut plan: Vec<(usize, Result<PathBuf, &'static str>)> = targets
        .iter()
        .map(|&k| {
            let old = &ed.files[k].path;
            let new = tagtools::format_name(pattern, &ed.values[k])
                .map(|name| match old.extension() {
                    Some(ext) => old.with_file_name(format!("{name}.{}", ext.to_string_lossy())),
                    None => old.with_file_name(name),
                })
                .ok_or(tr("les tags du motif sont vides"));
            (k, new)
        })
        .collect();
    // Deux fichiers ne peuvent pas prendre le même nom.
    let mut uses: HashMap<PathBuf, usize> = HashMap::new();
    for (_, new) in &plan {
        if let Ok(p) = new {
            *uses.entry(p.clone()).or_default() += 1;
        }
    }
    for (_, new) in &mut plan {
        if new.as_ref().is_ok_and(|p| uses[p] > 1) {
            *new = Err(tr("même nom qu'un autre fichier de la liste"));
        }
    }
    plan
}

impl Tool {
    pub fn new(kind: ToolKind, prefs: &Prefs) -> Self {
        Self {
            kind,
            all: false,
            find: String::new(),
            with: String::new(),
            regex: false,
            case: false,
            field: None,
            help: false,
            pattern: match kind {
                ToolKind::Rename => prefs.rename.clone(),
                _ => prefs.parse.clone(),
            },
            key: String::new(),
            preview: Preview::default(),
            focused: false,
        }
    }

    fn targets(&self, ed: &TagEditor) -> Vec<usize> {
        if self.all {
            ed.all_readable()
        } else {
            ed.scope()
        }
    }

    fn fields(&self) -> Vec<Field> {
        match self.field {
            Some(f) => vec![f],
            None => TEXT_FIELDS.to_vec(),
        }
    }

    fn replacer(&self) -> Result<Option<Replacer>, String> {
        Replacer::new(&self.find, &self.with, self.regex, self.case)
    }

    fn refresh(&mut self, ed: &TagEditor) {
        let targets = self.targets(ed);
        let key = format!(
            "{:?}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{:?}\u{1}{}\u{1}{}",
            self.kind,
            self.all,
            self.find,
            self.with,
            self.regex,
            self.case,
            self.field,
            self.pattern,
            targets.len()
        );
        if key == self.key {
            return;
        }
        self.key = key;
        let mut p = Preview::default();
        match self.kind {
            ToolKind::Replace => match self.replacer() {
                Err(e) => p.error = Some(trf!("Expression régulière invalide : {e}", e)),
                Ok(None) => {}
                Ok(Some(r)) => {
                    let fields = self.fields();
                    for &k in &targets {
                        let mut touched = false;
                        for &f in &fields {
                            let old = &ed.values[k][f.index()];
                            if let Some(new) = r.apply(old) {
                                p.changes += 1;
                                touched = true;
                                if p.rows.len() < SHOWN {
                                    p.rows.push(Row {
                                        tag: f.label(),
                                        left: old.clone(),
                                        right: new,
                                        warn: false,
                                        tip: file_name(&ed.files[k].path),
                                    });
                                }
                            }
                        }
                        p.files += touched as usize;
                    }
                }
            },
            ToolKind::Rename => {
                if !tagtools::has_placeholder(&self.pattern) {
                    p.error = Some(tr("Le motif doit contenir au moins un mot-clé, comme %title%.").into());
                } else {
                    for (k, new) in rename_plan(ed, &targets, &self.pattern) {
                        let old = &ed.files[k].path;
                        // La présence sur le disque n'est vérifiée que pour les lignes affichées
                        // (et de nouveau, pour toutes, au moment de renommer).
                        let new = match new {
                            Ok(n) if n != *old && p.rows.len() < SHOWN && n.exists() => {
                                Err(tr("un fichier porte déjà ce nom"))
                            }
                            other => other,
                        };
                        let (right, warn) = match &new {
                            Ok(n) if n == old => {
                                p.unchanged += 1;
                                continue;
                            }
                            Ok(n) => {
                                p.changes += 1;
                                (file_name(n), false)
                            }
                            Err(why) => {
                                p.problems += 1;
                                ((*why).to_owned(), true)
                            }
                        };
                        if p.rows.len() < SHOWN {
                            p.rows.push(Row {
                                tag: "",
                                left: file_name(old),
                                right,
                                warn,
                                tip: old.parent().map(|d| d.display().to_string()).unwrap_or_default(),
                            });
                        }
                    }
                    p.files = p.changes;
                }
            }
            ToolKind::FromName => match NameParser::new(&self.pattern) {
                None => {
                    p.error = Some(tr("Le motif doit contenir au moins un mot-clé, comme %title%.").into())
                }
                Some(parser) => {
                    for &k in &targets {
                        let path = &ed.files[k].path;
                        let stem = path.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default();
                        let (right, warn) = match parser.parse(&stem) {
                            Some(found) => {
                                p.changes += 1;
                                let text = found
                                    .iter()
                                    .map(|(f, v)| trf!("{0} : {v}", f.label(), v))
                                    .collect::<Vec<_>>()
                                    .join("  ·  ");
                                (text, false)
                            }
                            None => {
                                p.problems += 1;
                                (tr("ne correspond pas au motif").to_owned(), true)
                            }
                        };
                        if p.rows.len() < SHOWN {
                            p.rows.push(Row {
                                tag: "",
                                left: file_name(path),
                                right,
                                warn,
                                tip: String::new(),
                            });
                        }
                    }
                    p.files = p.changes;
                }
            },
        }
        self.preview = p;
    }

    /// Phrase de bilan sous l'aperçu.
    fn summary(&self) -> String {
        let p = &self.preview;
        match self.kind {
            ToolKind::Replace if self.find.is_empty() => tr("Tape le texte à rechercher.").to_owned(),
            ToolKind::Replace if p.changes == 0 => tr("Rien à remplacer.").to_owned(),
            ToolKind::Replace => trf!(
                "{0} dans {1}",
                plural(p.changes, "valeur à remplacer", "valeurs à remplacer"),
                plural(p.files, "fichier", "fichiers")
            ),
            ToolKind::Rename => {
                let mut parts = vec![plural(p.changes, "fichier à renommer", "fichiers à renommer")];
                if p.unchanged > 0 {
                    parts.push(plural(p.unchanged, "déjà bien nommé", "déjà bien nommés"));
                }
                if p.problems > 0 {
                    parts.push(plural(p.problems, "impossible", "impossibles"));
                }
                parts.join(" · ")
            }
            ToolKind::FromName => {
                let mut parts = vec![plural(p.changes, "fichier reconnu", "fichiers reconnus")];
                if p.problems > 0 {
                    parts.push(plural(
                        p.problems,
                        "ne correspond pas au motif",
                        "ne correspondent pas au motif",
                    ));
                }
                parts.join(" · ")
            }
        }
    }
}

/// Remplit les champs d'après le rechercher-remplacer ; renvoie (valeurs, fichiers) touchés.
fn apply_replace(tool: &Tool, ed: &mut TagEditor) -> (usize, usize) {
    let Ok(Some(r)) = tool.replacer() else {
        return (0, 0);
    };
    let fields = tool.fields();
    let (mut values, mut files) = (0, 0);
    for k in tool.targets(ed) {
        let mut touched = false;
        for &f in &fields {
            if let Some(new) = r.apply(&ed.values[k][f.index()]) {
                ed.values[k][f.index()] = new;
                values += 1;
                touched = true;
            }
        }
        files += touched as usize;
    }
    (values, files)
}

/// Remplit les champs lus dans les noms de fichiers ; renvoie le nombre de fichiers reconnus.
fn apply_from_name(tool: &Tool, ed: &mut TagEditor) -> usize {
    let Some(parser) = NameParser::new(&tool.pattern) else {
        return 0;
    };
    let mut files = 0;
    for k in tool.targets(ed) {
        let stem = ed.files[k]
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(found) = parser.parse(&stem) {
            for (f, v) in found {
                ed.values[k][f.index()] = v;
            }
            files += 1;
        }
    }
    files
}

impl SterMusic {
    pub(super) fn tool_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut tool) = self.tag_editor.tool.take() else {
            return;
        };
        let ed = &self.tag_editor;
        tool.refresh(ed);
        let (selected, total) = (ed.scope().len(), ed.all_readable().len());
        let (title, icon, verb) = match tool.kind {
            ToolKind::Replace => (tr("Rechercher et remplacer"), Icon::Replace, tr("Remplacer")),
            ToolKind::Rename => (
                tr("Renommer les fichiers d'après les tags"),
                Icon::FilePen,
                tr("Renommer"),
            ),
            ToolKind::FromName => (tr("Tags d'après le nom des fichiers"), Icon::Tag, tr("Appliquer")),
        };
        let (mut close, mut apply) = (false, false);
        let width = (ctx.screen_rect().width() - 100.0).clamp(620.0, 780.0);
        let modal = egui::Modal::new(egui::Id::new("tag_tool"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .inner_margin(24.0)
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                    icons::paint_centered(ui.painter(), icon, r.center(), 20.0, theme::accent_light());
                    ui.label(RichText::new(title).font(theme::semibold(20.0)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        // Sans rien de plus dans la liste que la sélection, pas de choix à faire.
                        if total > selected {
                            if segment(ui, &trf!("Toute la liste ({total})", total), tool.all).clicked() {
                                tool.all = true;
                            }
                            if segment(ui, &trf!("Sélection ({selected})", selected), !tool.all).clicked() {
                                tool.all = false;
                            }
                        }
                    });
                });
                ui.add_space(14.0);
                // L'aide et l'aperçu se partagent la hauteur de la fenêtre.
                let screen_h = ui.ctx().screen_rect().height();
                let top = ui.cursor().top();
                match tool.kind {
                    ToolKind::Replace => replace_form(ui, &mut tool, width, screen_h),
                    _ => pattern_form(ui, &mut tool, width),
                }
                ui.add_space(12.0);
                let form_h = ui.cursor().top() - top;
                preview(ui, &tool, width, (screen_h - form_h - 210.0).clamp(96.0, 250.0));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let (text, color) = match &tool.preview.error {
                        Some(e) => (e.clone(), ERROR),
                        None => (tool.summary(), theme::TEXT_2),
                    };
                    let g = widgets::galley(ui, &text, FontId::proportional(13.5), color, width - 260.0);
                    ui.label(g);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.scope(|ui| {
                            if tool.preview.changes == 0 || tool.preview.error.is_some() {
                                ui.disable();
                                ui.set_opacity(0.5);
                            }
                            if widgets::primary_button(ui, None, verb).clicked() {
                                apply = true;
                            }
                        });
                        if widgets::secondary_button(ui, None, tr("Annuler")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if modal.should_close() {
            close = true;
        }
        if !apply {
            if !close {
                self.tag_editor.tool = Some(tool);
            }
            return;
        }
        match tool.kind {
            ToolKind::Replace => {
                let (values, files) = apply_replace(&tool, &mut self.tag_editor);
                self.toast(trf!(
                    "{0} dans {1} : vérifie, puis enregistre",
                    plural(values, "valeur remplacée", "valeurs remplacées"),
                    plural(files, "fichier", "fichiers")
                ));
            }
            ToolKind::FromName => {
                self.tag_editor.prefs.parse = tool.pattern.trim().to_owned();
                let files = apply_from_name(&tool, &mut self.tag_editor);
                self.toast(trf!(
                    "Tags lus dans le nom de {0} : vérifie, puis enregistre",
                    plural(files, "fichier", "fichiers")
                ));
            }
            ToolKind::Rename => {
                if self.tag_editor.saving() {
                    self.toast(tr("Enregistrement en cours : réessaie quand il sera terminé"));
                    self.tag_editor.tool = Some(tool);
                    return;
                }
                self.tag_editor.prefs.rename = tool.pattern.trim().to_owned();
                let ed = &self.tag_editor;
                let plan = rename_plan(ed, &tool.targets(ed), &tool.pattern);
                self.rename_files(plan);
            }
        }
    }

    /// Renomme les fichiers sur le disque, et fait suivre tout ce qui les désigne par leur
    /// chemin : la liste de l'éditeur, la bibliothèque, les playlists, les favoris, les écoutes.
    fn rename_files(&mut self, plan: Vec<(usize, Result<PathBuf, &'static str>)>) {
        let mut renamed: Vec<PathBuf> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        let mut in_library = false;
        let mut history_changed = false;
        for (k, new) in plan {
            let old = self.tag_editor.files[k].path.clone();
            let result = match new {
                Ok(new) if new == old => continue,
                Ok(new) if new.exists() => Err((new, tr("un fichier porte déjà ce nom").to_owned())),
                Ok(new) => match std::fs::rename(&old, &new) {
                    Ok(()) => Ok(new),
                    Err(e) => Err((new, e.to_string())),
                },
                Err(why) => Err((old.clone(), why.to_owned())),
            };
            let new = match result {
                Ok(new) => new,
                Err((_, why)) => {
                    failed.push(format!("{} ({why})", file_name(&old)));
                    continue;
                }
            };
            let ed = &mut self.tag_editor;
            ed.known.remove(&old);
            ed.known.insert(new.clone());
            ed.files[k].path = new.clone();
            if let Some(i) = self.lib.find(&old) {
                self.lib.rename(i, new.clone());
                in_library = true;
            }
            let s = &mut self.settings;
            if s.favorites.remove(&old) {
                s.favorites.insert(new.clone());
            }
            if let Some(stat) = s.plays.remove(&old) {
                s.plays.insert(new.clone(), stat);
            }
            if let Some(stars) = s.ratings.remove(&old) {
                s.ratings.insert(new.clone(), stars);
            }
            for p in s.playlists.iter_mut().flat_map(|pl| pl.tracks.iter_mut()) {
                if *p == old {
                    *p = new.clone();
                }
            }
            if let Some(player) = &mut self.player {
                player.renamed(&old, &new);
            }
            history_changed |= crate::history::rename(&mut self.listens, &old, &new);
            renamed.push(old);
        }
        if in_library {
            library::save_cache(&self.lib.tracks);
        }
        if history_changed {
            crate::history::save(&self.listens);
        }
        self.covers.forget(&renamed);
        self.tag_editor.resort();
        let ok = plural(renamed.len(), "fichier renommé", "fichiers renommés");
        match failed.first() {
            None => self.toast(ok),
            Some(first) => self.toast(trf!(
                "{ok}, {0} : {first}",
                ok,
                plural(failed.len(), "échec", "échecs"),
                first
            )),
        }
    }
}

/// Champ de saisie qui prend le focus à l'ouverture de la fenêtre.
fn focus_once(tool: &mut Tool, resp: &egui::Response) {
    if !tool.focused {
        tool.focused = true;
        resp.request_focus();
    }
}

fn replace_form(ui: &mut egui::Ui, tool: &mut Tool, width: f32, screen_h: f32) {
    let half = (width - 10.0) / 2.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let hint = if tool.regex {
            tr("Rechercher (expression régulière)")
        } else {
            tr("Rechercher")
        };
        let r = widgets::text_field(ui, egui::Id::new("tool_find"), &mut tool.find, hint, half, 1);
        focus_once(tool, &r);
        widgets::text_field(
            ui,
            egui::Id::new("tool_with"),
            &mut tool.with,
            tr("Remplacer par (vide : supprimer)"),
            half,
            1,
        );
    });
    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.label(RichText::new(tr("Dans :")).color(theme::TEXT_2));
        if segment(ui, tr("Tous les champs"), tool.field.is_none()).clicked() {
            tool.field = None;
        }
        for f in TEXT_FIELDS {
            if segment(ui, f.label(), tool.field == Some(f)).clicked() {
                tool.field = Some(f);
            }
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if segment(ui, tr("Expression régulière"), tool.regex)
            .on_hover_text(tr(
                "Le texte recherché est un motif : « \\d+ » trouve un nombre, « ^ » le début…",
            ))
            .clicked()
        {
            tool.regex = !tool.regex;
            // L'aide s'ouvre avec le mode, la première fois qu'on en a besoin.
            tool.help |= tool.regex && tool.find.is_empty();
        }
        if segment(ui, tr("Respecter la casse"), tool.case)
            .on_hover_text(tr("« Live » ne trouve plus « live »"))
            .clicked()
        {
            tool.case = !tool.case;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::secondary_button(
                ui,
                Some(Icon::Help),
                if tool.help {
                    tr("Masquer l'aide")
                } else {
                    tr("Aide des expressions")
                },
            )
            .clicked()
            {
                tool.help = !tool.help;
            }
        });
    });
    if tool.help {
        ui.add_space(10.0);
        regex_help(ui, tool, width, (screen_h - 520.0).clamp(110.0, 320.0));
    }
}

/// Aide-mémoire des expressions régulières, et exemples qui se copient dans les champs d'un clic.
fn regex_help(ui: &mut egui::Ui, tool: &mut Tool, width: f32, max_height: f32) {
    egui::Frame::NONE
        .fill(theme::white(0x08))
        .stroke(egui::Stroke::new(1.0_f32, theme::white(0x0E)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(width - 28.0);
            egui::ScrollArea::vertical()
                .id_salt("regex_help")
                .max_height(max_height)
                .show(ui, |ui| regex_help_body(ui, tool, width - 28.0));
        });
}

fn regex_help_body(ui: &mut egui::Ui, tool: &mut Tool, width: f32) {
    {
        {
            ui.set_width(width - 10.0);
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(
                RichText::new(tr(
                    "Une expression régulière décrit une forme de texte plutôt qu'un texte exact.",
                ))
                .color(theme::TEXT_2),
            );
            ui.add_space(4.0);
            let col = (width - 10.0 - 16.0) / 2.0;
            for pair in REGEX_HELP.chunks(2) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    for (symbol, meaning) in pair {
                        let (r, _) = ui.allocate_exact_size(vec2(col, 20.0), Sense::hover());
                        ui.painter().text(
                            r.left_center(),
                            Align2::LEFT_CENTER,
                            symbol,
                            FontId::monospace(12.5),
                            theme::accent_light(),
                        );
                        let g = widgets::galley(
                            ui,
                            tr(meaning),
                            FontId::proportional(12.5),
                            theme::TEXT_2,
                            col - 112.0,
                        );
                        ui.painter().galley(
                            pos2(r.left() + 112.0, r.center().y - g.size().y / 2.0),
                            g,
                            theme::TEXT_2,
                        );
                    }
                });
            }
            ui.add_space(8.0);
            ui.label(
                RichText::new(tr("Exemples (un clic remplit les champs) :"))
                    .small()
                    .color(theme::TEXT_3),
            );
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for (what, find, with) in REGEX_EXAMPLES {
                    let tip = if with.is_empty() {
                        trf!("Rechercher : {find}\nRemplacer par : (rien)", find)
                    } else {
                        trf!("Rechercher : {find}\nRemplacer par : {with}", find, with)
                    };
                    if segment(ui, tr(what), tool.regex && tool.find == find)
                        .on_hover_text(tip)
                        .clicked()
                    {
                        tool.find = find.to_owned();
                        tool.with = with.to_owned();
                        tool.regex = true;
                    }
                }
            });
        }
    }
}

fn pattern_form(ui: &mut egui::Ui, tool: &mut Tool, width: f32) {
    let rename = tool.kind == ToolKind::Rename;
    ui.label(
        RichText::new(if rename {
            tr("Nouveau nom (sans l'extension) :")
        } else {
            tr("Forme du nom des fichiers (sans l'extension) :")
        })
        .color(theme::TEXT_2),
    );
    ui.add_space(4.0);
    let r = widgets::text_field(
        ui,
        egui::Id::new("tool_pattern"),
        &mut tool.pattern,
        "%track% - %title%",
        width,
        1,
    );
    focus_once(tool, &r);
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.label(RichText::new(tr("Mots-clés :")).color(theme::TEXT_2));
        for (name, field) in PLACEHOLDERS {
            // Les totaux servent rarement dans un nom de fichier.
            if matches!(field, Field::TrackTotal | Field::DiscTotal) {
                continue;
            }
            let key = format!("%{name}%");
            if segment(ui, &key, false)
                .on_hover_text(trf!("{0} : ajoute {key} au motif", field.label(), key))
                .clicked()
            {
                tool.pattern.push_str(&key);
            }
        }
        if !rename
            && segment(ui, "%_%", false)
                .on_hover_text(tr("Un morceau du nom à ignorer"))
                .clicked()
        {
            tool.pattern.push_str("%_%");
        }
    });
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.label(RichText::new(tr("Motifs courants :")).color(theme::TEXT_2));
        for p in PATTERNS {
            if segment(ui, p, tool.pattern.trim() == p).clicked() {
                tool.pattern = p.to_owned();
            }
        }
    });
    ui.add_space(8.0);
    ui.label(
        RichText::new(if rename {
            tr("Les fichiers sont renommés tout de suite, dans leur dossier ; playlists, favoris et \
             compteurs d'écoutes suivent. Les tags modifiés mais pas encore enregistrés comptent.")
        } else {
            tr("Les tags trouvés remplissent les champs de l'éditeur : rien n'est écrit avant « Enregistrer ».")
        })
        .small()
        .color(theme::TEXT_3),
    );
}

/// Aperçu : ce que deviendra chaque valeur ou chaque nom.
fn preview(ui: &mut egui::Ui, tool: &Tool, width: f32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        theme::white(0x05),
        egui::Stroke::new(1.0_f32, theme::white(0x0E)),
        egui::StrokeKind::Inside,
    );
    let rows = &tool.preview.rows;
    if rows.is_empty() {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            tr("L'aperçu des changements s'affichera ici"),
            FontId::proportional(13.0),
            theme::TEXT_3,
        );
        return;
    }
    let tagged = rows.iter().any(|r| !r.tag.is_empty());
    let tag_w = if tagged { 120.0 } else { 0.0 };
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(4.0)));
    inner.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt("tool_preview")
        .auto_shrink(false)
        .show_rows(&mut inner, 26.0, rows.len(), |ui, range| {
            for i in range {
                let row = &rows[i];
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                if i % 2 == 1 {
                    ui.painter()
                        .rect_filled(r, CornerRadius::same(4), theme::white(0x03));
                }
                let y = r.center().y;
                let x = r.left() + 8.0;
                if tagged {
                    let g = widgets::galley(
                        ui,
                        row.tag,
                        FontId::proportional(12.0),
                        theme::TEXT_3,
                        tag_w - 8.0,
                    );
                    ui.painter()
                        .galley(pos2(x, y - g.size().y / 2.0), g, theme::TEXT_3);
                }
                let half = (r.width() - 16.0 - tag_w - 30.0) / 2.0;
                let g = widgets::galley(ui, &row.left, FontId::proportional(13.0), theme::TEXT_2, half);
                ui.painter()
                    .galley(pos2(x + tag_w, y - g.size().y / 2.0), g, theme::TEXT_2);
                ui.painter().text(
                    pos2(x + tag_w + half + 15.0, y),
                    Align2::CENTER_CENTER,
                    "→",
                    FontId::proportional(13.0),
                    theme::TEXT_3,
                );
                let (text, color) = match (row.warn, row.right.is_empty()) {
                    (true, _) => (row.right.as_str(), WARN),
                    (false, true) => ("(vide)", theme::TEXT_3),
                    (false, false) => (row.right.as_str(), theme::accent_light()),
                };
                let g = widgets::galley(ui, text, FontId::proportional(13.0), color, half);
                ui.painter()
                    .galley(pos2(x + tag_w + half + 30.0, y - g.size().y / 2.0), g, color);
                if !row.tip.is_empty() {
                    resp.on_hover_text(&row.tip);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::FileTags;

    fn editor(files: &[(&str, &str, &str)]) -> TagEditor {
        let mut ed = TagEditor::new(Prefs::default());
        for (path, track, title) in files {
            let mut values: crate::tags::Values = Default::default();
            values[Field::Track.index()] = (*track).to_owned();
            values[Field::Title.index()] = (*title).to_owned();
            ed.files.push(FileTags {
                path: PathBuf::from(path),
                values: values.clone(),
                cover: None,
                kind: String::new(),
                tags: String::new(),
                duration_ms: 0,
                size: 0,
                error: None,
            });
            ed.values.push(values);
            ed.covers.push(None);
        }
        ed
    }

    #[test]
    fn rename_plans() {
        let ed = editor(&[
            ("/m/a/x.mp3", "3", "Un: deux"),
            ("/m/a/y.FLAC", "", "Seul"),
            ("/m/a/z.mp3", "", ""),
            ("/m/a/d1.mp3", "1", "Pareil"),
            ("/m/a/d2.mp3", "1", "Pareil"),
            // Même nom, autre dossier : pas de conflit.
            ("/m/b/d3.mp3", "1", "Pareil"),
        ]);
        let plan = rename_plan(&ed, &[0, 1, 2, 3, 4, 5], "%track% - %title%");
        let new = |k: usize| plan[k].1.clone().map(|p| p.display().to_string());
        assert_eq!(new(0).unwrap(), "/m/a/03 - Un- deux.mp3");
        assert_eq!(new(1).unwrap(), "/m/a/Seul.FLAC");
        assert!(new(2).is_err());
        assert!(new(3).is_err() && new(4).is_err());
        assert_eq!(new(5).unwrap(), "/m/b/01 - Pareil.mp3");
    }
}
