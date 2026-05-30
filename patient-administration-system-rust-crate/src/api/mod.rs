//! api

pub mod dashboard;
pub mod fhir;
pub mod openapi;
pub mod rest;

pub use openapi::ApiDoc;

use serde::{Deserialize, Serialize};

/// Envelope for every JSON response from the PAS REST API.
///
/// Success responses set `success = true` and populate `data`. Error responses
/// set `success = false` and populate `error`. Callers can dispatch on
/// `success` without unwrapping HTTP status codes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<ApiError>,
}

/// Machine-readable error payload carried in [`ApiResponse::error`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl<T> ApiResponse<T> {
    /// Build a success response carrying `data`.
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    /// Build an error response with the given machine-readable code and
    /// human-readable message.
    pub fn err(code: &str, message: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(ApiError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}
