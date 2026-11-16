//! Shared lexical representation. Database tokenizers receive bounded generated
//! lexemes; phrase verification uses the canonical sequence, never capped native
//! position lists. Raw mailbox text remains unchanged in search storage.
use crate::{Error, Result};
use sha2::{Digest, Sha256};
use unicode_general_category::{get_general_category, GeneralCategory as G};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

pub(crate) const REVISION: i64 = 5;
pub(crate) const WINDOW: usize = 64 * 1024;
pub(crate) const CHUNK: usize = 64 * 1024;
pub(crate) const OVERLAP: usize = 32 * 1024;
pub(crate) const WEIGHTS: [i64; 6] = [4, 4, 5, 1, 3, 1];
pub(crate) const PHRASE_WEIGHTS: [i64; 6] = [4, 4, 6, 2, 4, 2];
pub(crate) const EXACT_SUBJECT: i64 = 8;
pub(crate) const EXACT_PARTICIPANT: i64 = 6;
const WORD_KEY: usize = 511;
const GAP: &str = "g0";

/// Native lexemes use ASCII only. Database tokenizers cannot fold distinct
/// canonical Unicode words or split their meaningful combining marks.
pub(crate) fn native_unit(canonical: &str) -> String {
    encode_unit(canonical, None)
}
pub(crate) fn field_unit(canonical: &str, field: usize) -> String {
    debug_assert!(field < WEIGHTS.len());
    encode_unit(canonical, Some(field))
}
fn encode_unit(canonical: &str, field: Option<usize>) -> String {
    let mut native = String::with_capacity(canonical.len() * 2 + 1);
    for (position, word) in canonical.split_whitespace().enumerate() {
        if position > 0 {
            native.push(' ');
        }
        if let Some(field) = field {
            native.push('f');
            native.push(char::from(b'0' + field as u8));
        }
        native.push('n');
        for byte in word.bytes() {
            native.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
            native.push(char::from(b"0123456789abcdef"[(byte & 15) as usize]));
        }
    }
    native
}
pub(crate) fn native_scope(account: &str) -> String {
    format!("a{:x}", Sha256::digest(account.as_bytes()))
}
pub(crate) fn native_document(account: &str, field: usize, canonical: &str) -> (String, bool) {
    let mut counts = std::collections::HashMap::new();
    let mut positions = 1usize; // Account scope occupies the first position.
    let mut safe = true;
    for word in canonical.split_whitespace() {
        positions += 1;
        let count = counts.entry(word).or_insert(0usize);
        *count += 1;
        safe &= *count <= 256 && positions <= 16383;
    }
    (
        format!("{} {}", native_scope(account), field_unit(canonical, field)),
        safe,
    )
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(
            get_general_category(c),
            G::NonspacingMark | G::SpacingMark | G::EnclosingMark
        )
}
fn symbol(c: char) -> bool {
    matches!(
        get_general_category(c),
        G::MathSymbol | G::CurrencySymbol | G::ModifierSymbol | G::OtherSymbol
    ) || matches!(c, '\u{200c}' | '\u{200d}')
}
fn word_key(word: &str) -> Option<String> {
    let lower = word
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    // Fold common Latin accents without deleting vowel signs and other
    // meaning-bearing marks in Indic, Arabic, Greek or Cyrillic words.
    let mut latin = false;
    let mut value = String::new();
    for c in lower.nfd() {
        if matches!(c,'\u{fe00}'..='\u{fe0f}'|'\u{e0100}'..='\u{e01ef}') {
            continue;
        }
        if is_combining_mark(c) {
            if latin {
                continue;
            }
        } else {
            latin = c.is_ascii_alphabetic();
        }
        value.push(c);
    }
    if value.is_empty() {
        None
    } else if value.len() <= WORD_KEY {
        Some(format!("w{value}"))
    } else {
        Some(format!("h{:x}", Sha256::digest(value.as_bytes())))
    }
}
/// Complete input tokens, excluding an unfinished trailing word. A word longer
/// than a work window cannot be expressed by the 4 KiB public query grammar;
/// preserve an unqueryable boundary instead of inventing a shorter word or
/// allowing a phrase to join across the skipped content.
fn tokens(text: &str, finished: bool, mut skipping: bool) -> (Vec<String>, usize, bool) {
    let mut start = 0;
    if skipping {
        if let Some((position, _)) = text.char_indices().find(|(_, c)| !word_char(*c)) {
            start = position;
            skipping = false;
        } else {
            return (vec![], text.len(), true);
        }
    }
    let mut end = text.len();
    let mut gap = false;
    if !finished && text.chars().last().is_some_and(word_char) {
        if let Some((position, c)) = text[start..]
            .char_indices()
            .rev()
            .find(|(_, c)| !word_char(*c))
        {
            end = start + position + c.len_utf8();
        } else {
            end = start;
            gap = true;
        }
    }
    let mut result = vec![];
    let mut word = None;
    for (position, c) in text[start..end].char_indices() {
        let position = start + position;
        if word_char(c) {
            word.get_or_insert(position);
        } else {
            if let Some(begin) = word.take() {
                if let Some(key) = word_key(&text[begin..position]) {
                    result.push(key);
                }
            }
            if symbol(c) {
                result.push(format!("s{:x}", c as u32));
            }
        }
    }
    if let Some(begin) = word {
        if let Some(key) = word_key(&text[begin..end]) {
            result.push(key);
        }
    }
    if gap {
        result.push(GAP.into());
        end = text.len();
        skipping = true;
    }
    (result, end, skipping)
}

