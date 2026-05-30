//! HL7 v2 message parsing, encoding, and mapping to PAS domain types.
//!
//! v0.3 first cut: pipe-delimited ADT messages with the standard delimiter
//! set `|^~\&`. Supported segments: `MSH`, `EVN`, `PID`, `PV1`. Supported
//! message types for round-trip mapping: `ADT^A01` (admit), `ADT^A03`
//! (discharge), `ADT^A28` (add person info). Other message types parse to
//! the generic [`Message`] structure but are not mapped to domain objects.
//!
//! Wire format primer:
//!
//! ```text
//! MSH|^~\&|SENDING_APP|FAC|RECV_APP|FAC|20260523120000||ADT^A01|MSG001|P|2.5
//! EVN|A01|20260523120000
//! PID|1||MRN-001^^^FAC^MR||Doe^Jane^Marie||19900115|F|||123 Elm^^Springfield^IL^62701^US||(555)555-0100
//! PV1|1|I|WARD1^ROOM1^BED1|||||||||||||||VISIT001
//! ```
//!
//! - Segments separated by `\r` (the HL7 convention; this module also accepts
//!   `\n` and `\r\n` on the read path).
//! - Fields separated by `|`.
//! - Components separated by `^`.
//! - Subcomponents separated by `&`.
//! - Repetitions separated by `~`.
//!
//! Field numbering follows the HL7 convention: `PID-5` is the *fifth* field of
//! the segment, which means `segment.field(5)` returns it. For MSH, field 1
//! is the field separator itself; field 2 is the encoding characters
//! (`^~\&`).

pub mod ack;
pub mod batch;
pub mod encoder;
pub mod escape;
pub mod listener;
pub mod mapping;
pub mod mllp;
pub mod parser;

pub use ack::{AckCode, ack};
pub use batch::{Batch, MAX_BATCH_MESSAGES, encode_batch_ack, looks_like_batch, parse_batch};
pub use encoder::encode_message;
pub use escape::{escape_value, unescape_value};
pub use listener::MllpServer;
pub use mapping::{
    DftP03Item, DftP03Message, MfnM02Item, MfnM02Message, MfnM05Item, MfnM05Message, SiuMessage,
    encode_adt_a01, encode_adt_a02, encode_adt_a03, encode_adt_a04, encode_adt_a05, encode_adt_a06,
    encode_adt_a08, encode_adt_a11, encode_adt_a12, encode_adt_a13, encode_adt_a21, encode_adt_a22,
    encode_adt_a23, encode_adt_a28, encode_adt_a38, encode_adt_a40, encode_dft_p03, encode_mfn_m02,
    encode_mfn_m05, encode_siu_s12, encode_siu_s13, encode_siu_s14, encode_siu_s15, message_type,
    parse_dft_p03, parse_merge_source_mrn, parse_mfn_m02, parse_mfn_m05, parse_siu,
    patient_from_pid, pid_from_patient,
};
pub use parser::parse_message;

use serde::{Deserialize, Serialize};

/// Standard HL7 delimiter set. Custom delimiters are out of scope — every
/// message this module produces uses `|^~\&`.
pub const FIELD_SEP: char = '|';
pub const COMPONENT_SEP: char = '^';
pub const REPETITION_SEP: char = '~';
pub const ESCAPE_CHAR: char = '\\';
pub const SUBCOMPONENT_SEP: char = '&';

/// One parsed HL7 v2 segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    /// Three-letter segment name, e.g. `"MSH"`, `"PID"`.
    pub name: String,
    /// Raw field strings, including any embedded `^` / `~` / `&`. For the
    /// `MSH` segment, `fields[0]` is the field separator itself (`"|"`) and
    /// `fields[1]` is the encoding characters (`"^~\&"`), so `field(n)`
    /// matches HL7's 1-indexed numbering for every `n`.
    pub fields: Vec<String>,
}

impl Segment {
    /// Read field N (1-indexed, HL7-style). Returns `""` if the field is
    /// missing or beyond the end of the segment.
    pub fn field(&self, n: usize) -> &str {
        if n == 0 {
            return "";
        }
        // Field 1 = fields[0] (the segment-name field is dropped on parse).
        self.fields.get(n - 1).map(String::as_str).unwrap_or("")
    }

    /// Read component C (1-indexed) of field N (1-indexed). Returns `""` if
    /// either index is out of range.
    pub fn component(&self, field_n: usize, component_c: usize) -> &str {
        if component_c == 0 {
            return "";
        }
        let f = self.field(field_n);
        f.split(COMPONENT_SEP).nth(component_c - 1).unwrap_or("")
    }
}

/// A parsed HL7 v2 message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub segments: Vec<Segment>,
}

impl Message {
    /// Find the first segment with the given three-letter name.
    pub fn segment(&self, name: &str) -> Option<&Segment> {
        self.segments.iter().find(|s| s.name == name)
    }

    /// Iterator over every segment with the given name.
    pub fn all_segments<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Segment> + 'a {
        self.segments.iter().filter(move |s| s.name == name)
    }
}
