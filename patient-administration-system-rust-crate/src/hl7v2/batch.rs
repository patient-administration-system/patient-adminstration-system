//! HL7 v2 batch envelope (FHS / BHS / BTS / FTS).
//!
//! The HL7 v2 batch protocol wraps many messages into one transmission for
//! bulk send (end-of-day batches, backfill, system migrations). Two
//! envelopes:
//!
//! ```text
//! FHS|^~\&|...           ← file header   (optional, outer wrapper)
//!   BHS|^~\&|...         ← batch header  (required if there is a batch)
//!     MSH|^~\&|...
//!     [segments]
//!     MSH|^~\&|...
//!     [segments]
//!     ...
//!   BTS|<msg_count>      ← batch trailer (optional; count is informational)
//! FTS|<batch_count>      ← file trailer  (optional)
//! ```
//!
//! v0.6 first cut: one batch per request. A request may carry the file
//! envelope `FHS`/`FTS` around a single `BHS`/`BTS` batch (the most common
//! shape in the wild) or just a bare `BHS`/`BTS` batch. Multi-batch files
//! are *not* supported — every `BHS` after the first returns
//! `Error::Validation`.

use super::{
    COMPONENT_SEP, ESCAPE_CHAR, FIELD_SEP, Message, REPETITION_SEP, SUBCOMPONENT_SEP, Segment,
};
use crate::{Error, Result};

/// Hard cap on how many `MSH` messages one batch may contain. Anything
/// larger returns `Error::Validation` so the batch endpoint can return a
/// single `AR` rather than spending unbounded effort.
pub const MAX_BATCH_MESSAGES: usize = 1000;

/// One parsed HL7 v2 batch transmission. `fhs`/`fts` are present only when
/// the sender wrapped the batch in a file envelope. `bhs`/`bts` are
/// present whenever the sender used the batch envelope (every real batch
/// does, but the parser will also accept a bare list of `MSH` messages
/// for convenience and surface that as `bhs = None`, `bts = None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub fhs: Option<Segment>,
    pub bhs: Option<Segment>,
    pub messages: Vec<Message>,
    pub bts: Option<Segment>,
    pub fts: Option<Segment>,
}

