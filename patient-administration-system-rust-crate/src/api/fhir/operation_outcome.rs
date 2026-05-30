//! FHIR `OperationOutcome` error envelope.
//!
//! Used by FHIR handlers to report errors in a way clinical clients expect.
//! This is intentionally a tiny subset of the FHIR R5 `OperationOutcome` —
//! `severity`, `code`, and a free-text `diagnostics` per issue.

use serde::{Deserialize, Serialize};

/// FHIR R5 `OperationOutcome` resource (minimal subset).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OperationOutcome {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    pub issue: Vec<Issue>,
}

/// A single issue within an `OperationOutcome`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Issue {
    /// `"error" | "warning" | "information"`.
    pub severity: String,
    /// FHIR issue code, e.g. `"invalid"`, `"not-found"`, `"exception"`.
    pub code: String,
    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<String>,
}

impl OperationOutcome {
    /// Build an error-severity `OperationOutcome` with one issue.
    pub fn error(code: &str, diagnostics: impl Into<String>) -> Self {
        Self {
            resource_type: "OperationOutcome".into(),
            issue: vec![Issue {
                severity: "error".into(),
                code: code.into(),
                diagnostics: Some(diagnostics.into()),
            }],
        }
    }

    /// Convenience for `code = "not-found"` errors.
    pub fn not_found(diagnostics: impl Into<String>) -> Self {
        Self::error("not-found", diagnostics)
    }

    /// Convenience for `code = "exception"` errors.
    pub fn exception(diagnostics: impl Into<String>) -> Self {
        Self::error("exception", diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_outcome_error_shape() {
        let oo = OperationOutcome::error("invalid", "bad UUID");
        assert_eq!(oo.resource_type, "OperationOutcome");
        assert_eq!(oo.issue.len(), 1);
        assert_eq!(oo.issue[0].severity, "error");
        assert_eq!(oo.issue[0].code, "invalid");
        assert_eq!(oo.issue[0].diagnostics.as_deref(), Some("bad UUID"));
    }

    #[test]
    fn test_operation_outcome_serializes_resource_type() {
        let oo = OperationOutcome::not_found("patient missing");
        let json = serde_json::to_string(&oo).expect("serialize");
        assert!(json.contains("\"resourceType\":\"OperationOutcome\""));
        assert!(json.contains("\"code\":\"not-found\""));
        assert!(json.contains("\"diagnostics\":\"patient missing\""));
    }
}
