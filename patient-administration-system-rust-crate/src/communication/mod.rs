//! communication — Letter template rendering, persistence, and SMS dispatch.

pub mod sms;

pub use sms::{LogSmsProvider, NoopSmsProvider, SmsProvider};

use sea_orm::{DatabaseConnection, TransactionTrait};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::{
    audit::{AuditLogRepository, UserContext},
    letter::LetterRepository,
    outbox::OutboxRepository,
    patient::PatientRepository,
};
use crate::models::ContactPointSystem;
use crate::models::communication::{DeliveryChannel, GeneratedLetter, LetterStatus};
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

pub struct CommunicationService {
    db: DatabaseConnection,
    publisher: Arc<dyn EventPublisher>,
    sms_provider: Arc<dyn SmsProvider>,
}

impl CommunicationService {
    /// Build a service with the default SMS provider ([`NoopSmsProvider`]).
    /// SMS auto-send is **off** until a different provider is installed via
    /// [`Self::with_sms_provider`].
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self {
            db,
            publisher,
            sms_provider: Arc::new(NoopSmsProvider),
        }
    }

    /// Swap in a different [`SmsProvider`]. Use at construction time in
    /// `main.rs`/tests to pick `LogSmsProvider` (dev) or a real gateway.
    pub fn with_sms_provider(mut self, provider: Arc<dyn SmsProvider>) -> Self {
        self.sms_provider = provider;
        self
    }

    pub async fn generate_letter(
        &self,
        template_id: Uuid,
        patient_id: Uuid,
        appointment_id: Option<Uuid>,
        channel: DeliveryChannel,
        extra_context: serde_json::Value,
        ctx: &UserContext,
    ) -> Result<GeneratedLetter> {
        let template = LetterRepository::find_template_by_id(&self.db, template_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("letter template {template_id}")))?;
        let patient = PatientRepository::find_by_id(&self.db, patient_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("patient {patient_id}")))?;

        let mut tctx = tera::Context::new();
        tctx.insert("patient", &patient);
        if let serde_json::Value::Object(map) = &extra_context {
            for (k, v) in map {
                tctx.insert(k, v);
            }
        }

        for var in &template.required_variables {
            if tctx.get(var).is_none() {
                return Err(Error::validation(format!(
                    "missing required template variable: {var}"
                )));
            }
        }

        let rendered_body = tera::Tera::one_off(&template.body_tera, &tctx, false)
            .map_err(|e| Error::render(format!("body: {e}")))?;
        let rendered_subject = tera::Tera::one_off(&template.subject, &tctx, false)
            .map_err(|e| Error::render(format!("subject: {e}")))?;

        let mut letter = GeneratedLetter::new(
            template_id,
            patient_id,
            channel,
            rendered_subject,
            rendered_body,
        );
        letter.appointment_id = appointment_id;

        let ctx_clone = ctx.clone();
        let letter_clone = letter.clone();
        let letter = self
            .db
            .transaction::<_, GeneratedLetter, Error>(|txn| {
                Box::pin(async move {
                    let l = LetterRepository::create_generated_letter(txn, &letter_clone).await?;
                    AuditLogRepository::log(
                        txn,
                        "generated_letter",
                        l.id,
                        "generate",
                        None,
                        Some(serde_json::to_value(&l).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "LetterGenerated",
                        &serde_json::json!({
                            "letter_id": l.id,
                            "template_id": template_id,
                            "patient_id": patient_id,
                        }),
                    )
                    .await?;
                    Ok(l)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "LetterGenerated",
                serde_json::json!({ "letter_id": letter.id }),
            ))
            .await;

        // v0.8: when the channel is Sms and a real provider is wired, auto-
        // dispatch immediately and flip status. Print and Email keep the
        // v0.1 "render-and-stay-Pending" behavior — operators flip those
        // manually via the existing /sent and /failed endpoints.
        let letter = if matches!(channel, DeliveryChannel::Sms) && self.sms_provider.is_enabled() {
            let phone = patient
                .telecom
                .iter()
                .find(|c| c.system == ContactPointSystem::Phone)
                .map(|c| c.value.clone());
            match phone {
                None => {
                    tracing::warn!(
                        target: "pas::sms",
                        letter_id = %letter.id,
                        patient_id = %patient_id,
                        "SMS letter generated but patient has no Phone telecom; leaving status=pending"
                    );
                    let _ = AuditLogRepository::log(
                        &self.db,
                        "generated_letter",
                        letter.id,
                        "send_sms_skipped_no_phone",
                        None,
                        None,
                        ctx,
                    )
                    .await;
                    letter
                }
                Some(to) => match self.sms_provider.send(&to, &letter.rendered_body).await {
                    Ok(()) => match LetterRepository::mark_sent(&self.db, letter.id).await {
                        Ok(updated) => {
                            let _ = AuditLogRepository::log(
                                &self.db,
                                "generated_letter",
                                updated.id,
                                "send_sms_ok",
                                None,
                                Some(serde_json::json!({ "to": to })),
                                ctx,
                            )
                            .await;
                            updated
                        }
                        Err(e) => {
                            tracing::warn!(
                                target: "pas::sms",
                                letter_id = %letter.id,
                                "SMS sent but post-send mark_sent failed: {e}"
                            );
                            letter
                        }
                    },
                    Err(send_err) => {
                        tracing::warn!(
                            target: "pas::sms",
                            letter_id = %letter.id,
                            "SMS provider failed: {send_err}"
                        );
                        let _ = AuditLogRepository::log(
                            &self.db,
                            "generated_letter",
                            letter.id,
                            "send_sms_failed",
                            None,
                            Some(serde_json::json!({
                                "to": to,
                                "error": send_err.to_string(),
                            })),
                            ctx,
                        )
                        .await;
                        LetterRepository::mark_failed(&self.db, letter.id)
                            .await
                            .unwrap_or(letter)
                    }
                },
            }
        } else {
            letter
        };

        // Silence the unused-import warning when the SMS path isn't taken.
        let _ = LetterStatus::Pending;
        Ok(letter)
    }
}

fn unwrap_txn_err(e: sea_orm::TransactionError<Error>) -> Error {
    match e {
        sea_orm::TransactionError::Connection(c) => Error::Database(c),
        sea_orm::TransactionError::Transaction(t) => t,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_tera_render_smoke() {
        let mut ctx = tera::Context::new();
        ctx.insert("name", "World");
        let out = tera::Tera::one_off("Hello {{ name }}", &ctx, false).unwrap();
        assert_eq!(out, "Hello World");
    }
}
