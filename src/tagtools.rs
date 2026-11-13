//! Tag editor tools, without interface: file name patterns ("%track% - %title%"), both ways,
//! and search and replace (plain text or regular expression).

use regex::{NoExpand, Regex, RegexBuilder};

use crate::tags::{Field, Values};

/// Pattern keywords, between "%".
pub const PLACEHOLDERS: [(&str, Field); 11] = [
    ("title", Field::Title),
    ("artist", Field::Artist),
    ("album", Field::Album),
    ("albumartist", Field::AlbumArtist),
    ("year", Field::Year),
    ("track", Field::Track),
    ("tracktotal", Field::TrackTotal),
    ("disc", Field::Disc),
    ("disctotal", Field::DiscTotal),
    ("genre", Field::Genre),
    ("composer", Field::Composer),
];

#[derive(Debug, PartialEq)]
enum Token {
    Text(String),
    Field(Field),
    /// "%_%": a part of the name to ignore (tags ← file name).
    Skip,
}

/// Splits a pattern; an unknown keyword ("%thing%") stays text.
fn tokens(pattern: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = pattern;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let known = after.find('%').and_then(|end| {
            let name = after[..end].to_ascii_lowercase();
            let token = if name == "_" {
                Some(Token::Skip)
            } else {
                PLACEHOLDERS
                    .iter()
                    .find(|p| p.0 == name)
                    .map(|p| Token::Field(p.1))
            };
            token.map(|t| (t, end))
        });
        match known {
            Some((token, end)) => {
                text.push_str(&rest[..start]);
                if !text.is_empty() {
                    out.push(Token::Text(std::mem::take(&mut text)));
                }
                out.push(token);
                rest = &after[end + 1..];
            }
            None => {
                text.push_str(&rest[..=start]);
                rest = after;
            }
        }
    }
    text.push_str(rest);
    if !text.is_empty() {
        out.push(Token::Text(text));
    }
    out
}

/// True if the pattern contains at least one known keyword.
pub fn has_placeholder(pattern: &str) -> bool {
    tokens(pattern).iter().any(|t| matches!(t, Token::Field(_)))
}

/// Makes a text usable as a file name, including on an NTFS or FAT partition (USB stick, disk
/// shared with Windows).
fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '/' | '\\' | ':' | '|' => out.push('-'),
            '"' => out.push('\''),
            '?' | '*' | '<' | '>' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    // Windows refuses names that end with a dot or a space.
    let out = out.trim_end_matches(['.', ' ']).trim_start();
    out.chars().take(180).collect::<String>().trim_end().to_owned()
}

/// File name (without extension) from the tags. Track and disc numbers are written with two
/// digits; an empty field takes the separator that follows it along ("%track% - %title%" without
/// a number gives "Title", not "- Title"). `None` if the result is empty.
pub fn format_name(pattern: &str, values: &Values) -> Option<String> {
    let mut out = String::new();
    let mut skip_separator = false;
    for token in tokens(pattern) {
        match token {
            Token::Text(t) => {
                let separator = !t.chars().any(char::is_alphanumeric);
                if !(skip_separator && separator) {
                    out.push_str(&t);
                }
                skip_separator = false;
            }
            Token::Field(f) => {
                let v = values[f.index()].trim();
                skip_separator = v.is_empty();
                if matches!(f, Field::Track | Field::Disc) && !v.is_empty() {
                    out.push_str(&format!("{v:0>2}"));
                } else {
                    // A "/" in a title must not create a subfolder.
                    out.push_str(&v.replace('/', "-"));
                }
            }
            Token::Skip => skip_separator = false,
        }
    }
    let name = sanitize(&out);
    (!name.is_empty()).then_some(name)
}

/// Reads tags from a file name (without extension) according to a pattern:
/// "%track% - %artist% - %title%" on "03 - Daft Punk - Aerodynamic". `None` if the name doesn't
/// match the pattern.
pub struct NameParser {
    re: Regex,
    fields: Vec<Field>,
}

impl NameParser {
    pub fn new(pattern: &str) -> Option<Self> {
        let tokens = tokens(pattern.trim());
        let mut re = String::from("^");
        let mut fields = Vec::new();
        for t in &tokens {
            match t {
                // Spaces in the pattern accept any number of spaces or "_".
                Token::Text(t) => {
                    let mut in_space = false;
                    for c in t.chars() {
                        if c.is_whitespace() {
                            if !in_space {
                                re.push_str(r"[\s_]*");
                            }
                            in_space = true;
                        } else {
                            in_space = false;
                            re.push_str(&regex::escape(c.encode_utf8(&mut [0; 4])));
                        }
                    }
                }
                Token::Field(f) => {
                    re.push_str(if f.numeric() { r"(\d+)" } else { "(.+?)" });
                    fields.push(*f);
                }
                Token::Skip => re.push_str(".+?"),
            }
        }
        re.push('$');
        if fields.is_empty() {
            return None;
        }
        Some(Self {
            re: Regex::new(&re).ok()?,
            fields,
        })
    }

