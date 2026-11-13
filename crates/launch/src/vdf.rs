//! A reader for Valve's `KeyValues` text, the format of Steam's library list and app manifests.
//!
//! A file is a tree of key and value pairs. A value is a quoted string or a `{ }` block of pairs.

use std::iter::Peekable;
use std::str::Chars;

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Text(String),
    Open,
    Close,
}

/// The text values of every `key` entry at any depth, in file order. Keys match without case, as
/// Steam reads them.
pub(crate) fn values(text: &str, key: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending_key: Option<String> = None;
    for token in tokens(text) {
        match (token, pending_key.take()) {
            (Token::Text(value), Some(named)) => {
                if named.eq_ignore_ascii_case(key) {
                    found.push(value);
                }
            }
            (Token::Text(named), None) => pending_key = Some(named),
            (Token::Open | Token::Close, _) => {}
        }
    }
    found
}

fn tokens(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(next) = chars.next() {
        match next {
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '"' => tokens.push(Token::Text(quoted(&mut chars))),
            other if other.is_whitespace() => {}
            other => tokens.push(Token::Text(unquoted(other, &mut chars))),
        }
    }
    tokens
}

/// The rest of a quoted string after its opening quote. Steam escapes a backslash as `\\`.
fn quoted(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut text = String::new();
    while let Some(next) = chars.next() {
        match next {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => text.push('\n'),
                Some('t') => text.push('\t'),
                Some(escaped) => text.push(escaped),
                None => text.push('\\'),
            },
            other => text.push(other),
        }
    }
    text
}

/// A string written without quotes, which ends at whitespace, a quote or a brace.
fn unquoted(first: char, chars: &mut Peekable<Chars<'_>>) -> String {
    let mut text = String::from(first);
    while let Some(next) = chars.next_if(|next| !next.is_whitespace() && !"\"{}".contains(*next)) {
        text.push(next);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::values;

    #[test]
    fn doubled_backslashes_read_as_one() {
        let text = r#""libraryfolders" { "0" { "path" "C:\\Program Files (x86)\\Steam" } }"#;
        assert_eq!(values(text, "path"), [r"C:\Program Files (x86)\Steam"]);
    }

    #[test]
    fn values_of_a_key_come_from_every_block_in_file_order() {
        let text = concat!(
            "\"libraryfolders\"\n{\n",
            "\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Steam\"\n\t\t\"label\"\t\t\"\"\n",
            "\t\t\"apps\"\n\t\t{\n\t\t\t\"228980\"\t\t\"123\"\n\t\t}\n\t}\n",
            "\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}\n",
        );
        assert_eq!(values(text, "path"), [r"C:\Steam", r"D:\SteamLibrary"]);
    }

    #[test]
    fn a_key_that_is_another_key_s_value_is_not_read_as_a_key() {
        let text = r#""0" { "label" "path" "path" "E:\\Games" "name" "path" }"#;
        assert_eq!(values(text, "path"), [r"E:\Games"]);
    }

    #[test]
    fn keys_match_without_case_and_a_block_value_is_skipped() {
        let text = r#""AppState" { "InstallDir" "Baldurs Gate 3" "installdir" { "x" "y" } }"#;
        assert_eq!(values(text, "installdir"), ["Baldurs Gate 3"]);
    }

    #[test]
    fn unquoted_strings_and_escapes_read_as_steam_writes_them() {
        let text = "root { key value \"quote\" \"a\\\"b\\tc\" }";
        assert_eq!(values(text, "key"), ["value"]);
        assert_eq!(values(text, "quote"), ["a\"b\tc"]);
    }

    #[test]
    fn empty_or_unterminated_text_yields_what_it_holds() {
        assert_eq!(values("", "path"), Vec::<String>::new());
        assert_eq!(values(r#""path" "C:\Ste"#, "path"), [r"C:Ste"]);
        assert_eq!(values(r#""path""#, "path"), Vec::<String>::new());
    }
}
