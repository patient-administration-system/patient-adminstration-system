//! HL7 v2 escape sequences.
//!
//! v2 reserves five characters as field/component/repetition/subcomponent
//! separators and the escape character itself. When a *value* needs to
//! contain one of those characters literally, the sender escapes it with the
//! sequence `\X\` where `X` identifies which delimiter is being escaped:
//!
//! | Escape | Means literal | ASCII |
//! |--------|---------------|-------|
//! | `\F\`  | field sep     | `|`   |
//! | `\S\`  | component sep | `^`   |
//! | `\T\`  | subcomp sep   | `&`   |
//! | `\R\`  | repetition    | `~`   |
//! | `\E\`  | escape char   | `\`   |
//!
//! This module provides the two functions that should be applied at the
//! *domain ↔ wire* boundary:
//!
//! - [`escape_value`] when serializing a domain string into a v2 field/
//!   component value (encoder side).
//! - [`unescape_value`] when reading a v2 field/component value back into a
//!   domain string (parser/mapping side).
//!
//! These deliberately do NOT touch the raw segment bytes or the
//! `Segment::fields` storage in [`crate::hl7v2::Segment`] — that storage is
//! still the literal wire bytes between delimiters. Callers in
//! [`crate::hl7v2::mapping`] apply the escapes when crossing the boundary.

use super::{COMPONENT_SEP, ESCAPE_CHAR, FIELD_SEP, REPETITION_SEP, SUBCOMPONENT_SEP};

/// Escape every reserved HL7 v2 delimiter character in `value`. The
/// escape-character backslash itself must be escaped *first* so that newly
/// introduced backslashes from the other replacements aren't re-escaped.
pub fn escape_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            // \E\ must come first conceptually; here we just emit it for
            // every literal backslash we see in the input.
            c if c == ESCAPE_CHAR => out.push_str("\\E\\"),
            c if c == FIELD_SEP => out.push_str("\\F\\"),
            c if c == COMPONENT_SEP => out.push_str("\\S\\"),
            c if c == SUBCOMPONENT_SEP => out.push_str("\\T\\"),
            c if c == REPETITION_SEP => out.push_str("\\R\\"),
            other => out.push(other),
        }
    }
    out
}

/// Inverse of [`escape_value`]. Unknown escape sequences (`\X\` where `X`
/// is not one of `F S T R E`) are passed through verbatim — this matches
/// the de-facto behavior of most v2 parsers and avoids silently dropping
/// vendor extensions like `\H\` / `\N\` / hex `\Xnn...\`.
pub fn unescape_value(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 2 < bytes.len() && bytes[i + 2] == b'\\' {
            let code = bytes[i + 1];
            let replacement = match code {
                b'F' => Some(FIELD_SEP),
                b'S' => Some(COMPONENT_SEP),
                b'T' => Some(SUBCOMPONENT_SEP),
                b'R' => Some(REPETITION_SEP),
                b'E' => Some(ESCAPE_CHAR),
                _ => None,
            };
            if let Some(c) = replacement {
                out.push(c);
                i += 3;
                continue;
            }
            // Unknown escape — emit the sequence as-is and advance.
            out.push('\\');
            out.push(code as char);
            out.push('\\');
            i += 3;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_preserves_normal_strings() {
        assert_eq!(escape_value("Doe"), "Doe");
        assert_eq!(escape_value("Jane Marie"), "Jane Marie");
    }

    #[test]
    fn test_escape_each_delimiter() {
        assert_eq!(escape_value("a|b"), "a\\F\\b");
        assert_eq!(escape_value("a^b"), "a\\S\\b");
        assert_eq!(escape_value("a&b"), "a\\T\\b");
        assert_eq!(escape_value("a~b"), "a\\R\\b");
        assert_eq!(escape_value("a\\b"), "a\\E\\b");
    }

    #[test]
    fn test_escape_handles_multiple_delimiters_in_one_value() {
        assert_eq!(escape_value("O^Brien-Jones"), "O\\S\\Brien-Jones");
        assert_eq!(escape_value("a|b^c&d"), "a\\F\\b\\S\\c\\T\\d");
    }

    #[test]
    fn test_unescape_roundtrip() {
        let cases = [
            "Doe",
            "O^Brien",
            "a|b",
            "a&b",
            "a~b",
            "back\\slash",
            "all|of^the~separators&plus\\escape",
        ];
        for c in cases {
            let escaped = escape_value(c);
            let unescaped = unescape_value(&escaped);
            assert_eq!(unescaped, c, "round-trip failed for {c:?}");
        }
    }

    #[test]
    fn test_unescape_unknown_sequence_passes_through() {
        // Vendor extensions like \H\ / \N\ / hex \X41\ should not be lost.
        assert_eq!(unescape_value("text\\H\\more"), "text\\H\\more");
    }

    #[test]
    fn test_unescape_does_not_match_lone_backslash() {
        // A backslash that's not part of a complete \X\ sequence is literal.
        assert_eq!(unescape_value("path\\to\\file"), "path\\to\\file");
    }

    #[test]
    fn test_escape_does_not_double_escape_existing_backslash() {
        // Input is "a\F\b" (with a literal backslash). Escaping should treat
        // each backslash as an escape-char to escape, NOT recognize the
        // existing "\F\" as a delimiter token.
        let escaped = escape_value("a\\F\\b");
        assert_eq!(escaped, "a\\E\\F\\E\\b");
        let back = unescape_value(&escaped);
        assert_eq!(back, "a\\F\\b");
    }
}
