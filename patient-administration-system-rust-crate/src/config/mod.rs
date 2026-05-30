//! config
//!
//! Environment-driven configuration for the Patient Administration System.
//!
//! [`Config::from_env`] reads runtime parameters from process environment
//! variables. `DATABASE_URL` is required; all other values have sensible
//! defaults so a development binary can boot with a single env var set.

use crate::{Error, Result};

/// Runtime configuration values for the PAS.
#[derive(Debug, Clone)]
pub struct Config {
    /// PostgreSQL connection URL (e.g. `postgres://user:pass@host:5432/dbname`).
    /// Required — `from_env` returns an error if it is unset.
    pub database_url: String,
    /// Host/IP the HTTP server binds to. Defaults to `0.0.0.0`.
    pub server_host: String,
    /// TCP port the HTTP server binds to. Defaults to `8080`.
    pub server_port: u16,
    /// Filesystem path where the Tantivy patient index lives.
    /// Defaults to `./search_index`.
    pub search_index_path: String,
    /// `RUST_LOG`-style filter for the tracing subscriber. Defaults to `info`.
    pub rust_log: String,
    /// Optional OTLP collector endpoint for OpenTelemetry export.
    /// `None` when the env var is unset — OTLP export is then disabled
    /// (only the fmt layer is wired). When set, [`observability::init`]
    /// builds an OTLP HTTP/protobuf exporter pointed at this URL and
    /// installs a `tracing_opentelemetry` layer alongside fmt.
    pub otlp_endpoint: Option<String>,
    /// `service.name` resource attribute attached to every exported span.
    /// Defaults to `pas-axum`. Override via `OTEL_SERVICE_NAME` when you
    /// need to disambiguate replicas / environments in the collector.
    pub otel_service_name: String,
    /// Optional API bearer token. When set, requests to `/api/*` must
    /// present `Authorization: Bearer <token>` matching this value. When
    /// `None`, the API runs in trusted-caller mode (any request is allowed).
    pub api_token: Option<String>,
    /// Comma-separated list of allowed CORS origins. When empty, CORS is
    /// permissive (any origin). Otherwise the layer mirrors only the listed
    /// origins.
    pub cors_origins: Vec<String>,
    /// Optional `host:port` bind address for the MLLP TCP listener. When
    /// set, the server spawns an additional listener that accepts MLLP-framed
    /// HL7 v2 messages and dispatches them through the same handler logic
    /// as `POST /api/hl7/v2/...`. Conventional MLLP port is 2575.
    pub hl7v2_mllp_bind: Option<String>,
    /// Optional `host:port` of a downstream MLLP receiver. When set, the
    /// outbox dispatcher emits ADT^A01/A02/A03 to this peer for the
    /// corresponding `EncounterAdmitted`/`Transferred`/`Discharged` events.
    pub hl7v2_outbound_peer: Option<String>,
    /// Per-event retry budget for the outbox dispatcher (v0.5). After the
    /// N-th consecutive failed publish the row is moved to
    /// `outbox_dead_letters` and stops being retried. Defaults to
    /// [`crate::streaming::dispatcher::DEFAULT_MAX_RETRIES`] (10). Set
    /// `PAS_OUTBOX_MAX_RETRIES=0` to disable dead-lettering (retry forever).
    pub pas_outbox_max_retries: u32,
    /// SMS provider selector (v0.8). Recognized values: `"none"` (the
    /// default — install [`crate::communication::NoopSmsProvider`], SMS
    /// auto-send is disabled, letters with `channel = Sms` stay
    /// `Pending`); `"log"` (install [`crate::communication::LogSmsProvider`]
    /// — every outbound message is logged at `tracing::info!(target =
    /// "pas::sms")` and the letter is flipped to `Sent`). Read from
    /// `PAS_SMS_PROVIDER`. Unknown values fall back to `"none"` with a
    /// startup `warn!`.
    pub pas_sms_provider: String,
    /// Per-IP rate-limit refill rate (v0.12). Sustained requests per
    /// minute the limiter will allow before throttling. `0` disables
    /// the limiter entirely — useful in dev / behind a load balancer
    /// that does its own rate-limiting. Default `600` (10 req/sec).
    /// Read from `PAS_RATE_LIMIT_RPM`.
    pub pas_rate_limit_rpm: u32,
    /// Per-IP rate-limit bucket capacity (v0.12). A client may fire
    /// `burst` requests back-to-back before the bucket drains;
    /// subsequent requests are gated to `pas_rate_limit_rpm`. Default
    /// `60`. Read from `PAS_RATE_LIMIT_BURST`. Ignored when
    /// `pas_rate_limit_rpm == 0`.
    pub pas_rate_limit_burst: u32,
    /// Outbox webhook destination URL (v0.14). When set,
    /// [`crate::streaming::WebhookEventPublisher`] POSTs every outbox
    /// event to this URL as `application/json`. When unset, no webhook
    /// fan-out happens. Composes with `hl7v2_outbound_peer` via
    /// [`crate::streaming::CompositePublisher`] when both are set. Read
    /// from `PAS_WEBHOOK_URL`.
    pub pas_webhook_url: Option<String>,
    /// Optional HMAC-SHA256 secret for the webhook publisher (v0.14).
    /// When set, every request carries an `X-PAS-Signature:
    /// sha256=<hex>` header signing the raw body. Receivers MUST
    /// verify this constant-time. Read from `PAS_WEBHOOK_SECRET`.
    pub pas_webhook_secret: Option<String>,
    /// Webhook request timeout in seconds (v0.14). Keep strictly
    /// shorter than the dispatcher poll interval (2 s base) for
    /// responsive retries; default `10`. Read from
    /// `PAS_WEBHOOK_TIMEOUT_SECS`. Ignored when `pas_webhook_url` is
    /// `None`.
    pub pas_webhook_timeout_secs: u64,
}

