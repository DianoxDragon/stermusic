//! Interface language: French (the language of the code) or English.
//!
//! Texts stay written in French in the code; [`tr`] gives their translation in the chosen
//! language, and [`trf!`] does the same for a text with placeholders ("{n} fichiers"), before
//! filling them in. A text without a translation is shown in French: nothing breaks if one is
//! missing. The English translations are in `i18n_en.rs`.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Lang {
    Fr,
    #[default]
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Fr, Lang::En];

    /// Name of the language, in that language (recognizable even without reading the other one).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Fr => "Français",
            Lang::En => "English",
        }
    }

    /// Desktop language (`LC_ALL`, `LC_MESSAGES`, `LANG`): French if it is French, otherwise English
    /// (including with no language set, "C" or empty).
    pub fn from_env() -> Lang {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        if locale.starts_with("fr") {
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

/// Marks a text to translate where `tr` cannot be called (a constant): the translation happens
/// when it is displayed, with `tr`. Also used by the test that checks everything is translated.
pub const fn n_(fr: &'static str) -> &'static str {
    fr
}

/// The text in the interface language (an unknown text is returned as is: a track title, for
/// example).
pub fn tr(fr: &str) -> &str {
    match lang() {
        Lang::Fr => fr,
        Lang::En => english().get(fr).copied().unwrap_or(fr),
    }
}

/// Names of the placeholders of a text ("{n}", "{total}"), in order of first appearance.
fn holes(template: &str) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        let after = &rest[i + 1..];
        // "{{" and "}}" are literal braces.
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

/// Text with placeholders in the interface language: `trf!("{n} fichiers sur {total}", n, total)`.
/// The values are given in the order their placeholders appear in the French text; the
/// translation may place them elsewhere.
#[macro_export]
macro_rules! trf {
    ($fr:expr $(, $value:expr)* $(,)?) => {
        $crate::i18n::fill($fr, &[$(&$value as &dyn ::std::fmt::Display),*])
    };
}

/// What `trf!` does: translates the text, then fills its placeholders.
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

/// "60 %" in French, "60%" in English.
pub fn percent(value: f32) -> String {
    match lang() {
        Lang::Fr => format!("{value:.0} %"),
        Lang::En => format!("{value:.0}%"),
    }
}

/// "3 files", "1 file": the number and the word, singular or plural depending on the language
/// (in French, 0 is singular; in English, plural).
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

    /// A single test function touches the (global) language: tests run in parallel.
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

    /// Every translated text keeps the same placeholders as the original (otherwise a value would be lost).
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

    /// Every text of the code passed to `tr`, `trf` or `plural` has its English translation.
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
