//! SMS provider trait + first-party implementations.
//!
//! v0.8 wires the long-dormant `DeliveryChannel::Sms` variant. The PAS
//! itself does not speak to any real SMS gateway (Twilio, MessageBird,
//! Plivo, …); instead [`CommunicationService::generate_letter`] looks up
//! the configured [`SmsProvider`] in [`AppState`] and calls
//! [`SmsProvider::send`] when a letter's `channel == Sms` and the provider
//! reports `is_enabled()`. Consumers swap in a real provider by
//! implementing the trait against their gateway of choice.
//!
//! Two first-party implementations ship:
//!
//! - [`NoopSmsProvider`] — the default. `is_enabled() == false`, so
//!   `generate_letter` skips auto-send entirely; SMS letters are rendered
//!   and stored as `Pending`, same as v0.7 behavior. Picked when
//!   `PAS_SMS_PROVIDER` is unset or set to `none`.
//! - [`LogSmsProvider`] — logs every outbound message at
//!   `tracing::info!(target: "pas::sms", …)`. Useful for dev / smoke
//!   tests / replays without spending real money. Picked when
//!   `PAS_SMS_PROVIDER=log`.

use async_trait::async_trait;

use crate::Result;

/// Asynchronous SMS provider. Implementations connect to a downstream SMS
/// gateway (or stub it out for dev / testing).
///
/// `to` is the recipient phone number — exactly the value stored in the
/// patient's first `ContactPoint { system: Phone }`. The PAS does not
/// E.164-normalize it before calling `send`; the provider is responsible
/// for whatever transport-specific formatting its gateway requires.
///
/// `body` is the rendered Tera template output (already SMS-safe — Tera
/// templates intended for SMS should keep themselves under the relevant
/// 160 / 1600-char limits).
#[async_trait]
pub trait SmsProvider: Send + Sync {
    /// Send one SMS message. Returns `Ok(())` on accepted-for-delivery.
    /// Returns `Err(Error::Streaming)` (or similar) when the gateway
    /// rejects or is unreachable — the caller flips the
    /// [`crate::models::communication::GeneratedLetter`] to `Failed` and
    /// records the diagnostic in the audit log.
    async fn send(&self, to: &str, body: &str) -> Result<()>;

    /// `true` when the provider should actually attempt sends. `false`
    /// means "stay out of the way" — `generate_letter` skips auto-send
    /// for SMS letters and leaves them `Pending` (an operator can still
    /// flip status via `POST /api/letters/{id}/sent` or `…/failed`).
    ///
    /// Defaults to `true` so a hand-written provider doesn't have to
    /// opt in. [`NoopSmsProvider`] overrides to `false`.
    fn is_enabled(&self) -> bool {
        true
    }
}

/// Default provider: never sends, never claims to be enabled. With this
/// installed, generating an SMS letter behaves exactly like v0.7 — the
/// letter is rendered and persisted as `Pending`, and the operator is
/// expected to flip status manually (or wire a real provider).
pub struct NoopSmsProvider;

#[async_trait]
impl SmsProvider for NoopSmsProvider {
    async fn send(&self, _to: &str, _body: &str) -> Result<()> {
        Ok(())
    }
    fn is_enabled(&self) -> bool {
        false
    }
}

/// Dev / test provider: emits every outbound message to the `pas::sms`
/// tracing target at `info!` and returns `Ok(())`. No money spent, no
/// real gateway involved, but the auto-send code path runs end-to-end
/// so the letter flips to `Sent` and gets `sent_at` stamped.
pub struct LogSmsProvider;

#[async_trait]
impl SmsProvider for LogSmsProvider {
    async fn send(&self, to: &str, body: &str) -> Result<()> {
        tracing::info!(
            target: "pas::sms",
            recipient = %to,
            chars = body.chars().count(),
            "SMS dispatched (LogSmsProvider): {body}"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_noop_provider_is_disabled_and_succeeds() {
        let p = NoopSmsProvider;
        assert!(!p.is_enabled(), "Noop must report disabled");
        assert!(
            p.send("+15555550100", "ignored").await.is_ok(),
            "Noop send always succeeds (without doing anything)"
        );
    }

    #[tokio::test]
    async fn test_log_provider_is_enabled_and_succeeds() {
        let p = LogSmsProvider;
        assert!(p.is_enabled(), "Log provider must report enabled");
        assert!(p.send("+15555550100", "hello world").await.is_ok());
    }

    #[tokio::test]
    async fn test_log_provider_handles_empty_body() {
        // Empty body is a legal (if useless) SMS — the provider must not
        // panic on it. Real gateways may reject; that's their call.
        let p = LogSmsProvider;
        assert!(p.send("+15555550100", "").await.is_ok());
    }
}
