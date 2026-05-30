//! HL7 v2 message encoder (pipe-delimited, standard delimiters only).

use super::{FIELD_SEP, Message, Segment};

/// Encode a parsed [`Message`] back to a wire string.
///
/// Segments are joined by `\r` (the HL7 convention). For the MSH segment,
/// `fields[0]` is rendered as the encoding-characters block (`^~\&`) and
/// the canonical field separator `|` is emitted right after `MSH`.
///
/// The output is byte-identical to the input for messages that were parsed
/// by this module's [`crate::hl7v2::parser::parse_message`] when callers
/// haven't mutated the message — see the round-trip test in this module.
pub fn encode_message(m: &Message) -> String {
    let mut parts = Vec::with_capacity(m.segments.len());
    for seg in &m.segments {
        parts.push(encode_segment(seg));
    }
    let mut out = parts.join("\r");
    out.push('\r');
    out
}

fn encode_segment(seg: &Segment) -> String {
    if seg.name == "MSH" {
        // fields[0] = the field separator itself (typically "|").
        // fields[1] = the encoding characters (typically "^~\&").
        let sep = seg.fields.first().map(String::as_str).unwrap_or("|");
        let encoding = seg.fields.get(1).map(String::as_str).unwrap_or("^~\\&");
        let rest: Vec<&str> = seg.fields.iter().skip(2).map(String::as_str).collect();
        let mut out = String::with_capacity(64);
        out.push_str("MSH");
        out.push_str(sep);
        out.push_str(encoding);
        for f in &rest {
            out.push_str(sep);
            out.push_str(f);
        }
        out
    } else {
        let mut out = String::with_capacity(64);
        out.push_str(&seg.name);
        out.push(FIELD_SEP);
        let joined: Vec<&str> = seg.fields.iter().map(String::as_str).collect();
        out.push_str(&joined.join(&FIELD_SEP.to_string()));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hl7v2::parse_message;

    const A01: &str = "MSH|^~\\&|SENDAPP|FAC|RECVAPP|FAC|20260523120000||ADT^A01|MSG001|P|2.5\r\
EVN|A01|20260523120000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F|||123 Elm^^Springfield^IL^62701^US||(555)555-0100\r\
PV1|1|I|WARD1^ROOM1^BED1|||||||||||||||VISIT001\r";

    #[test]
    fn test_encode_roundtrip_a01() {
        let parsed = parse_message(A01).expect("parse");
        let out = encode_message(&parsed);
        assert_eq!(out, A01, "round-trip must be byte-identical");
    }

    #[test]
    fn test_encode_empty_msh_fields_preserve_pipes() {
        let parsed = parse_message(A01).expect("parse");
        let out = encode_message(&parsed);
        // MSH-8 (security) is empty in the source; the leading pipes must
        // survive so field 9 stays at position 9.
        assert!(out.contains("|20260523120000||ADT^A01|"));
    }
}