impl Config {
    /// Build a [`Config`] from process environment variables.
    ///
    /// `DATABASE_URL` is required; all other variables fall back to defaults.
    /// Returns [`Error::Config`] if `DATABASE_URL` is missing or unreadable.
    pub fn from_env() -> Result<Self> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| Error::config("DATABASE_URL is required"))?;
        let server_host = std::env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let server_port = std::env::var("SERVER_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8080);
        let search_index_path =
            std::env::var("SEARCH_INDEX_PATH").unwrap_or_else(|_| "./search_index".to_string());
        let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
        let otlp_endpoint = std::env::var("OTLP_ENDPOINT")
            .ok()
            .filter(|s| !s.is_empty());
        let otel_service_name = std::env::var("OTEL_SERVICE_NAME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "pas-axum".to_string());
        let api_token = std::env::var("API_TOKEN").ok().filter(|s| !s.is_empty());
        let cors_origins = std::env::var("CORS_ORIGINS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let hl7v2_mllp_bind = std::env::var("HL7V2_MLLP_BIND")
            .ok()
            .filter(|s| !s.is_empty());
        let hl7v2_outbound_peer = std::env::var("HL7V2_OUTBOUND_PEER")
            .ok()
            .filter(|s| !s.is_empty());
        let pas_outbox_max_retries = std::env::var("PAS_OUTBOX_MAX_RETRIES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(crate::streaming::dispatcher::DEFAULT_MAX_RETRIES);
        let pas_sms_provider = std::env::var("PAS_SMS_PROVIDER")
            .ok()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "none".to_string());
        let pas_rate_limit_rpm = std::env::var("PAS_RATE_LIMIT_RPM")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(600);
        let pas_rate_limit_burst = std::env::var("PAS_RATE_LIMIT_BURST")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        let pas_webhook_url = std::env::var("PAS_WEBHOOK_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let pas_webhook_secret = std::env::var("PAS_WEBHOOK_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        let pas_webhook_timeout_secs = std::env::var("PAS_WEBHOOK_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(crate::streaming::webhook::DEFAULT_TIMEOUT_SECS);
        Ok(Self {
            database_url,
            server_host,
            server_port,
            search_index_path,
            rust_log,
            otlp_endpoint,
            otel_service_name,
            api_token,
            cors_origins,
            hl7v2_mllp_bind,
            hl7v2_outbound_peer,
            pas_outbox_max_retries,
            pas_sms_provider,
            pas_rate_limit_rpm,
            pas_rate_limit_burst,
            pas_webhook_url,
            pas_webhook_secret,
            pas_webhook_timeout_secs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Environment variables are process-global. Serialize the tests that
    // mutate them so they cannot race against each other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Snapshot the env vars we touch so each test can restore the prior
    /// process state on the way out (so we don't pollute sibling tests).
    struct EnvGuard {
        keys: &'static [&'static str],
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn new(keys: &'static [&'static str]) -> Self {
            let saved = keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
            // Clear all of them to start from a known blank slate.
            for k in keys {
                // SAFETY: tests are serialized via ENV_LOCK.
                unsafe {
                    std::env::remove_var(k);
                }
            }
            Self { keys, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                // SAFETY: tests are serialized via ENV_LOCK.
                unsafe {
                    match v {
                        Some(val) => std::env::set_var(k, val),
                        None => std::env::remove_var(k),
                    }
                }
            }
            for k in self.keys {
                // Best-effort scrub anything the test set that wasn't saved.
                if !self.saved.iter().any(|(sk, _)| sk == k) {
                    // SAFETY: tests are serialized via ENV_LOCK.
                    unsafe {
                        std::env::remove_var(k);
                    }
                }
            }
        }
    }

    const ENV_KEYS: &[&str] = &[
        "DATABASE_URL",
        "SERVER_HOST",
        "SERVER_PORT",
        "SEARCH_INDEX_PATH",
        "RUST_LOG",
        "OTLP_ENDPOINT",
        "OTEL_SERVICE_NAME",
        "PAS_SMS_PROVIDER",
        "PAS_RATE_LIMIT_RPM",
        "PAS_RATE_LIMIT_BURST",
        "PAS_WEBHOOK_URL",
        "PAS_WEBHOOK_SECRET",
        "PAS_WEBHOOK_TIMEOUT_SECS",
    ];

    fn set(k: &str, v: &str) {
        // SAFETY: tests holding ENV_LOCK guarantee single-threaded access.
        unsafe { std::env::set_var(k, v) }
    }

    #[test]
    fn test_from_env_requires_database_url() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(ENV_KEYS);
        let err = Config::from_env().expect_err("missing DATABASE_URL should error");
        match err {
            Error::Config(msg) => assert!(msg.contains("DATABASE_URL"), "msg: {msg}"),
            other => panic!("expected Error::Config, got {other:?}"),
        }
    }

    #[test]
    fn test_from_env_populates_all_fields() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(ENV_KEYS);

        set("DATABASE_URL", "postgres://u:p@h:5432/pas");
        set("SERVER_HOST", "127.0.0.1");
        set("SERVER_PORT", "9090");
        set("SEARCH_INDEX_PATH", "/tmp/pas-search");
        set("RUST_LOG", "debug");
        set("OTLP_ENDPOINT", "http://otlp:4317");
        set("OTEL_SERVICE_NAME", "pas-axum-staging");

        let cfg = Config::from_env().expect("all vars set");
        assert_eq!(cfg.database_url, "postgres://u:p@h:5432/pas");
        assert_eq!(cfg.server_host, "127.0.0.1");
        assert_eq!(cfg.server_port, 9090);
        assert_eq!(cfg.search_index_path, "/tmp/pas-search");
        assert_eq!(cfg.rust_log, "debug");
        assert_eq!(cfg.otlp_endpoint.as_deref(), Some("http://otlp:4317"));
        assert_eq!(cfg.otel_service_name, "pas-axum-staging");
    }

    #[test]
    fn test_from_env_uses_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(ENV_KEYS);

        set("DATABASE_URL", "postgres://u:p@h:5432/pas");

        let cfg = Config::from_env().expect("DATABASE_URL is enough");
        assert_eq!(cfg.database_url, "postgres://u:p@h:5432/pas");
        assert_eq!(cfg.server_host, "0.0.0.0");
        assert_eq!(cfg.server_port, 8080);
        assert_eq!(cfg.search_index_path, "./search_index");
        assert_eq!(cfg.rust_log, "info");
        assert!(cfg.otlp_endpoint.is_none());
        // Service name has a default — it's always populated.
        assert_eq!(cfg.otel_service_name, "pas-axum");
        // SMS provider defaults to "none".
        assert_eq!(cfg.pas_sms_provider, "none");
        // Rate-limit defaults: 600 rpm, 60 burst.
        assert_eq!(cfg.pas_rate_limit_rpm, 600);
        assert_eq!(cfg.pas_rate_limit_burst, 60);
        // Webhook publisher disabled by default.
        assert!(cfg.pas_webhook_url.is_none());
        assert!(cfg.pas_webhook_secret.is_none());
        assert_eq!(cfg.pas_webhook_timeout_secs, 10);
    }

    #[test]
    fn test_from_env_parses_webhook_overrides() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(ENV_KEYS);

        set("DATABASE_URL", "postgres://u:p@h:5432/pas");
        set("PAS_WEBHOOK_URL", "https://hooks.example.com/pas");
        set("PAS_WEBHOOK_SECRET", "supersecret");
        set("PAS_WEBHOOK_TIMEOUT_SECS", "30");

        let cfg = Config::from_env().expect("env parses");
        assert_eq!(
            cfg.pas_webhook_url.as_deref(),
            Some("https://hooks.example.com/pas")
        );
        assert_eq!(cfg.pas_webhook_secret.as_deref(), Some("supersecret"));
        assert_eq!(cfg.pas_webhook_timeout_secs, 30);
    }

    #[test]
    fn test_from_env_parses_rate_limit_overrides() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(ENV_KEYS);

        set("DATABASE_URL", "postgres://u:p@h:5432/pas");
        set("PAS_RATE_LIMIT_RPM", "120");
        set("PAS_RATE_LIMIT_BURST", "20");

        let cfg = Config::from_env().expect("env parses");
        assert_eq!(cfg.pas_rate_limit_rpm, 120);
        assert_eq!(cfg.pas_rate_limit_burst, 20);
    }

    #[test]
    fn test_from_env_normalizes_sms_provider_value() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(ENV_KEYS);

        set("DATABASE_URL", "postgres://u:p@h:5432/pas");
        // Uppercase + extra whitespace must normalize cleanly.
        set("PAS_SMS_PROVIDER", "  LOG  ");

        let cfg = Config::from_env().expect("env parses");
        assert_eq!(cfg.pas_sms_provider, "log");
    }

    #[test]
    fn test_from_env_ignores_unparseable_port() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::new(ENV_KEYS);

        set("DATABASE_URL", "postgres://u:p@h:5432/pas");
        set("SERVER_PORT", "not-a-number");

        let cfg = Config::from_env().expect("falls back to default port");
        assert_eq!(cfg.server_port, 8080);
    }
}
