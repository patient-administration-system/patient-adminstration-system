//! CSRF protection — double-submit cookie pattern.
//!
//! Threat model: a third-party site tricks the admin's browser into
//! submitting one of the write-flow forms (admit, book, compose). We
//! defend with the standard double-submit-cookie pattern:
//!
//! 1. On every form GET, [`ensure_token`] reads (or, if absent,
//!    generates) a `pas_csrf` cookie with `SameSite=Strict; HttpOnly`
//!    (and `Secure` when [`cookie_secure`] returns true).
//! 2. The form embeds the same token as a hidden `csrf_token` field
//!    (server-side render via Tera).
//! 3. On submit, [`verify_token`] requires the cookie value and the
//!    posted form value to match (constant-time compare).
//!
//! `SameSite=Strict` + `HttpOnly` mean a third-party site can neither
//! read the cookie nor cause the browser to send it on a cross-origin
//! POST, so the attacker can't supply a matching `csrf_token` field.
//! The `Secure` flag (production only) blocks the cookie from ever
//! travelling over plaintext HTTP — set `PAS_COOKIE_SECURE=1` in any
//! environment terminated behind HTTPS.

use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use loco_rs::{Error, Result};
use uuid::Uuid;

/// Name of the cookie that carries the CSRF token.
pub const COOKIE_NAME: &str = "pas_csrf";

/// Name of the form field every protected form must include.
pub const FORM_FIELD: &str = "csrf_token";

/// Env var that toggles the `Secure` cookie flag. Defaults to **off** so
/// local-dev over plaintext HTTP still works; production deployments
/// (HTTPS) must set this to `1` / `true` / `yes`.
pub const SECURE_ENV: &str = "PAS_COOKIE_SECURE";

/// True iff the `Secure` cookie flag should be set on `pas_csrf`. Reads
/// [`SECURE_ENV`] each call (cheap; std env access is a Mutex around a
/// HashMap lookup) so tests can flip it via `std::env::set_var`.
pub fn cookie_secure() -> bool {
    matches!(
        std::env::var(SECURE_ENV).as_deref(),
        Ok("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

/// Return the CSRF token attached to this jar, generating + setting a
/// fresh one if no cookie is present. The returned [`CookieJar`] must be
/// returned to axum from the handler so the `Set-Cookie` header reaches
/// the browser.
pub fn ensure_token(jar: CookieJar) -> (String, CookieJar) {
    if let Some(c) = jar.get(COOKIE_NAME) {
        let value = c.value().to_string();
        if !value.is_empty() {
            return (value, jar);
        }
    }
    let token = Uuid::new_v4().simple().to_string();
    let cookie = Cookie::build((COOKIE_NAME, token.clone()))
        .path("/")
        .http_only(true)
        .secure(cookie_secure())
        .same_site(SameSite::Strict)
        .build();
    (token, jar.add(cookie))
}

/// Validate that the supplied form `csrf_token` matches the cookie value.
/// Returns [`Error::BadRequest`] on mismatch.
pub fn verify_token(jar: &CookieJar, form_token: &str) -> Result<()> {
    let cookie_value = jar
        .get(COOKIE_NAME)
        .map(|c| c.value().to_string())
        .unwrap_or_default();
    if cookie_value.is_empty() || !constant_time_eq(cookie_value.as_bytes(), form_token.as_bytes())
    {
        return Err(Error::BadRequest("CSRF token mismatch".into()));
    }
    Ok(())
}

/// Byte-by-byte equality without short-circuiting on length or content.
/// Length mismatch is a fast-fail (which leaks the length, but the token
/// length is a constant so that's fine).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq_match() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_constant_time_eq_mismatch() {
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abc", b""));
    }

    #[test]
    fn test_ensure_token_generates_when_missing() {
        let jar = CookieJar::new();
        let (token, jar) = ensure_token(jar);
        assert_eq!(token.len(), 32, "uuid simple format is 32 hex chars");
        assert_eq!(
            jar.get(COOKIE_NAME).map(|c| c.value()),
            Some(token.as_str())
        );
    }

    #[test]
    fn test_ensure_token_reuses_existing() {
        let jar = CookieJar::new().add(
            Cookie::build((COOKIE_NAME, "existing-token-value"))
                .path("/")
                .build(),
        );
        let (token, _jar) = ensure_token(jar);
        assert_eq!(token, "existing-token-value");
    }

    #[test]
    fn test_verify_token_accepts_match() {
        let jar = CookieJar::new().add(Cookie::build((COOKIE_NAME, "abc123")).path("/").build());
        assert!(verify_token(&jar, "abc123").is_ok());
    }

    #[test]
    fn test_verify_token_rejects_mismatch() {
        let jar = CookieJar::new().add(Cookie::build((COOKIE_NAME, "abc123")).path("/").build());
        assert!(verify_token(&jar, "abc124").is_err());
    }

    #[test]
    fn test_verify_token_rejects_missing_cookie() {
        let jar = CookieJar::new();
        assert!(verify_token(&jar, "abc123").is_err());
    }

    #[test]
    fn test_verify_token_rejects_empty_form() {
        let jar = CookieJar::new().add(Cookie::build((COOKIE_NAME, "abc123")).path("/").build());
        assert!(verify_token(&jar, "").is_err());
    }

    // The `cookie_secure` env-var helper is exercised end-to-end in
    // `test_ensure_token_sets_secure_when_env_truthy` below. We avoid
    // asserting `cookie_secure() == false` in isolation because
    // `std::env` is process-global and another concurrent test in the
    // same binary could flip the var.

    #[test]
    fn test_ensure_token_sets_secure_when_env_truthy() {
        // Save + restore the env so this test stays independent.
        let prev = std::env::var(SECURE_ENV).ok();
        unsafe {
            std::env::set_var(SECURE_ENV, "1");
        }
        let (_token, jar) = ensure_token(CookieJar::new());
        let cookie = jar.get(COOKIE_NAME).expect("cookie must be set");
        assert_eq!(cookie.secure(), Some(true), "Secure flag must be set");
        unsafe {
            match prev {
                Some(v) => std::env::set_var(SECURE_ENV, v),
                None => std::env::remove_var(SECURE_ENV),
            }
        }
    }
}
