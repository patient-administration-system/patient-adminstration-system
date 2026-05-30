//! observability
//!
//! Tracing initialization for the Patient Administration System.
//!
//! [`init`] configures the global [`tracing`] subscriber from a `RUST_LOG`-
//! style filter string. The fmt layer (local stdout in human-readable form)
//! is always present.
//!
//! Additionally, when `OTLP_ENDPOINT` is set, a second layer is wired
//! (v0.7.0):
//!
//! - **OpenTelemetry layer** — every `tracing` span is bridged into an OTel
//!   `Tracer` backed by an OTLP exporter (HTTP/protobuf transport via
//!   reqwest, no tonic). Service name comes from `OTEL_SERVICE_NAME`
//!   (default `pas-axum`). The tracer provider uses the tokio runtime,
//!   so span export does not block the request path.
//!
//! Setup failures (bad endpoint, network unreachable at start, etc.) are
//! logged at `warn!` and the function falls back to fmt-only — never fatal
//! at startup. The shipped service must always come up; observability is
//! a best-effort cross-cutting concern.

use crate::Result;
use crate::config::Config;
use opentelemetry::KeyValue;
use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::TracerProvider;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

/// Initialize the global tracing subscriber.
///
/// `cfg.rust_log` is parsed as an [`EnvFilter`] directive (e.g. `info`,
/// `pas=debug,sea_orm=warn`). If the directive fails to parse, the
/// subscriber falls back to `info`.
///
/// When `cfg.otlp_endpoint` is `Some(url)` and the OTLP exporter sets up
/// successfully, a `tracing_opentelemetry` layer is installed alongside
/// the fmt layer; spans land in the configured collector. On any setup
/// failure (parse error, exporter build error, etc.), the function logs
/// a warning and continues with the fmt layer only.
///
/// Subsequent calls are no-ops: `try_init` returns an error when a
/// global subscriber is already installed and we swallow it intentionally
/// so the function is safe to call from tests and from `main`.
pub fn init(cfg: &Config) -> Result<()> {
    let filter = EnvFilter::try_new(&cfg.rust_log).unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = fmt::layer();

    match cfg.otlp_endpoint.as_deref() {
        Some(endpoint) => match build_otlp_layer(endpoint, &cfg.otel_service_name) {
            Ok(otel_layer) => {
                let _ = tracing_subscriber::registry()
                    .with(filter)
                    .with(fmt_layer)
                    .with(otel_layer)
                    .try_init();
                tracing::info!(
                    "OpenTelemetry OTLP exporter wired: endpoint={endpoint}, \
                     service.name={}",
                    cfg.otel_service_name
                );
            }
            Err(e) => {
                let _ = tracing_subscriber::registry()
                    .with(filter)
                    .with(fmt_layer)
                    .try_init();
                tracing::warn!(
                    "OpenTelemetry OTLP setup failed (endpoint={endpoint}): {e}. \
                     Falling back to fmt-only tracing."
                );
            }
        },
        None => {
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(fmt_layer)
                .try_init();
        }
    }
    Ok(())
}

/// Build the OTel `tracing` layer backed by an OTLP HTTP/protobuf
/// exporter.
///
/// Returns `Err` (with a human diagnostic) if the exporter cannot be
/// constructed — for example because `endpoint` is malformed or the HTTP
/// client init fails. Network failures at *send* time do not surface
/// here; the OTel SDK silently drops spans when the collector is
/// unreachable.
///
/// The return type is `impl Layer<S>` so the caller can compose it onto
/// any subscriber chain — pinning a concrete `S` (e.g. `Registry`) would
/// reject the `Layered<EnvFilter, Registry>` chain that `init` builds.
fn build_otlp_layer<S>(
    endpoint: &str,
    service_name: &str,
) -> std::result::Result<impl tracing_subscriber::Layer<S>, String>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .with_protocol(Protocol::HttpBinary)
        .build()
        .map_err(|e| format!("build OTLP exporter: {e}"))?;
    let resource = Resource::new(vec![KeyValue::new(
        "service.name",
        service_name.to_string(),
    )]);
    let provider = TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(resource)
        .build();
    let tracer = provider.tracer(service_name.to_string());
    // Install the provider globally so downstream `global::tracer(...)` calls
    // hit the same exporter chain.
    global::set_tracer_provider(provider);
    Ok(tracing_opentelemetry::layer().with_tracer(tracer))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_for_test(otlp: Option<&str>) -> Config {
        Config {
            database_url: "postgres://test".into(),
            server_host: "0.0.0.0".into(),
            server_port: 8080,
            search_index_path: "./search_index".into(),
            rust_log: "info".into(),
            otlp_endpoint: otlp.map(String::from),
            otel_service_name: "pas-axum-test".into(),
            api_token: None,
            cors_origins: vec![],
            hl7v2_mllp_bind: None,
            hl7v2_outbound_peer: None,
            pas_outbox_max_retries: 10,
            pas_sms_provider: "none".into(),
            pas_rate_limit_rpm: 600,
            pas_rate_limit_burst: 60,
            pas_webhook_url: None,
            pas_webhook_secret: None,
            pas_webhook_timeout_secs: 10,
        }
    }

    #[test]
    fn test_init_without_otlp_returns_ok() {
        // Without OTLP_ENDPOINT, init does only fmt-layer setup. Safe to
        // call repeatedly — try_init's Err is swallowed.
        let cfg = cfg_for_test(None);
        assert!(init(&cfg).is_ok());
        assert!(init(&cfg).is_ok(), "second call must also succeed");
    }

    #[tokio::test]
    async fn test_init_with_otlp_endpoint_does_not_panic() {
        // The reqwest-backed exporter wants a tokio runtime to build, so this
        // test must run under `#[tokio::test]`. We hand it a syntactically-
        // valid URL — the exporter constructs cleanly even though the
        // collector at that address may not be running. We only care that
        // init returns Ok and doesn't poison the global subscriber.
        let cfg = cfg_for_test(Some("http://127.0.0.1:4318/v1/traces"));
        assert!(init(&cfg).is_ok());
    }
}