/// Parse one batch transmission. Recognises `FHS`/`BHS`/`MSH`/`BTS`/`FTS`
/// segment headers; everything else is treated as a body segment that
/// attaches to the current `MSH` message. Multiple `MSH` headers split
/// the body into successive messages.
///
/// Errors:
/// - empty input or no `MSH`/`BHS`/`FHS` → `Error::Validation`.
/// - non-standard delimiters on any header segment → `Error::Validation`.
/// - more than one `BHS` (multi-batch files) → `Error::Validation`.
/// - more than `MAX_BATCH_MESSAGES` messages → `Error::Validation`.
pub fn parse_batch(raw: &str) -> Result<Batch> {
    let normalized = raw.replace("\r\n", "\r").replace('\n', "\r");
    let segment_strs: Vec<&str> = normalized
        .split('\r')
        .map(str::trim_end)
        .filter(|s| !s.is_empty())
        .collect();
    if segment_strs.is_empty() {
        return Err(Error::validation("HL7v2 batch: empty input"));
    }

    let mut fhs: Option<Segment> = None;
    let mut bhs: Option<Segment> = None;
    let mut bts: Option<Segment> = None;
    let mut fts: Option<Segment> = None;
    let mut messages: Vec<Message> = Vec::new();
    // Segments accumulated for the current in-flight message. Flushed when
    // we hit the next MSH, BTS, FTS, BHS, or end of input.
    let mut current: Option<Vec<Segment>> = None;

    for s in segment_strs {
        if s.len() < 3 {
            return Err(Error::validation(format!(
                "HL7v2 batch: segment too short: {s:?}"
            )));
        }
        let name = &s[..3];
        match name {
            "FHS" => {
                if fhs.is_some() {
                    return Err(Error::validation(
                        "HL7v2 batch: more than one FHS segment is not supported",
                    ));
                }
                if let Some(prev) = current.take() {
                    messages.push(Message { segments: prev });
                }
                fhs = Some(parse_header_segment("FHS", s)?);
            }
            "BHS" => {
                if bhs.is_some() {
                    return Err(Error::validation(
                        "HL7v2 batch: more than one BHS in a single transmission is not supported \
                         (multi-batch files not implemented)",
                    ));
                }
                if let Some(prev) = current.take() {
                    messages.push(Message { segments: prev });
                }
                bhs = Some(parse_header_segment("BHS", s)?);
            }
            "MSH" => {
                if let Some(prev) = current.take() {
                    messages.push(Message { segments: prev });
                    if messages.len() > MAX_BATCH_MESSAGES {
                        return Err(Error::validation(format!(
                            "HL7v2 batch: more than {MAX_BATCH_MESSAGES} messages",
                        )));
                    }
                }
                current = Some(vec![parse_header_segment("MSH", s)?]);
            }
            "BTS" => {
                if let Some(prev) = current.take() {
                    messages.push(Message { segments: prev });
                }
                bts = Some(parse_plain_segment(s));
            }
            "FTS" => {
                if let Some(prev) = current.take() {
                    messages.push(Message { segments: prev });
                }
                fts = Some(parse_plain_segment(s));
            }
            _ => {
                // Ordinary body segment (EVN, PID, PV1, ...). Must belong to a
                // current MSH.
                let body = parse_body_segment(name, s)?;
                match current.as_mut() {
                    Some(segs) => segs.push(body),
                    None => {
                        return Err(Error::validation(format!(
                            "HL7v2 batch: segment {name:?} appears before any MSH"
                        )));
                    }
                }
            }
        }
    }

    if let Some(prev) = current.take() {
        messages.push(Message { segments: prev });
    }
    if messages.len() > MAX_BATCH_MESSAGES {
        return Err(Error::validation(format!(
            "HL7v2 batch: more than {MAX_BATCH_MESSAGES} messages",
        )));
    }
    if messages.is_empty() && bhs.is_none() && fhs.is_none() {
        return Err(Error::validation(
            "HL7v2 batch: input must contain at least one MSH, BHS, or FHS segment",
        ));
    }
    Ok(Batch {
        fhs,
        bhs,
        messages,
        bts,
        fts,
    })
}

/// Parse a header segment (MSH, FHS, or BHS). They share the same shape:
/// 3-letter name, field separator, encoding characters, then 1-indexed
/// fields. The standard delimiter set is required.
fn parse_header_segment(expected_name: &str, s: &str) -> Result<Segment> {
    if s.len() < 8 {
        return Err(Error::validation(format!(
            "HL7v2 batch: {expected_name} segment is too short"
        )));
    }
    let bytes = s.as_bytes();
    if bytes[3] as char != FIELD_SEP {
        return Err(Error::validation(format!(
            "HL7v2 batch: {expected_name} field separator must be '|' (got {:?})",
            bytes[3] as char
        )));
    }
    let encoding = &s[4..8];
    let mut chars = encoding.chars();
    let c1 = chars.next().unwrap_or_default();
    let c2 = chars.next().unwrap_or_default();
    let c3 = chars.next().unwrap_or_default();
    let c4 = chars.next().unwrap_or_default();
    if (c1, c2, c3, c4) != (COMPONENT_SEP, REPETITION_SEP, ESCAPE_CHAR, SUBCOMPONENT_SEP) {
        return Err(Error::validation(format!(
            "HL7v2 batch: {expected_name} encoding characters must be \"^~\\&\", got {encoding:?}"
        )));
    }
    // Same layout as parser::parse_message for MSH: fields[0] = "|",
    // fields[1] = encoding chars, fields[2..] = the rest.
    let after_encoding = &s[8..];
    let rest = after_encoding
        .strip_prefix(FIELD_SEP)
        .unwrap_or(after_encoding);
    let mut fields = vec![FIELD_SEP.to_string(), encoding.to_string()];
    fields.extend(rest.split(FIELD_SEP).map(String::from));
    Ok(Segment {
        name: expected_name.to_string(),
        fields,
    })
}

