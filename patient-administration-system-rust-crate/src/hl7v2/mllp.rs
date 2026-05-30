//! MLLP (Minimal Lower Layer Protocol) framing primitives for HL7 v2.
//!
//! Every MLLP frame on the wire looks like:
//!
//! ```text
//!   <SB><payload><EB><CR>
//!   SB = 0x0B (vertical tab)
//!   EB = 0x1C (file separator)
//!   CR = 0x0D
//! ```
//!
//! This module provides the byte-level read/write helpers; the higher-level
//! TCP listener that uses them lives in [`crate::hl7v2::listener`].

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Error, Result};

/// Start-of-block byte.
pub const SB: u8 = 0x0B;
/// End-of-block byte.
pub const EB: u8 = 0x1C;
/// Carriage return (the byte that follows EB).
pub const CR: u8 = 0x0D;

/// Cap on frame payload size to keep a misbehaving sender from exhausting
/// memory. 1 MiB is far above any realistic ADT message.
const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Read one MLLP frame from `r`. Returns `Ok(None)` on a clean EOF (peer
/// closed the connection without starting a frame). Returns the payload
/// bytes (i.e. the content *between* `SB` and `EB CR`) on success.
///
/// The reader is consumed byte-by-byte — fine for ADT-sized messages, where
/// the throughput cost is dominated by network I/O, not framing.
pub async fn read_frame<R: AsyncReadExt + Unpin>(r: &mut R) -> Result<Option<Vec<u8>>> {
    // Skip any noise bytes before SB. A clean EOF here = no frame at all.
    let mut buf = [0u8; 1];
    loop {
        match r.read(&mut buf).await {
            Ok(0) => return Ok(None),
            Ok(_) => {
                if buf[0] == SB {
                    break;
                }
                // Anything other than SB before the frame starts is a
                // protocol violation. Many real-world senders preface frames
                // with stray CRLFs though, so we silently skip non-SB bytes.
            }
            Err(e) => return Err(Error::Streaming(format!("MLLP read SB: {e}"))),
        }
    }

    let mut payload = Vec::with_capacity(512);
    loop {
        if payload.len() >= MAX_FRAME_BYTES {
            return Err(Error::Streaming(format!(
                "MLLP frame exceeds {MAX_FRAME_BYTES} bytes"
            )));
        }
        let mut byte = [0u8; 1];
        match r.read(&mut byte).await {
            Ok(0) => {
                return Err(Error::Streaming("MLLP read: EOF before EB".to_string()));
            }
            Ok(_) => {}
            Err(e) => return Err(Error::Streaming(format!("MLLP read body: {e}"))),
        }
        if byte[0] == EB {
            // Expect CR immediately after EB.
            let mut cr = [0u8; 1];
            match r.read(&mut cr).await {
                Ok(0) => {
                    return Err(Error::Streaming("MLLP read: EOF before CR".to_string()));
                }
                Ok(_) => {}
                Err(e) => return Err(Error::Streaming(format!("MLLP read CR: {e}"))),
            }
            if cr[0] != CR {
                return Err(Error::Streaming(format!(
                    "MLLP frame ended with EB {:#04x} (expected CR 0x0D)",
                    cr[0]
                )));
            }
            return Ok(Some(payload));
        }
        payload.push(byte[0]);
    }
}

/// Write one MLLP frame (`SB <payload> EB CR`) to `w` and flush.
pub async fn write_frame<W: AsyncWriteExt + Unpin>(w: &mut W, payload: &[u8]) -> Result<()> {
    w.write_all(&[SB])
        .await
        .map_err(|e| Error::Streaming(format!("MLLP write SB: {e}")))?;
    w.write_all(payload)
        .await
        .map_err(|e| Error::Streaming(format!("MLLP write body: {e}")))?;
    w.write_all(&[EB, CR])
        .await
        .map_err(|e| Error::Streaming(format!("MLLP write EB CR: {e}")))?;
    w.flush()
        .await
        .map_err(|e| Error::Streaming(format!("MLLP flush: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[tokio::test]
    async fn test_read_frame_happy_path() {
        let bytes = b"\x0bMSH|^~\\&|...\x1c\x0d";
        let mut r = Cursor::new(bytes.to_vec());
        let payload = read_frame(&mut r).await.expect("read").expect("Some");
        assert_eq!(payload, b"MSH|^~\\&|...");
    }

    #[tokio::test]
    async fn test_read_frame_returns_none_on_clean_eof() {
        let mut r: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let res = read_frame(&mut r).await.expect("read");
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn test_read_frame_skips_noise_bytes_before_sb() {
        // Real-world senders sometimes precede the frame with a CR/LF.
        let bytes = b"\r\n\x0bABC\x1c\x0d";
        let mut r = Cursor::new(bytes.to_vec());
        let payload = read_frame(&mut r).await.expect("read").expect("Some");
        assert_eq!(payload, b"ABC");
    }

    #[tokio::test]
    async fn test_read_frame_errors_on_missing_eb() {
        let bytes = b"\x0bABCDEF"; // truncated, no EB
        let mut r = Cursor::new(bytes.to_vec());
        assert!(read_frame(&mut r).await.is_err());
    }

    #[tokio::test]
    async fn test_read_frame_errors_on_eb_without_cr() {
        let bytes = b"\x0bABC\x1cX";
        let mut r = Cursor::new(bytes.to_vec());
        assert!(read_frame(&mut r).await.is_err());
    }

    #[tokio::test]
    async fn test_write_frame_round_trip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"MSH|^~\\&|sample")
            .await
            .expect("write");
        // SB + body + EB + CR.
        assert_eq!(buf.first().copied(), Some(SB));
        assert_eq!(buf[buf.len() - 2], EB);
        assert_eq!(*buf.last().unwrap(), CR);
        // Round-trip back through read_frame.
        let mut r = Cursor::new(buf);
        let payload = read_frame(&mut r).await.expect("read").expect("Some");
        assert_eq!(payload, b"MSH|^~\\&|sample");
    }

    #[tokio::test]
    async fn test_two_frames_back_to_back() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"FIRST").await.unwrap();
        write_frame(&mut buf, b"SECOND").await.unwrap();
        let mut r = Cursor::new(buf);
        let a = read_frame(&mut r).await.unwrap().unwrap();
        let b = read_frame(&mut r).await.unwrap().unwrap();
        let eof = read_frame(&mut r).await.unwrap();
        assert_eq!(a, b"FIRST");
        assert_eq!(b, b"SECOND");
        assert!(eof.is_none());
    }
}