/// A complete query unit. One unit is one word or one quoted phrase, including
/// punctuation inside addresses. `None` explicitly requires provider fallback.
pub(crate) fn unit(text: &str) -> Result<Option<String>> {
    if text.len() > 4096 || text.contains('\0') {
        return Err(Error::Invalid);
    }
    let (tokens, _, _) = tokens(text, true, false);
    let value = tokens.join(" ");
    if value.len() > OVERLAP {
        return Err(Error::Invalid);
    }
    Ok((!value.is_empty()).then_some(value))
}

pub(crate) struct Advance {
    pub chunks: Vec<String>,
    pub carry: String,
    pub consumed: usize,
    pub skipping: bool,
}
/// At most one input window and bounded overlap. A retained suffix contains at
/// least OVERLAP bytes at token boundaries, so every supported phrase survives
/// a chunk or work-turn boundary. Encoded long words fit native lexeme limits.
pub(crate) fn advance(text: &str, finished: bool, skipping: bool, carry: &str) -> Result<Advance> {
    if text.len() > WINDOW || carry.len() > OVERLAP + WORD_KEY + 2 {
        return Err(Error::Invalid);
    }
    let (tokens, consumed, skipping) = tokens(text, finished, skipping);
    let mut carry = carry.to_owned();
    let mut chunks = vec![];
    for token in tokens {
        if carry.len() + token.len() + usize::from(!carry.is_empty()) > CHUNK {
            chunks.push(carry.clone());
            carry = suffix(&carry);
        }
        if !carry.is_empty() {
            carry.push(' ');
        }
        carry.push_str(&token);
    }
    if finished && consumed == text.len() {
        if !carry.is_empty() {
            chunks.push(std::mem::take(&mut carry));
        }
    } else if carry.len() > OVERLAP + WORD_KEY + 2 {
        chunks.push(carry.clone());
        carry = suffix(&carry);
    }
    Ok(Advance {
        chunks,
        carry,
        consumed,
        skipping,
    })
}
fn suffix(text: &str) -> String {
    let mut keep_from = text.len().saturating_sub(OVERLAP);
    while !text.is_char_boundary(keep_from) {
        keep_from -= 1;
    }
    let begin = text[..keep_from].rfind(' ').map_or(0, |p| p + 1);
    text[begin..].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chunks(text: &str) -> Vec<String> {
        let mut result = vec![];
        let mut position = 0;
        let mut carry = String::new();
        let mut skipping = false;
        for _ in 0..10000 {
            if position >= text.len() {
                break;
            }
            let mut end = (position + WINDOW).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let turn = advance(&text[position..end], end == text.len(), skipping, &carry).unwrap();
            assert!(turn.consumed > 0);
            position += turn.consumed;
            carry = turn.carry;
            skipping = turn.skipping;
            result.extend(turn.chunks);
        }
        assert_eq!(position, text.len());
        assert!(carry.is_empty());
        result
    }
    fn matches(chunks: &[String], q: &str) -> bool {
        let query = format!(" {} ", unit(q).unwrap().unwrap());
        chunks.iter().any(|c| format!(" {c} ").contains(&query))
    }
    #[test]
    fn native_tokens_are_lossless_and_phrase_safety_accounts_for_database_limits() {
        for (left, right) in [
            ("दिन", "दन"),
            ("й", "и"),
            ("ά", "α"),
            ("ς", "σ"),
            ("東京", "京東"),
        ] {
            let left = unit(left).unwrap().unwrap();
            let right = unit(right).unwrap().unwrap();
            assert_ne!(native_unit(&left), native_unit(&right));
            for key in [native_unit(&left), native_unit(&right)] {
                assert!(key.bytes().all(|b| b.is_ascii_alphanumeric()));
                assert!(key.len() < 2048);
            }
        }
        let canonical = unit("café 東京 सभा 📦 arrived").unwrap().unwrap();
        let (native, safe) = native_document("account one", 2, &canonical);
        assert!(safe);
        assert!(native.starts_with(&native_scope("account one")));
        assert!(native
            .split_whitespace()
            .skip(1)
            .all(|word| word.starts_with("f2n")));
        for field in 0..WEIGHTS.len() {
            let encoded = field_unit(&canonical, field);
            assert_eq!(
                encoded.split_whitespace().count(),
                canonical.split_whitespace().count()
            );
            assert!(encoded
                .split_whitespace()
                .all(|word| word.starts_with(&format!("f{field}n"))));
            for other in 0..field {
                assert_ne!(encoded, field_unit(&canonical, other));
            }
        }
        assert_ne!(native_scope("account one"), native_scope("account two"));
        assert_eq!(
            native.split_whitespace().count(),
            canonical.split_whitespace().count() + 1
        );
        assert!(native_document("a", 3, &vec!["wrepeat"; 256].join(" ")).1);
        assert!(!native_document("a", 3, &vec!["wrepeat"; 257].join(" ")).1);
        let unique = (0..16382)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(native_document("a", 3, &unique).1);
        assert!(!native_document("a", 3, &format!("{unique} wlast")).1);
        assert!(native_unit(&unit(&"é".repeat(1000)).unwrap().unwrap()).len() < 2048);
    }
    #[test]
    fn late_phrases_repeated_words_and_boundaries_are_preserved() {
        let text = format!(
            "{}terminal needle {} 東京 会議",
            "garden ".repeat(18000),
            "résumé ".repeat(18000)
        );
        let indexed = chunks(&text);
        assert!(matches(&indexed, "garden terminal needle"));
        assert!(matches(&indexed, "resume resume resume"));
        assert!(matches(&indexed, "東京 会議"));
        assert!(!matches(&indexed, "garden needle"));
        assert!(!matches(&indexed, "terminal resume"));
        assert!(indexed
            .iter()
            .all(|c| c.len() <= CHUNK && c.split(' ').all(|t| t.len() < 2048)));
        // A phrase near the maximum query size crosses canonical chunk and raw
        // input-window boundaries, including thousands of repeated positions.
        let phrase = "a ".repeat(1900);
        let text = format!("{}{} z", "b ".repeat(WINDOW), phrase);
        assert!(matches(&chunks(&text), phrase.trim()));
    }
    #[test]
    fn long_words_hash_without_prefix_collisions_or_false_cross_gap_phrases() {
        let word = "résumé".repeat(200);
        let indexed = chunks(&format!("left {word} right"));
        assert!(matches(&indexed, &word));
        assert!(!matches(&indexed, "left right"));
        let key = unit(&word).unwrap().unwrap();
        assert!(key.starts_with('h'));
        assert!(!matches(&indexed, &key));
        let text = format!("left {} right", "q".repeat(WINDOW * 3));
        let indexed = chunks(&text);
        assert!(!matches(&indexed, "left right"));
        assert!(!matches(&indexed, "q"));
        assert!(!matches(&indexed, GAP));
    }
    #[test]
    fn canonical_rules_are_shared_for_addresses_accents_unicode_and_symbols() {
        let indexed = chunks("Maya <MAYA@EXAMPLE.INVALID> Café 東京 会議. Parcel 📦 arrived 💶 +5");
        for q in [
            "maya@example.invalid",
            "cafe",
            "cafe 東京",
            "parcel 📦 arrived",
            "💶 +5",
        ] {
            assert!(matches(&indexed, q), "{q}");
        }
        assert!(!matches(&indexed, "parcel arrived"));
        assert!(unit("...").unwrap().is_none());
        assert!(unit(&"x".repeat(4097)).is_err());
        assert!(unit("nul\0byte").is_err());
        assert_ne!(unit("दिन").unwrap(), unit("दन").unwrap());
        assert_ne!(unit("й").unwrap(), unit("и").unwrap());
        assert_ne!(unit("ά").unwrap(), unit("α").unwrap());
    }
}
