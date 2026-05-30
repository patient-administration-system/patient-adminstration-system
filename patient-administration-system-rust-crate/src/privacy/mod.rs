//! privacy
//!
//! Data masking and GDPR-style export helpers.
//!
//! [`mask_value`] is the low-level primitive: replace all-but-the-last-N
//! characters with `*`. [`mask_contact_point`] and [`mask_patient`] layer
//! domain-aware policy on top of it. [`export_patient`] dumps a patient as
//! JSON — the format the subject-access-request endpoint hands back.

use serde_json::Value;

use crate::models::patient::Patient;
use crate::models::{ContactPoint, ContactPointSystem};

/// Mask all but the last `keep_tail` characters of `value` with `*`.
///
/// If `keep_tail >= value.chars().count()`, the entire string is masked.
/// Operates on Unicode scalar values, not bytes, so multi-byte characters
/// count as one.
pub fn mask_value(value: &str, keep_tail: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= keep_tail {
        return "*".repeat(chars.len());
    }
    let masked_len = chars.len() - keep_tail;
    let mut out: String = std::iter::repeat_n('*', masked_len).collect();
    out.extend(chars.iter().skip(masked_len));
    out
}

/// Mask a [`ContactPoint`] according to its system.
///
/// - `Email`: keep the `@domain` intact, mask the local part except for one
///   tail character.
/// - `Phone`, `Sms`, `Fax`: keep the last four characters; mask the rest.
/// - Anything else: mask everything.
pub fn mask_contact_point(c: &ContactPoint) -> ContactPoint {
    let mut out = c.clone();
    out.value = match c.system {
        ContactPointSystem::Email => {
            if let Some(at) = c.value.find('@') {
                let (local, domain) = c.value.split_at(at);
                format!("{}{}", mask_value(local, 1), domain)
            } else {
                mask_value(&c.value, 0)
            }
        }
        ContactPointSystem::Phone | ContactPointSystem::Sms | ContactPointSystem::Fax => {
            mask_value(&c.value, 4)
        }
        _ => mask_value(&c.value, 0),
    };
    out
}

/// Return a clone of `p` with PII masked.
///
/// Currently masks every contact point and every identifier value (keeping
/// the last four characters of each identifier).
pub fn mask_patient(p: &Patient) -> Patient {
    let mut out = p.clone();
    out.telecom = out.telecom.iter().map(mask_contact_point).collect();
    out.identifiers = out
        .identifiers
        .iter()
        .map(|i| {
            let mut x = i.clone();
            x.value = mask_value(&i.value, 4);
            x
        })
        .collect();
    out
}

/// Produce a JSON dump of `p` suitable for a GDPR subject-access response.
///
/// Returns [`Value::Null`] if serialization fails — this is a defensive
/// fallback; `Patient` always serializes successfully in practice.
pub fn export_patient(p: &Patient) -> Value {
    serde_json::to_value(p).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::identifier::Identifier;
    use crate::models::patient::HumanName;
    use crate::models::{ContactPoint, ContactPointSystem, ContactPointUse, Gender};

    fn sample_patient() -> Patient {
        let name = HumanName {
            use_type: None,
            family: "Doe".into(),
            given: vec!["Jane".into()],
            prefix: vec![],
            suffix: vec![],
        };
        Patient::new(name, Gender::Female)
    }

    #[test]
    fn test_mask_value_keeps_tail() {
        assert_eq!(mask_value("1234567", 4), "***4567");
    }

    #[test]
    fn test_mask_value_when_tail_equals_length() {
        assert_eq!(mask_value("12", 4), "**");
    }

    #[test]
    fn test_mask_value_zero_tail_masks_all() {
        assert_eq!(mask_value("hello", 0), "*****");
    }

    #[test]
    fn test_mask_value_unicode_counts_scalars() {
        // 5 scalars, keep last 2.
        let out = mask_value("héllo", 2);
        assert_eq!(out.chars().count(), 5);
        assert!(out.starts_with("***"));
        assert!(out.ends_with("lo"));
    }

    #[test]
    fn test_mask_contact_point_email() {
        let c = ContactPoint {
            system: ContactPointSystem::Email,
            value: "jane.doe@example.com".into(),
            use_type: Some(ContactPointUse::Home),
        };
        let masked = mask_contact_point(&c);
        assert!(masked.value.ends_with("@example.com"));
        // local part length 8: 'jane.doe' → 7 stars + 'e'
        assert!(masked.value.starts_with("*******e@"));
    }

    #[test]
    fn test_mask_contact_point_email_without_at_falls_back() {
        let c = ContactPoint {
            system: ContactPointSystem::Email,
            value: "no-at-sign".into(),
            use_type: None,
        };
        let masked = mask_contact_point(&c);
        assert_eq!(masked.value, "**********");
    }

    #[test]
    fn test_mask_contact_point_phone_keeps_last_four() {
        let c = ContactPoint {
            system: ContactPointSystem::Phone,
            value: "5551234567".into(),
            use_type: None,
        };
        let masked = mask_contact_point(&c);
        assert_eq!(masked.value, "******4567");
    }

    #[test]
    fn test_mask_contact_point_sms_uses_phone_rule() {
        let c = ContactPoint {
            system: ContactPointSystem::Sms,
            value: "5551234567".into(),
            use_type: None,
        };
        let masked = mask_contact_point(&c);
        assert_eq!(masked.value, "******4567");
    }

    #[test]
    fn test_mask_contact_point_other_systems_fully_masked() {
        let c = ContactPoint {
            system: ContactPointSystem::Url,
            value: "https://example.com".into(),
            use_type: None,
        };
        let masked = mask_contact_point(&c);
        assert!(masked.value.chars().all(|c| c == '*'));
    }

    #[test]
    fn test_mask_patient_masks_telecom_and_identifiers() {
        let mut p = sample_patient();
        p.telecom.push(ContactPoint {
            system: ContactPointSystem::Email,
            value: "jane@example.com".into(),
            use_type: None,
        });
        p.telecom.push(ContactPoint {
            system: ContactPointSystem::Phone,
            value: "5551234567".into(),
            use_type: None,
        });
        p.identifiers
            .push(Identifier::mrn("urn:facility:1", "MRN-987654"));

        let masked = mask_patient(&p);
        assert!(masked.telecom[0].value.ends_with("@example.com"));
        assert!(!masked.telecom[0].value.starts_with("jane"));
        assert_eq!(masked.telecom[1].value, "******4567");
        // "MRN-987654" length 10, keep last 4
        assert_eq!(masked.identifiers[0].value, "******7654");
    }

    #[test]
    fn test_export_patient_is_json_object() {
        let p = sample_patient();
        let v = export_patient(&p);
        assert!(!v.is_null());
        assert!(v.is_object());
        let obj = v.as_object().unwrap();
        assert!(obj.contains_key("id"));
        assert!(obj.contains_key("name"));
        assert!(obj.contains_key("gender"));
    }
}
