//! TCP listener that accepts MLLP-framed HL7 v2 messages and dispatches
//! them to the existing HTTP handlers via [`axum::Router::oneshot`].
//!
//! Design: zero handler refactoring. The listener inspects the message
//! type, picks the right `/api/hl7/v2/...` endpoint, builds a synthetic
//! request, and hands it to the router. The response body is the ACK
//! envelope, which the listener writes back in MLLP framing.

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt;

use crate::hl7v2::{message_type, mllp, parse_message};
use crate::{Error, Result};

/// MLLP TCP server. Owns a clone of the axum router and accepts connections
/// in a loop; each connection runs on its own tokio task and may carry
/// multiple frames.
pub struct MllpServer {
    router: Router,
}

impl MllpServer {
    pub fn new(router: Router) -> Self {
        Self { router }
    }

    /// Bind and accept connections forever. Returns only on a fatal accept
    /// error (e.g. socket closed); per-connection errors are logged and the
    /// connection is dropped, but the listener keeps running.
    pub async fn serve(self, bind: &str) -> Result<()> {
        let listener = TcpListener::bind(bind)
            .await
            .map_err(|e| Error::Streaming(format!("MLLP bind {bind}: {e}")))?;
        tracing::info!(target: "pas::hl7v2::mllp", "MLLP listener bound on {bind}");
        loop {
            let (sock, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(target: "pas::hl7v2::mllp", "accept: {e}");
                    return Err(Error::Streaming(format!("MLLP accept: {e}")));
                }
            };
            tracing::debug!(target: "pas::hl7v2::mllp", "new MLLP connection from {peer}");
            let router = self.router.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(sock, router).await {
                    tracing::warn!(target: "pas::hl7v2::mllp", "{peer} connection: {e}");
                }
            });
        }
    }
}

/// Read frames from one TCP connection until EOF; for each frame, dispatch
/// and write the ACK back as a fresh MLLP frame.
async fn handle_connection(sock: TcpStream, router: Router) -> Result<()> {
    let (mut rd, mut wr) = sock.into_split();
    loop {
        let frame = match mllp::read_frame(&mut rd).await {
            Ok(Some(f)) => f,
            Ok(None) => return Ok(()),
            Err(e) => return Err(e),
        };
        let ack = dispatch_frame(&router, &frame).await;
        mllp::write_frame(&mut wr, ack.as_bytes()).await?;
    }
}

/// Decide which HTTP endpoint to route a frame to, call the router, and
/// return the response body (which is the ACK envelope).
async fn dispatch_frame(router: &Router, frame: &[u8]) -> String {
    let body_str = String::from_utf8_lossy(frame).into_owned();
    let path = route_for_payload(&body_str);
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/hl7-v2")
        .body(Body::from(body_str))
        .expect("static request builder");
    let resp = match router.clone().oneshot(req).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(target: "pas::hl7v2::mllp", "router oneshot: {e}");
            return crate::hl7v2::ack(
                "PAS",
                "UNKNOWN",
                "UNKNOWN",
                crate::hl7v2::AckCode::Reject,
                Some(&format!("router error: {e}")),
            );
        }
    };
    let bytes = match axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return crate::hl7v2::ack(
                "PAS",
                "UNKNOWN",
                "UNKNOWN",
                crate::hl7v2::AckCode::Reject,
                Some(&format!("read response: {e}")),
            );
        }
    };
    String::from_utf8(bytes.to_vec()).unwrap_or_else(|e| {
        crate::hl7v2::ack(
            "PAS",
            "UNKNOWN",
            "UNKNOWN",
            crate::hl7v2::AckCode::Reject,
            Some(&format!("non-utf8 response: {e}")),
        )
    })
}

/// Pick the `/api/hl7/v2/<X>` endpoint for the given payload. If the payload
/// fails to parse, the patient endpoint will return an AR — same as it
/// would over plain HTTP.
fn route_for_payload(body: &str) -> &'static str {
    // FHS/BHS-prefixed payloads carry a batch envelope; route them to the
    // batch endpoint, which dispatches each contained message internally.
    if crate::hl7v2::looks_like_batch(body) {
        return "/api/hl7/v2/batch";
    }
    let msg = match parse_message(body) {
        Ok(m) => m,
        Err(_) => return "/api/hl7/v2/patient",
    };
    let (code, event) = message_type(&msg);
    match (code.as_str(), event.as_str()) {
        ("ADT", "A01") => "/api/hl7/v2/admit",
        ("ADT", "A02") => "/api/hl7/v2/transfer",
        ("ADT", "A03") => "/api/hl7/v2/discharge",
        ("ADT", "A04") => "/api/hl7/v2/register",
        ("ADT", "A05") => "/api/hl7/v2/pre-admit",
        ("ADT", "A06") => "/api/hl7/v2/change-to-inpatient",
        ("ADT", "A07") => "/api/hl7/v2/change-to-outpatient",
        ("ADT", "A21") => "/api/hl7/v2/leave-start",
        ("ADT", "A22") => "/api/hl7/v2/leave-end",
        ("ADT", "A23") => "/api/hl7/v2/delete-patient",
        ("ADT", "A38") => "/api/hl7/v2/cancel-pre-admit",
        ("ADT", "A08") => "/api/hl7/v2/update",
        ("ADT", "A11") => "/api/hl7/v2/cancel-admit",
        ("ADT", "A12") => "/api/hl7/v2/cancel-transfer",
        ("ADT", "A13") => "/api/hl7/v2/cancel-discharge",
        ("ADT", "A40") => "/api/hl7/v2/merge",
        ("DFT", "P03") => "/api/hl7/v2/dft",
        ("MFN", "M02") => "/api/hl7/v2/mfn-staff",
        ("MFN", "M05") => "/api/hl7/v2/mfn-location",
        ("SIU", "S12") => "/api/hl7/v2/schedule-book",
        ("SIU", "S13") => "/api/hl7/v2/schedule-reschedule",
        ("SIU", "S14") => "/api/hl7/v2/schedule-modify",
        ("SIU", "S15") => "/api/hl7/v2/schedule-cancel",
        _ => "/api/hl7/v2/patient",
    }
}

