# Observability

## What's wired today

- Structured logging via the `tracing` crate (`tracing-subscriber` 0.3 with `env-filter` + `json` features). The fmt layer (human-readable stdout) is always present.
- Log level controlled by `RUST_LOG` (default `info`). Filter syntax is the standard `tracing-subscriber::EnvFilter` directive set.
- Request/response logging via `tower_http::trace::TraceLayer::new_for_http()` wrapped around the merged router in `main.rs`.
- Error logging happens at the boundary where domain `Error` is converted to HTTP status; the outbox dispatcher logs failed publishes as `warn!` and leaves the row unpublished for the next tick.
- `src/observability/mod.rs::init(&Config)` is the wiring point — called once from `main.rs` at startup with the full `Config`.

## OpenTelemetry OTLP exporter (v0.7.0)

When `OTLP_ENDPOINT` is set, `observability::init` additionally wires a `tracing_opentelemetry::layer()` backed by an OTLP exporter:

- Transport: **HTTP/protobuf** via `reqwest` (no `tonic`). Standard OTLP endpoint shape is `http://<collector>:4318/v1/traces`.
- Tracer provider: batch span processor running on the tokio runtime. Span export is asynchronous and does not block the request path.
- Service identity: `OTEL_SERVICE_NAME` (default `pas-axum`) becomes the `service.name` resource attribute on every span. Override per replica or environment to disambiguate in the collector.
- Failures during setup (malformed endpoint, exporter build error) log a `warn!` and the server falls back to fmt-only — never fatal at startup.

Crates pulled in for this surface:

- `opentelemetry = "0.27"`
- `opentelemetry_sdk = "0.27"` (with the `rt-tokio` feature)
- `opentelemetry-otlp = "0.27"` (`default-features = false`, with `http-proto` + `reqwest-client`)
- `tracing-opentelemetry = "0.28"`

All four were removed in the v0.3.1 dep audit because nothing was using them at that point; v0.7.0 reintroduced them because they now are.

## Process-level

- The MLLP listener (when `HL7V2_MLLP_BIND` is set) logs `accepted` / `frame received` / `dispatch error` lines at `info!`/`warn!`.
- The outbox dispatcher logs `tick` summaries (`fetched=N, published=K`) so an operator can watch publish health in real time. v0.5 dead-letter moves are logged at `warn!` with the final error and the moved row id.
