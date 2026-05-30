//! HL7 v2 message parser (pipe-delimited, standard delimiters only).

use super::{
    COMPONENT_SEP, ESCAPE_CHAR, FIELD_SEP, Message, REPETITION_SEP, SUBCOMPONENT_SEP, Segment,
};
use crate::{Error, Result};

/// Parse a v2 message string into a structured [`Message`].
///
/// Tolerates `\r`, `\n`, and `\r\n` as segment terminators. Requires the
/// first segment to be `MSH` with standard delimiters `|^~\&`. Returns a
/// [`Error::Validation`] error on any structural problem.
pub fn parse_message(raw: &str) -> Result<Message> {
    let normalized = raw.replace("\r\n", "\r").replace('\n', "\r");
    let mut segment_strs: Vec<&str> = normalized
        .split('\r')
        .map(str::trim_end)
        .filter(|s| !s.is_empty())
        .collect();

    let first = segment_strs
        .first()
        .copied()
        .ok_or_else(|| Error::validation("HL7v2: empty message"))?;
    if !first.starts_with("MSH") {
        return Err(Error::validation(format!(
            "HL7v2: first segment must be MSH, got {:?}",
            first.chars().take(3).collect::<String>()
        )));
    }
    if first.len() < 8 {
        return Err(Error::validation("HL7v2: MSH segment is too short"));
    }
    // Verify standard delimiters: MSH-1 is the byte right after "MSH" (field
    // separator); MSH-2 is the next four characters (encoding chars).
    let bytes = first.as_bytes();
    if bytes[3] as char != FIELD_SEP {
        return Err(Error::validation(format!(
            "HL7v2: field separator must be '|' (got {:?})",
            bytes[3] as char
        )));
    }
    let encoding = &first[4..8];
    let mut chars = encoding.chars();
    let c1 = chars.next().unwrap_or_default();
    let c2 = chars.next().unwrap_or_default();
    let c3 = chars.next().unwrap_or_default();
    let c4 = chars.next().unwrap_or_default();
    if (c1, c2, c3, c4) != (COMPONENT_SEP, REPETITION_SEP, ESCAPE_CHAR, SUBCOMPONENT_SEP) {
        return Err(Error::validation(format!(
            "HL7v2: encoding characters must be \"^~\\&\", got {encoding:?}"
        )));
    }

    // Build the MSH segment specially. HL7 convention:
    //   MSH-1 = the field separator itself ("|").
    //   MSH-2 = the encoding characters ("^~\&").
    // So `fields[0]` holds the field separator and `fields[1]` holds the
    // encoding characters; `field(n)` then matches the HL7 1-indexed
    // numbering for every n.
    let mut segments = Vec::with_capacity(segment_strs.len());
    let after_encoding = &first[8..]; // starts with "|<MSH-3>|<MSH-4>|..."
    let rest = after_encoding
        .strip_prefix(FIELD_SEP)
        .unwrap_or(after_encoding);
    let mut msh_fields = vec![FIELD_SEP.to_string(), encoding.to_string()];
    msh_fields.extend(rest.split(FIELD_SEP).map(String::from));
    segments.push(Segment {
        name: "MSH".into(),
        fields: msh_fields,
    });

    // Parse remaining segments uniformly.
    segment_strs.remove(0);
    for s in segment_strs {
        if s.len() < 3 {
            return Err(Error::validation(format!(
                "HL7v2: segment too short: {s:?}"
            )));
        }
        let (name, rest) = s.split_at(3);
        // The 4th char must be the field separator (or the segment must end).
        let body = if rest.is_empty() {
            ""
        } else if rest.starts_with(FIELD_SEP) {
            &rest[1..]
        } else {
            return Err(Error::validation(format!(
                "HL7v2: segment {name:?} not followed by field separator"
            )));
        };
        let fields: Vec<String> = body.split(FIELD_SEP).map(String::from).collect();
        segments.push(Segment {
            name: name.to_string(),
            fields,
        });
    }

    Ok(Message { segments })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A01: &str = "MSH|^~\\&|SENDAPP|FAC|RECVAPP|FAC|20260523120000||ADT^A01|MSG001|P|2.5\r\
EVN|A01|20260523120000\r\
PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F|||123 Elm^^Springfield^IL^62701^US||(555)555-0100\r\
PV1|1|I|WARD1^ROOM1^BED1|||||||||||||||VISIT001\r";

    #[test]
    fn test_parse_segment_names() {
        let m = parse_message(A01).expect("parse");
        let names: Vec<&str> = m.segments.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["MSH", "EVN", "PID", "PV1"]);
    }

    #[test]
    fn test_parse_msh_keeps_encoding_in_field_2() {
        let m = parse_message(A01).expect("parse");
        let msh = m.segment("MSH").expect("MSH");
        // MSH-2 is the encoding characters.
        assert_eq!(msh.field(2), "^~\\&");
        // MSH-3 is the sending app.
        assert_eq!(msh.field(3), "SENDAPP");
        // MSH-9 is the message type.
        assert_eq!(msh.field(9), "ADT^A01");
        assert_eq!(msh.component(9, 1), "ADT");
        assert_eq!(msh.component(9, 2), "A01");
    }

    #[test]
    fn test_parse_pid_components() {
        let m = parse_message(A01).expect("parse");
        let pid = m.segment("PID").expect("PID");
        assert_eq!(pid.field(1), "1");
        assert_eq!(pid.field(3), "MRN-001^^^FAC^MR");
        assert_eq!(pid.component(3, 1), "MRN-001");
        assert_eq!(pid.component(3, 5), "MR");
        assert_eq!(pid.field(5), "Doe^Jane^Marie");
        assert_eq!(pid.component(5, 1), "Doe");
        assert_eq!(pid.component(5, 2), "Jane");
        assert_eq!(pid.component(5, 3), "Marie");
        assert_eq!(pid.field(7), "19900115");
        assert_eq!(pid.field(8), "F");
        assert_eq!(pid.component(11, 3), "Springfield");
        assert_eq!(pid.field(13), "(555)555-0100");
    }

    #[test]
    fn test_parse_pv1_class_and_location() {
        let m = parse_message(A01).expect("parse");
        let pv1 = m.segment("PV1").expect("PV1");
        assert_eq!(pv1.field(2), "I");
        assert_eq!(pv1.component(3, 1), "WARD1");
        assert_eq!(pv1.component(3, 2), "ROOM1");
        assert_eq!(pv1.component(3, 3), "BED1");
    }

    #[test]
    fn test_parse_accepts_lf_line_endings() {
        let lf = A01.replace('\r', "\n");
        let m = parse_message(&lf).expect("parse LF");
        assert_eq!(m.segments.len(), 4);
    }

    #[test]
    fn test_parse_rejects_missing_msh() {
        let bad = "PID|1||X||Doe^Jane||19900115|F\r";
        let err = parse_message(bad).expect_err("must reject");
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn test_parse_rejects_non_standard_delimiters() {
        let bad = "MSH#$%@&|X|Y\r";
        assert!(matches!(parse_message(bad), Err(Error::Validation(_))));
    }
}
