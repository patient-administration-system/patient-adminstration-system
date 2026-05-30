//! letter repository — LetterTemplate and GeneratedLetter

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::{generated_letter, letter_template};
use crate::models::communication::{
    DeliveryChannel, GeneratedLetter, LetterStatus, LetterTemplate,
};
use crate::{Error, Result};

pub struct LetterRepository;

impl LetterRepository {
    pub async fn create_template<C: ConnectionTrait>(
        conn: &C,
        t: &LetterTemplate,
    ) -> Result<LetterTemplate> {
        let am = template_to_active_model(t)?;
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(t.clone())
    }

    pub async fn find_template_by_id<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<LetterTemplate>> {
        let m = letter_template::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(template_from_model).transpose()
    }

    pub async fn list_active_templates<C: ConnectionTrait>(
        conn: &C,
    ) -> Result<Vec<LetterTemplate>> {
        let rows = letter_template::Entity::find()
            .filter(letter_template::Column::Active.eq(true))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(template_from_model).collect()
    }

    pub async fn create_generated_letter<C: ConnectionTrait>(
        conn: &C,
        g: &GeneratedLetter,
    ) -> Result<GeneratedLetter> {
        let am = letter_to_active_model(g);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(g.clone())
    }

    pub async fn find_generated_letter_by_id<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<GeneratedLetter>> {
        let m = generated_letter::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(letter_from_model).transpose()
    }

    pub async fn mark_sent<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<GeneratedLetter> {
        let m = generated_letter::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("generated_letter {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am: generated_letter::ActiveModel = m.into();
        am.status = Set(letter_status_to_str(LetterStatus::Sent).to_string());
        am.sent_at = Set(Some(now));
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        letter_from_model(updated)
    }

    pub async fn mark_failed<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<GeneratedLetter> {
        let m = generated_letter::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("generated_letter {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am: generated_letter::ActiveModel = m.into();
        am.status = Set(letter_status_to_str(LetterStatus::Failed).to_string());
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        letter_from_model(updated)
    }
}

// --- conversion helpers ---

pub(crate) fn delivery_channel_to_str(c: DeliveryChannel) -> &'static str {
    match c {
        DeliveryChannel::Print => "print",
        DeliveryChannel::Email => "email",
        DeliveryChannel::Sms => "sms",
    }
}

pub(crate) fn delivery_channel_from_str(s: &str) -> Result<DeliveryChannel> {
    match s {
        "print" => Ok(DeliveryChannel::Print),
        "email" => Ok(DeliveryChannel::Email),
        "sms" => Ok(DeliveryChannel::Sms),
        other => Err(Error::internal(format!(
            "unknown delivery channel: {other}"
        ))),
    }
}

pub(crate) fn letter_status_to_str(s: LetterStatus) -> &'static str {
    match s {
        LetterStatus::Pending => "pending",
        LetterStatus::Sent => "sent",
        LetterStatus::Failed => "failed",
    }
}

pub(crate) fn letter_status_from_str(s: &str) -> Result<LetterStatus> {
    match s {
        "pending" => Ok(LetterStatus::Pending),
        "sent" => Ok(LetterStatus::Sent),
        "failed" => Ok(LetterStatus::Failed),
        other => Err(Error::internal(format!("unknown letter status: {other}"))),
    }
}

fn template_to_active_model(t: &LetterTemplate) -> Result<letter_template::ActiveModel> {
    let required = serde_json::to_value(&t.required_variables)
        .map_err(|e| Error::internal(format!("serialize required_variables: {e}")))?;
    let channels = serde_json::to_value(&t.channels)
        .map_err(|e| Error::internal(format!("serialize channels: {e}")))?;
    Ok(letter_template::ActiveModel {
        id: Set(t.id),
        name: Set(t.name.clone()),
        subject: Set(t.subject.clone()),
        body_tera: Set(t.body_tera.clone()),
        required_variables: Set(required),
        channels: Set(channels),
        active: Set(t.active),
        created_at: Set(t.created_at.fixed_offset()),
        updated_at: Set(t.updated_at.fixed_offset()),
    })
}

fn template_from_model(m: letter_template::Model) -> Result<LetterTemplate> {
    Ok(LetterTemplate {
        id: m.id,
        name: m.name,
        subject: m.subject,
        body_tera: m.body_tera,
        required_variables: serde_json::from_value(m.required_variables)
            .map_err(|e| Error::internal(format!("deserialize required_variables: {e}")))?,
        channels: serde_json::from_value(m.channels)
            .map_err(|e| Error::internal(format!("deserialize channels: {e}")))?,
        active: m.active,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

fn letter_to_active_model(g: &GeneratedLetter) -> generated_letter::ActiveModel {
    generated_letter::ActiveModel {
        id: Set(g.id),
        template_id: Set(g.template_id),
        patient_id: Set(g.patient_id),
        appointment_id: Set(g.appointment_id),
        rendered_subject: Set(g.rendered_subject.clone()),
        rendered_body: Set(g.rendered_body.clone()),
        channel: Set(delivery_channel_to_str(g.channel).to_string()),
        status: Set(letter_status_to_str(g.status).to_string()),
        sent_at: Set(g.sent_at.map(|t| t.fixed_offset())),
        created_at: Set(g.created_at.fixed_offset()),
        updated_at: Set(g.updated_at.fixed_offset()),
    }
}

fn letter_from_model(m: generated_letter::Model) -> Result<GeneratedLetter> {
    Ok(GeneratedLetter {
        id: m.id,
        template_id: m.template_id,
        patient_id: m.patient_id,
        appointment_id: m.appointment_id,
        rendered_subject: m.rendered_subject,
        rendered_body: m.rendered_body,
        channel: delivery_channel_from_str(&m.channel)?,
        status: letter_status_from_str(&m.status)?,
        sent_at: m.sent_at.map(|t| t.with_timezone(&chrono::Utc)),
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_letter_template_roundtrip_via_active_model() {
        let mut t = LetterTemplate::new(
            "appointment_reminder".into(),
            "Your appointment".into(),
            "Hello {{ name }}".into(),
        );
        t.required_variables = vec!["name".into()];
        t.channels = vec![DeliveryChannel::Email, DeliveryChannel::Print];
        let am = template_to_active_model(&t).expect("to_active_model");
        let m = letter_template::Model {
            id: am.id.clone().unwrap(),
            name: am.name.clone().unwrap(),
            subject: am.subject.clone().unwrap(),
            body_tera: am.body_tera.clone().unwrap(),
            required_variables: am.required_variables.clone().unwrap(),
            channels: am.channels.clone().unwrap(),
            active: am.active.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = template_from_model(m).expect("from_model");
        assert_eq!(back.id, t.id);
        assert_eq!(back.required_variables, vec!["name".to_string()]);
        assert_eq!(back.channels.len(), 2);
        assert!(back.active);
    }
}
