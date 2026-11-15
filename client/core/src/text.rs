//! Text rules shared with the JavaScript app. Lengths there are counted in UTF-16 units, and a
//! name cut at a different place on each side would make two apps disagree about the same data.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};

pub const MAX_NAME_LENGTH: usize = 40;

/// The first `max` UTF-16 units of `text`, never splitting a character in two.
pub fn truncate_utf16(text: &str, max: usize) -> &str {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        units += character.len_utf16();
        if units > max {
            return &text[..index];
        }
    }
    text
}

pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// Runs of whitespace become one space, the ends are trimmed and the result is cut to `max`.
pub fn collapse_spaces(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_utf16(&collapsed, max).to_owned()
}

/// A name as every store keeps it. Empty input gives the fallback.
pub fn clean_name(name: &str, fallback: &str) -> String {
    let clean = collapse_spaces(name, MAX_NAME_LENGTH);
    if clean.is_empty() { fallback.to_owned() } else { clean }
}

pub fn encode_base64(data: &[u8]) -> String {
    STANDARD.encode(data)
}

pub fn encode_base64_url(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

/// Decodes the way Node's `Buffer.from(text, 'base64')` does: both alphabets, padding optional,
/// anything else skipped. What the other side sends was produced by that function's pair.
pub fn decode_base64(text: &str) -> Vec<u8> {
    let mut filtered = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' => filtered.push(character),
            '-' => filtered.push('+'),
            '_' => filtered.push('/'),
            '=' => break,
            _ => {}
        }
    }
    // A lone trailing symbol carries no whole byte, and Node drops it.
    if filtered.len() % 4 == 1 {
        filtered.pop();
    }
    let lenient = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_allow_trailing_bits(true)
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    lenient.decode(filtered.as_bytes()).unwrap_or_default()
}

/// Strict decoding for data this app wrote itself.
pub fn decode_base64_strict(text: &str) -> Option<Vec<u8>> {
    STANDARD.decode(text.as_bytes()).or_else(|_| STANDARD_NO_PAD.decode(text.as_bytes())).ok()
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the system random source is unavailable");
    bytes
}

pub fn random_hex(bytes: usize) -> String {
    let mut data = vec![0u8; bytes];
    getrandom::fill(&mut data).expect("the system random source is unavailable");
    hex::encode(data)
}

pub fn new_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_collapsed_trimmed_and_cut() {
        assert_eq!(clean_name("  Ana   Maria \n", "x"), "Ana Maria");
        assert_eq!(clean_name("   ", "Contato"), "Contato");
        assert_eq!(utf16_len(&clean_name(&"a".repeat(80), "x")), 40);
    }

    #[test]
    fn cutting_counts_utf16_units_and_keeps_characters_whole() {
        // An emoji outside the basic plane takes two units.
        assert_eq!(truncate_utf16("ab😀cd", 3), "ab");
        assert_eq!(truncate_utf16("ab😀cd", 4), "ab😀");
        assert_eq!(truncate_utf16("ação", 10), "ação");
    }

    #[test]
    fn base64_is_read_like_node_reads_it() {
        assert_eq!(decode_base64("aGVsbG8="), b"hello");
        assert_eq!(decode_base64("aGVsbG8"), b"hello");
        assert_eq!(decode_base64("aGV\nsbG8=\n"), b"hello");
        assert_eq!(decode_base64("-_-_"), decode_base64("+/+/"));
        assert_eq!(decode_base64("!!!"), b"");
    }
}