    pub fn parse(&self, stem: &str) -> Option<Vec<(Field, String)>> {
        let caps = self.re.captures(stem.trim())?;
        let out = self
            .fields
            .iter()
            .enumerate()
            .map(|(i, &f)| {
                let v = caps.get(i + 1).map_or("", |m| m.as_str());
                let v = if f.numeric() {
                    // "03" → "3", as tags are shown elsewhere.
                    let n = v.trim_start_matches('0');
                    if n.is_empty() && !v.is_empty() { "0" } else { n }.to_owned()
                } else {
                    v.replace('_', " ")
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                (f, v)
            })
            .collect();
        Some(out)
    }
}

/// Search and replace: plain text or regular expression ("$1" reuses a group).
pub struct Replacer {
    re: Regex,
    with: String,
    regex: bool,
}

impl Replacer {
    /// `Ok(None)`: nothing to search for. `Err`: invalid regular expression (readable message).
    pub fn new(find: &str, with: &str, regex: bool, case_sensitive: bool) -> Result<Option<Self>, String> {
        if find.is_empty() {
            return Ok(None);
        }
        let pattern = if regex {
            find.to_owned()
        } else {
            regex::escape(find)
        };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|e| {
                // The last line of the `regex` message says what matters ("error: unclosed group").
                let text = e.to_string();
                let last = text.lines().last().unwrap_or("").trim();
                last.strip_prefix("error: ").unwrap_or(last).to_owned()
            })?;
        Ok(Some(Self {
            re,
            with: with.to_owned(),
            regex,
        }))
    }

    /// The text after replacement, or `None` if it doesn't change.
    pub fn apply(&self, text: &str) -> Option<String> {
        if !self.re.is_match(text) {
            return None;
        }
        let out = if self.regex {
            self.re.replace_all(text, self.with.as_str())
        } else {
            self.re.replace_all(text, NoExpand(&self.with))
        };
        let out = out.trim();
        (out != text).then(|| out.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(Field, &str)]) -> Values {
        let mut v: Values = Default::default();
        for (f, s) in pairs {
            v[f.index()] = (*s).to_owned();
        }
        v
    }

    #[test]
    fn names_from_tags() {
        let v = values(&[
            (Field::Title, "Around the World / Harder: Better?"),
            (Field::Artist, "Daft Punk"),
            (Field::Track, "3"),
        ]);
        let f = |p: &str| format_name(p, &v).unwrap();
        assert_eq!(f("%track% - %title%"), "03 - Around the World - Harder- Better");
        assert_eq!(
            f("%artist% - %TITLE%"),
            "Daft Punk - Around the World - Harder- Better"
        );
        // Empty field: its separator disappears; unknown keyword: kept as is.
        assert_eq!(f("%disc% - %track%. %artist%"), "03. Daft Punk");
        assert_eq!(f("%album% (%year%) %artist%"), "Daft Punk");
        assert_eq!(f("100%truc% %artist%"), "100%truc% Daft Punk");
        assert!(format_name("%album%", &v).is_none());
        let dots = values(&[(Field::Title, "...Baby One More Time...")]);
        assert_eq!(
            format_name("%track% - %title%", &dots).unwrap(),
            "...Baby One More Time"
        );
    }

    #[test]
    fn tags_from_names() {
        let p = |pattern: &str, name: &str| NameParser::new(pattern).and_then(|p| p.parse(name));
        assert_eq!(
            p("%track% - %artist% - %title%", "03 - Daft Punk - Harder - Better").unwrap(),
            vec![
                (Field::Track, "3".to_owned()),
                (Field::Artist, "Daft Punk".to_owned()),
                (Field::Title, "Harder - Better".to_owned()),
            ]
        );
        assert_eq!(
            p("%track%. %title%", "07.Mon_titre").unwrap(),
            vec![
                (Field::Track, "7".to_owned()),
                (Field::Title, "Mon titre".to_owned())
            ]
        );
        assert_eq!(
            p("%_% - %title%", "xyz - Fin").unwrap(),
            vec![(Field::Title, "Fin".to_owned())]
        );
        assert!(p("%track% - %title%", "Sans numéro").is_none());
        assert!(NameParser::new("pas de mot-clé").is_none());
    }

    #[test]
    fn find_and_replace() {
        let r = |find: &str, with: &str, regex: bool, case: bool| {
            Replacer::new(find, with, regex, case).unwrap().unwrap()
        };
        assert_eq!(
            r("feat.", "avec", false, false).apply("A FEAT. B").unwrap(),
            "A avec B"
        );
        assert!(r("feat.", "avec", false, true).apply("A FEAT. B").is_none());
        // In plain text, neither the pattern nor the replacement are interpreted.
        assert_eq!(
            r("(live)", "$1", false, false).apply("Titre (Live)").unwrap(),
            "Titre $1"
        );
        assert_eq!(
            r(r"^(.+), (.+)$", "$2 $1", true, false)
                .apply("Brel, Jacques")
                .unwrap(),
            "Jacques Brel"
        );
        assert_eq!(
            r(r"\s*[\(\[]feat\.?[^\)\]]*[\)\]]", "", true, false)
                .apply("Titre (feat. X)")
                .unwrap(),
            "Titre"
        );
        assert!(Replacer::new("", "x", false, false).unwrap().is_none());
        assert!(Replacer::new("(", "x", true, false).is_err());
        assert!(Replacer::new("(", "x", false, false).is_ok());
    }
}
