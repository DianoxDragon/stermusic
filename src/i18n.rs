//! Langue de l'interface : français (la langue du code) ou anglais.
//!
//! Les textes restent écrits en français dans le code ; [`tr`] donne leur traduction dans la
//! langue choisie, et [`trf!`] fait de même pour un texte à trous (« {n} fichiers »), avant de
//! remplir les trous. Un texte sans traduction s'affiche en français : rien ne casse s'il en
//! manque une. Les traductions anglaises sont dans `i18n_en.rs`.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Lang {
    #[default]
    Fr,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Fr, Lang::En];

    /// Nom de la langue, dans cette langue (on la reconnaît même sans lire l'autre).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Fr => "Français",
            Lang::En => "English",
        }
    }

    /// Langue du bureau (`LC_ALL`, `LC_MESSAGES`, `LANG`) : français si elle l'est, sinon anglais.
    pub fn from_env() -> Lang {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        if locale.is_empty() || locale.starts_with("fr") || locale == "C" || locale.starts_with("C.") {
            Lang::Fr
        } else {
            Lang::En
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::En,
        _ => Lang::Fr,
    }
}

fn english() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| crate::i18n_en::EN.iter().copied().collect())
}

/// Marque un texte à traduire là où `tr` ne peut pas être appelé (une constante) : la traduction
/// se fait à l'affichage, avec `tr`. Sert aussi au test qui vérifie que tout est traduit.
pub const fn n_(fr: &'static str) -> &'static str {
    fr
}

/// Le texte dans la langue de l'interface (un texte inconnu est rendu tel quel : un titre de
/// morceau, par exemple).
pub fn tr(fr: &str) -> &str {
    match lang() {
        Lang::Fr => fr,
        Lang::En => english().get(fr).copied().unwrap_or(fr),
    }
}

/// Noms des trous d'un texte (« {n} », « {total} »), dans l'ordre de leur première apparition.
fn holes(template: &str) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        let after = &rest[i + 1..];
        // « {{ » et « }} » sont des accolades écrites telles quelles.
        if after.starts_with(&rest[i..i + 1]) {
            rest = &after[1..];
            continue;
        }
        if rest.as_bytes()[i] == b'{' {
            if let Some(end) = after.find('}') {
                let name = &after[..end];
                if !names.contains(&name) {
                    names.push(name);
                }
                rest = &after[end + 1..];
                continue;
            }
        }
        rest = after;
    }
    names
}

/// Texte à trous dans la langue de l'interface : `trf!("{n} fichiers sur {total}", n, total)`.
/// Les valeurs sont données dans l'ordre où leurs trous apparaissent dans le texte français ; la
/// traduction peut les placer ailleurs.
#[macro_export]
macro_rules! trf {
    ($fr:expr $(, $value:expr)* $(,)?) => {
        $crate::i18n::fill($fr, &[$(&$value as &dyn ::std::fmt::Display),*])
    };
}

/// Ce que fait `trf!` : traduit le texte, puis remplit ses trous.
pub fn fill(fr: &str, values: &[&dyn Display]) -> String {
    let names = holes(fr);
    let text = tr(fr);
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let brace = &rest[i..i + 1];
        let after = &rest[i + 1..];
        if after.starts_with(brace) {
            out.push_str(brace);
            rest = &after[1..];
            continue;
        }
        if brace == "{" {
            if let Some(end) = after.find('}') {
                let name = &after[..end];
                match names.iter().position(|n| *n == name).and_then(|k| values.get(k)) {
                    Some(v) => out.push_str(&v.to_string()),
                    None => out.push_str(&rest[i..i + end + 2]),
                }
                rest = &after[end + 1..];
                continue;
            }
        }
        out.push_str(brace);
        rest = after;
    }
    out.push_str(rest);
    out
}

/// « 60 % » en français, « 60% » en anglais.
pub fn percent(value: f32) -> String {
    match lang() {
        Lang::Fr => format!("{value:.0} %"),
        Lang::En => format!("{value:.0}%"),
    }
}

/// « 3 fichiers », « 1 fichier » : le nombre et le mot, au singulier ou au pluriel selon la
/// langue (en français, 0 est au singulier ; en anglais, au pluriel).
pub fn plural(n: usize, one: &str, many: &str) -> String {
    let single = match lang() {
        Lang::Fr => n <= 1,
        Lang::En => n == 1,
    };
    format!("{n} {}", tr(if single { one } else { many }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une seule fonction de test touche à la langue (globale) : les tests tournent en parallèle.
    #[test]
    fn translations() {
        set(Lang::Fr);
        assert_eq!(tr("Fermer"), "Fermer");
        assert_eq!(fill("{n} fichiers sur {total}", &[&3, &10]), "3 fichiers sur 10");
        assert_eq!(crate::trf!("{0} · {1}", "a", "b"), "a · b");
        assert_eq!(fill("{{brut}} {n}", &[&1]), "{brut} 1");
        assert_eq!(plural(0, "fichier", "fichiers"), "0 fichier");
        set(Lang::En);
        assert_eq!(tr("Fermer"), "Close");
        assert_eq!(tr("texte sans traduction"), "texte sans traduction");
        assert_eq!(plural(0, "fichier", "fichiers"), "0 files");
        assert_eq!(plural(1, "fichier", "fichiers"), "1 file");
        set(Lang::Fr);
        assert_eq!(holes("{a} {b} {a} {{x}}"), vec!["a", "b"]);
    }

    /// Chaque texte traduit garde les mêmes trous que l'original (sinon une valeur se perdrait).
    #[test]
    fn same_holes() {
        for (fr, en) in crate::i18n_en::EN {
            let mut a = holes(fr);
            let mut b = holes(en);
            a.sort_unstable();
            b.sort_unstable();
            assert_eq!(a, b, "« {fr} » → « {en} »");
        }
    }

    /// Chaque texte du code passé à `tr`, `trf` ou `plural` a sa traduction anglaise.
    #[test]
    fn everything_translated() {
        let english = english();
        let mut missing = Vec::new();
        for text in crate::i18n_keys::keys() {
            if !english.contains_key(text.as_str()) {
                missing.push(text);
            }
        }
        assert!(
            missing.is_empty(),
            "{} textes sans traduction :\n{}",
            missing.len(),
            missing.join("\n")
        );
    }
}