/// Public entry-point usable in tests: dispatch one MLLP frame end-to-end
/// against any `AsyncRead + AsyncWrite` pair. Useful for in-process round-
/// trip tests that don't want to bind a real socket.
pub async fn handle_one_frame<RW>(stream: &mut RW, router: Router) -> Result<()>
where
    RW: AsyncRead + AsyncWrite + Unpin,
{
    let frame = match mllp::read_frame(stream).await? {
        Some(f) => f,
        None => return Ok(()),
    };
    let ack = dispatch_frame(&router, &frame).await;
    mllp::write_frame(stream, ack.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_for_payload_a01() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A01|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/admit");
    }

    #[test]
    fn test_route_for_payload_a02() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A02|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/transfer");
    }

    #[test]
    fn test_route_for_payload_a03() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A03|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/discharge");
    }

    #[test]
    fn test_route_for_payload_a04() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A04|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/register");
    }

    #[test]
    fn test_route_for_payload_a05() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A05|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/pre-admit");
    }

    #[test]
    fn test_route_for_payload_a06() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A06|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/change-to-inpatient");
    }

    #[test]
    fn test_route_for_payload_a07() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A07|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/change-to-outpatient");
    }

    #[test]
    fn test_route_for_payload_a38() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A38|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/cancel-pre-admit");
    }

    #[test]
    fn test_route_for_payload_a21() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A21|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/leave-start");
    }

    #[test]
    fn test_route_for_payload_a22() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A22|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/leave-end");
    }

    #[test]
    fn test_route_for_payload_a23() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A23|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/delete-patient");
    }

    #[test]
    fn test_route_for_payload_a28_defaults_to_patient() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A28|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/patient");
    }

    #[test]
    fn test_route_for_payload_a08() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A08|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/update");
    }

    #[test]
    fn test_route_for_payload_a11() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A11|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/cancel-admit");
    }

    #[test]
    fn test_route_for_payload_a12() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A12|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/cancel-transfer");
    }

    #[test]
    fn test_route_for_payload_a13() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A13|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/cancel-discharge");
    }

    #[test]
    fn test_route_for_payload_a40() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A40|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/merge");
    }

    #[test]
    fn test_route_for_payload_dft_p03() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||DFT^P03|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/dft");
    }

    #[test]
    fn test_route_for_payload_mfn_m02() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||MFN^M02|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/mfn-staff");
    }

    #[test]
    fn test_route_for_payload_mfn_m05() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||MFN^M05|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/mfn-location");
    }

    #[test]
    fn test_route_for_payload_siu_s12() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||SIU^S12|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/schedule-book");
    }

    #[test]
    fn test_route_for_payload_siu_s15() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||SIU^S15|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/schedule-cancel");
    }

    #[test]
    fn test_route_for_payload_siu_s13() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||SIU^S13|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/schedule-reschedule");
    }

    #[test]
    fn test_route_for_payload_siu_s14() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||SIU^S14|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/schedule-modify");
    }

    #[test]
    fn test_route_for_payload_siu_unknown_trigger_falls_back() {
        // SIU^S17 (block schedule) isn't implemented — falls back to
        // /patient which AE-ACKs it as an unsupported trigger.
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||SIU^S17|X|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/patient");
    }

    #[test]
    fn test_route_for_payload_bhs_routes_to_batch() {
        let body = "BHS|^~\\&|S|F|R|F|20260523120000||BATCH-1|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/batch");
    }

    #[test]
    fn test_route_for_payload_fhs_routes_to_batch() {
        let body = "FHS|^~\\&|S|F|R|F|20260523120000||FILE-1|P|2.5\r";
        assert_eq!(route_for_payload(body), "/api/hl7/v2/batch");
    }

    #[test]
    fn test_route_for_payload_garbage_routes_to_patient() {
        assert_eq!(route_for_payload("not-v2"), "/api/hl7/v2/patient");
    }
}
