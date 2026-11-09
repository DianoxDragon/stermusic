//! Pour les tests : tous les textes du code passés à `tr`, `trf!`, `n_` ou `plural`, lus
//! directement dans les fichiers sources (voir le test `everything_translated`).

use std::path::Path;

/// Contenu d'un littéral Rust ("…" sans les guillemets), échappements interprétés.
fn unescape(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('u') => {
                let code: String = chars.by_ref().skip(1).take_while(|&c| c != '}').collect();
                if let Some(c) = u32::from_str_radix(&code, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                }
            }
            // Fin de ligne échappée : la suite reprend au premier caractère non blanc.
            Some('\n') => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            Some(c) => out.push(c),
            None => {}
        }
    }
    out
}

/// Le littéral qui commence à `at` (sur le guillemet), et l'indice qui le suit.
fn literal(src: &str, at: usize) -> Option<(String, usize)> {
    let bytes = src.as_bytes();
    if bytes.get(at) != Some(&b'"') {
        return None;
    }
    let mut i = at + 1;
    while i < bytes.len() && bytes[i] != b'"' {
        i += if bytes[i] == b'\\' { 2 } else { 1 };
    }
    Some((unescape(&src[at + 1..i]), i + 1))
}

fn skip_space(src: &str, mut i: usize) -> usize {
    let bytes = src.as_bytes();
    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    i
}

/// Indice de la virgule qui termine l'argument commençant à `i` (profondeur zéro).
fn end_of_arg(src: &str, mut i: usize) -> usize {
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = literal(src, i).map(|l| l.1).unwrap_or(i + 1);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth == 0 {
                    return i;
                }
                depth -= 1;
            }
            b',' if depth == 0 => return i,
            _ => {}
        }
        i += 1;
    }
    i
}

fn scan(src: &str, keys: &mut Vec<String>) {
    // Les tests eux-mêmes ne comptent pas.
    let src = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
    for call in ["tr(", "trf!(", "n_(", "plural("] {
        let mut from = 0;
        while let Some(found) = src[from..].find(call) {
            let at = from + found;
            from = at + call.len();
            // « tr( » au milieu d'un autre nom (« attr( »…) ou une définition de fonction.
            let before = src[..at].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') || src[..at].ends_with("fn ") {
                continue;
            }
            let mut i = skip_space(src, from);
            if call == "plural(" {
                // Le premier argument est le nombre ; viennent ensuite le singulier et le pluriel.
                i = skip_space(src, end_of_arg(src, i) + 1);
                for _ in 0..2 {
                    match literal(src, i) {
                        Some((text, end)) => {
                            keys.push(text);
                            i = skip_space(src, end_of_arg(src, end) + 1);
                        }
                        None => break,
                    }
                }
            } else if let Some((text, _)) = literal(src, i) {
                keys.push(text);
            }
        }
    }
}

pub fn keys() -> Vec<String> {
    let mut keys = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut dirs = vec![root];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && !path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("i18n"))
            {
                scan(&std::fs::read_to_string(&path).unwrap(), &mut keys);
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

/// Liste les textes sans traduction dans un fichier JSON :
/// `STERMUSIC_KEYS=keys.json cargo test dump_keys -- --ignored`.
#[test]
#[ignore]
fn dump_keys() {
    let path = std::env::var("STERMUSIC_KEYS").unwrap();
    let english: std::collections::HashSet<&str> = crate::i18n_en::EN.iter().map(|p| p.0).collect();
    let missing: Vec<String> = keys()
        .into_iter()
        .filter(|k| !english.contains(k.as_str()))
        .collect();
    std::fs::write(path, serde_json::to_string_pretty(&missing).unwrap()).unwrap();
}
