//! Minimal bearer-token middleware for the REST API.
//!
//! When an API token is configured, every request to `/api/*` must include
//! `Authorization: Bearer <token>` with a matching value. The `/api/health`
//! endpoint is exempt so liveness probes work without credentials.
//!
//! When no token is configured, the middleware is a no-op and the API runs
//! in trusted-caller mode (header-only user context). This matches the v0.1
//! posture documented in `plan.md`.

use axum::{
    Json,
    extract::Request,
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::api::ApiResponse;

/// Required token, supplied at construction time. Cloned into the middleware
/// closure so it can be checked per-request.
#[derive(Clone)]
pub struct RequireBearerToken {
    expected: String,
}

impl RequireBearerToken {
    pub fn new(expected: impl Into<String>) -> Self {
        Self {
            expected: expected.into(),
        }
    }
}

/// Axum 0.7 `from_fn_with_state`-compatible middleware function.
pub async fn require_bearer(
    state: axum::extract::State<RequireBearerToken>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    if path == "/api/health" {
        return next.run(req).await;
    }
    let supplied = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim());
    match supplied {
        Some(t) if t == state.0.expected => next.run(req).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<serde_json::Value>::err(
                "UNAUTHORIZED",
                "missing or invalid bearer token",
            )),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_require_bearer_token_construct() {
        let r = RequireBearerToken::new("abc");
        assert_eq!(r.expected, "abc");
    }
}