/// Parse a non-header body segment (EVN, PID, PV1, ...). Same shape as in
/// `parser::parse_message`.
fn parse_body_segment(name: &str, s: &str) -> Result<Segment> {
    let rest = &s[3..];
    let body = if rest.is_empty() {
        ""
    } else if rest.starts_with(FIELD_SEP) {
        &rest[1..]
    } else {
        return Err(Error::validation(format!(
            "HL7v2 batch: segment {name:?} not followed by field separator"
        )));
    };
    Ok(Segment {
        name: name.to_string(),
        fields: body.split(FIELD_SEP).map(String::from).collect(),
    })
}

/// Parse a "plain" trailer segment (BTS, FTS). These don't carry encoding
/// characters; their one practical field is the count of contained items.
/// Infallible at this layer — the BTS/FTS body is always a free-form
/// pipe-separated list and we don't enforce the count.
fn parse_plain_segment(s: &str) -> Segment {
    let name = s[..3].to_string();
    let rest = &s[3..];
    let body = rest.strip_prefix(FIELD_SEP).unwrap_or("");
    Segment {
        name,
        fields: body.split(FIELD_SEP).map(String::from).collect(),
    }
}

/// Build a batch ACK envelope. Wraps the per-message ACK bodies inside
/// one `BHS` / `BTS` pair so the sender can read each MSA on the same
/// connection.
///
/// The shape matches what most HL7 v2 receivers emit:
///
/// ```text
/// BHS|^~\&|PAS|FAC|<sender>|FAC|<now>||<batch_ctl_id>|P|2.5
/// <ack-1>
/// <ack-2>
/// ...
/// BTS|<n>
/// ```
///
/// Each per-message ACK is already a complete `MSH ... MSA` block produced
/// by [`crate::hl7v2::ack::ack`], so the function just stitches them
/// together with the envelope.
pub fn encode_batch_ack(
    sending_app: &str,
    receiving_app: &str,
    batch_control_id: &str,
    per_message_acks: &[String],
) -> String {
    let now = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let mut out =
        String::with_capacity(256 + per_message_acks.iter().map(|s| s.len()).sum::<usize>());
    out.push_str(&format!(
        "BHS|^~\\&|{sending_app}|FAC|{receiving_app}|FAC|{now}||{batch_control_id}|P|2.5\r"
    ));
    for ack in per_message_acks {
        // Each ack ends with `\r` already; preserve as-is so per-message MSH
        // and MSA segments are individually terminated.
        out.push_str(ack);
    }
    out.push_str(&format!("BTS|{}\r", per_message_acks.len()));
    out
}

