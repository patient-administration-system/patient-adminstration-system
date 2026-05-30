//! communication

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How a letter should reach its recipient.
///
/// - `Print` — render and store; an external process picks up the body and
///   produces the physical mail. The PAS never flips status on its own;
///   `POST /api/letters/{id}/sent` is the manual signal.
/// - `Email` — render and store; same manual-flip semantics as `Print`.
/// - `Sms` — render and store. As of v0.8, when an enabled
///   [`crate::communication::SmsProvider`] is wired (`PAS_SMS_PROVIDER=log`
///   or a consumer-supplied provider), `CommunicationService::generate_letter`
///   also auto-dispatches the message to the patient's first
///   `ContactPoint { system: Phone }` and flips status to `Sent` on success
///   or `Failed` on error. When the configured provider is
///   [`crate::communication::NoopSmsProvider`] (the default), the letter
///   stays `Pending` — the same behavior as `Print` and `Email`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryChannel {
    Print,
    Email,
    Sms,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LetterStatus {
    Pending,
    Sent,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LetterTemplate {
    pub id: Uuid,
    pub name: String,
    pub subject: String,
    pub body_tera: String,
    pub required_variables: Vec<String>,
    pub channels: Vec<DeliveryChannel>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl LetterTemplate {
    pub fn new(name: String, subject: String, body_tera: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            subject,
            body_tera,
            required_variables: Vec::new(),
            channels: vec![DeliveryChannel::Print],
            active: true,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedLetter {
    pub id: Uuid,
    pub template_id: Uuid,
    pub patient_id: Uuid,
    pub appointment_id: Option<Uuid>,
    pub rendered_subject: String,
    pub rendered_body: String,
    pub channel: DeliveryChannel,
    pub status: LetterStatus,
    pub sent_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl GeneratedLetter {
    pub fn new(
        template_id: Uuid,
        patient_id: Uuid,
        channel: DeliveryChannel,
        rendered_subject: String,
        rendered_body: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            template_id,
            patient_id,
            appointment_id: None,
            rendered_subject,
            rendered_body,
            channel,
            status: LetterStatus::Pending,
            sent_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn mark_sent(&mut self) {
        let now = Utc::now();
        self.status = LetterStatus::Sent;
        self.sent_at = Some(now);
        self.updated_at = now;
    }

    pub fn mark_failed(&mut self) {
        self.status = LetterStatus::Failed;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_template() -> LetterTemplate {
        LetterTemplate::new(
            "appointment_reminder".into(),
            "Your appointment".into(),
            "Hello {{ patient.name }}, your appointment is on {{ appointment.date }}.".into(),
        )
    }

    fn sample_generated() -> GeneratedLetter {
        GeneratedLetter::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            DeliveryChannel::Print,
            "Your appointment".into(),
            "Hello Jane Doe, your appointment is on 2026-06-01.".into(),
        )
    }

    #[test]
    fn test_letter_template_new_defaults() {
        let t = sample_template();
        assert!(t.active);
        assert_eq!(t.channels, vec![DeliveryChannel::Print]);
        assert!(t.required_variables.is_empty());
        assert_eq!(t.name, "appointment_reminder");
        assert_eq!(t.subject, "Your appointment");
        assert_eq!(t.created_at, t.updated_at);
    }

    #[test]
    fn test_generated_letter_new_defaults() {
        let g = sample_generated();
        assert_eq!(g.status, LetterStatus::Pending);
        assert!(g.sent_at.is_none());
        assert!(g.appointment_id.is_none());
        assert_eq!(g.channel, DeliveryChannel::Print);
        assert_eq!(g.created_at, g.updated_at);
    }

    #[test]
    fn test_mark_sent_flips_status() {
        let mut g = sample_generated();
        let original_updated = g.updated_at;
        // Ensure a measurable tick on platforms with coarse clocks.
        std::thread::sleep(std::time::Duration::from_millis(1));
        g.mark_sent();
        assert_eq!(g.status, LetterStatus::Sent);
        assert!(g.sent_at.is_some());
        assert!(g.updated_at >= original_updated);
    }

    #[test]
    fn test_mark_failed_flips_status() {
        let mut g = sample_generated();
        let original_updated = g.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(1));
        g.mark_failed();
        assert_eq!(g.status, LetterStatus::Failed);
        assert!(g.sent_at.is_none());
        assert!(g.updated_at >= original_updated);
    }

    #[test]
    fn test_letter_template_serde_roundtrip() {
        let mut t = sample_template();
        t.required_variables = vec!["patient.name".into(), "appointment.date".into()];
        t.channels = vec![DeliveryChannel::Print, DeliveryChannel::Email];
        let json = serde_json::to_string(&t).expect("serialize");
        let back: LetterTemplate = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, t.id);
        assert_eq!(back.name, t.name);
        assert_eq!(back.subject, t.subject);
        assert_eq!(back.body_tera, t.body_tera);
        assert_eq!(back.required_variables, t.required_variables);
        assert_eq!(back.channels, t.channels);
        assert_eq!(back.active, t.active);
    }

    #[test]
    fn test_generated_letter_serde_roundtrip() {
        let mut g = sample_generated();
        g.appointment_id = Some(Uuid::new_v4());
        g.mark_sent();
        let json = serde_json::to_string(&g).expect("serialize");
        let back: GeneratedLetter = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, g.id);
        assert_eq!(back.template_id, g.template_id);
        assert_eq!(back.patient_id, g.patient_id);
        assert_eq!(back.appointment_id, g.appointment_id);
        assert_eq!(back.rendered_subject, g.rendered_subject);
        assert_eq!(back.rendered_body, g.rendered_body);
        assert_eq!(back.channel, g.channel);
        assert_eq!(back.status, g.status);
        assert_eq!(back.sent_at, g.sent_at);
    }

    #[test]
    fn test_delivery_channel_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&DeliveryChannel::Print).unwrap(),
            "\"print\""
        );
        assert_eq!(
            serde_json::to_string(&DeliveryChannel::Email).unwrap(),
            "\"email\""
        );
        assert_eq!(
            serde_json::to_string(&DeliveryChannel::Sms).unwrap(),
            "\"sms\""
        );
    }

    #[test]
    fn test_letter_status_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&LetterStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&LetterStatus::Sent).unwrap(),
            "\"sent\""
        );
        assert_eq!(
            serde_json::to_string(&LetterStatus::Failed).unwrap(),
            "\"failed\""
        );
    }
}