/// Returns `true` if `payload` looks like a batch (first segment is `FHS`
/// or `BHS`). The MLLP listener uses this to pick the right route.
pub fn looks_like_batch(payload: &str) -> bool {
    let trimmed = payload.trim_start();
    trimmed.starts_with("FHS|") || trimmed.starts_with("BHS|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hl7v2::AckCode;
    use crate::hl7v2::ack;

    const A28: &str = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A28|MSG-1|P|2.5\r\
EVN|A28|20260523120000\r\
PID|1||MRN-1^^^FAC^MR||Smith^John||19800101|M\r";

    const A28_B: &str = "MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120100||ADT^A28|MSG-2|P|2.5\r\
EVN|A28|20260523120100\r\
PID|1||MRN-2^^^FAC^MR||Jones^Mary||19850515|F\r";

    fn wrap_batch(messages: &[&str]) -> String {
        let mut s = String::from("BHS|^~\\&|EMR|FAC|PAS|FAC|20260523120000||BATCH-001|P|2.5\r");
        for m in messages {
            s.push_str(m);
        }
        s.push_str(&format!("BTS|{}\r", messages.len()));
        s
    }

    #[test]
    fn test_parse_batch_with_two_messages() {
        let body = wrap_batch(&[A28, A28_B]);
        let b = parse_batch(&body).expect("parse");
        assert!(b.bhs.is_some(), "BHS must be parsed");
        assert!(b.bts.is_some(), "BTS must be parsed");
        assert_eq!(b.messages.len(), 2);
        assert_eq!(b.messages[0].segments[0].name, "MSH");
        assert_eq!(
            b.messages[0].segment("PID").unwrap().component(3, 1),
            "MRN-1"
        );
        assert_eq!(
            b.messages[1].segment("PID").unwrap().component(3, 1),
            "MRN-2"
        );
        let bts = b.bts.as_ref().unwrap();
        assert_eq!(bts.field(1), "2");
    }

    #[test]
    fn test_parse_batch_accepts_file_envelope() {
        let body = format!(
            "FHS|^~\\&|EMR|FAC|PAS|FAC|20260523120000||FILE-001|P|2.5\r\
{}FTS|1\r",
            wrap_batch(&[A28])
        );
        let b = parse_batch(&body).expect("parse FHS+BHS+FTS");
        assert!(b.fhs.is_some());
        assert!(b.bhs.is_some());
        assert!(b.bts.is_some());
        assert!(b.fts.is_some());
        assert_eq!(b.messages.len(), 1);
    }

    #[test]
    fn test_parse_batch_accepts_bare_messages_without_envelope() {
        // Just an MSH list without BHS — accept it (some senders are lazy).
        let body = format!("{A28}{A28_B}");
        let b = parse_batch(&body).expect("parse bare MSH list");
        assert!(b.bhs.is_none());
        assert_eq!(b.messages.len(), 2);
    }

    #[test]
    fn test_parse_batch_rejects_empty_input() {
        assert!(matches!(parse_batch(""), Err(Error::Validation(_))));
        assert!(matches!(parse_batch("\r\n\r\n"), Err(Error::Validation(_))));
    }

    #[test]
    fn test_parse_batch_rejects_body_segment_before_any_msh() {
        let body = "BHS|^~\\&|E|F|R|F|20260523120000||B-1|P|2.5\r\
PID|1||MRN-X^^^FAC^MR||Nobody^A||19800101|F\r\
BTS|0\r";
        assert!(matches!(parse_batch(body), Err(Error::Validation(_))));
    }

    #[test]
    fn test_parse_batch_rejects_second_bhs() {
        let body = format!(
            "{}BHS|^~\\&|EMR|FAC|PAS|FAC|20260523120100||BATCH-002|P|2.5\r{A28}BTS|1\r",
            wrap_batch(&[A28]),
        );
        let err = parse_batch(&body).expect_err("two BHS must fail");
        match err {
            Error::Validation(msg) => assert!(msg.contains("BHS"), "diag: {msg}"),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_batch_rejects_oversize_input() {
        let mut body = String::from("BHS|^~\\&|EMR|FAC|PAS|FAC|20260523120000||B-1|P|2.5\r");
        for _ in 0..(MAX_BATCH_MESSAGES + 1) {
            body.push_str(A28);
        }
        body.push_str("BTS|0\r");
        let err = parse_batch(&body).expect_err("over limit must fail");
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn test_encode_batch_ack_wraps_per_message_acks() {
        let a1 = ack("PAS", "EMR", "MSG-1", AckCode::Accept, None);
        let a2 = ack("PAS", "EMR", "MSG-2", AckCode::AppError, Some("bad PID"));
        let out = encode_batch_ack("PAS", "EMR", "ACK-BATCH-1", &[a1.clone(), a2.clone()]);
        assert!(out.starts_with("BHS|^~\\&|PAS|FAC|EMR|FAC|"));
        assert!(out.contains("|ACK-BATCH-1|P|2.5\r"));
        assert!(out.contains(&a1));
        assert!(out.contains(&a2));
        assert!(out.contains("MSA|AA|MSG-1"));
        assert!(out.contains("MSA|AE|MSG-2|bad PID"));
        assert!(out.ends_with("BTS|2\r"));
    }

    #[test]
    fn test_looks_like_batch_detects_bhs_and_fhs() {
        assert!(looks_like_batch("BHS|^~\\&|E|F|R|F|datetime||X|P|2.5\r"));
        assert!(looks_like_batch("  FHS|^~\\&|E|F|R|F|datetime||X|P|2.5\r"));
        assert!(!looks_like_batch(
            "MSH|^~\\&|E|F|R|F|datetime||ADT^A01|X|P|2.5\r"
        ));
        assert!(!looks_like_batch("garbage"));
        assert!(!looks_like_batch(""));
    }
}
