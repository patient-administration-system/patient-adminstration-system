# Technology stack

Snapshot of the dependencies declared in this crate's `Cargo.toml`. The README has the same table with versions; this file mirrors it so AGENTS-only readers see the stack without context-switching. Update both together.

The v0.3.1 dep audit removed 15 unused direct deps (`loco-rs`, `fluvio`, `tonic`, `prost`, `tonic-build`, `openapiv3`, `jsonwebtoken`, `argon2`, `strsim`, `anyhow`, `bigdecimal`, `validator`, `hyper`, plus all five `opentelemetry*` crates). v0.3.4 removed `mockall` + `tokio-test` from dev-deps. v0.7.0 reintroduced four `opentelemetry*` crates (`opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, `tracing-opentelemetry`) when the OTLP exporter actually started being used. Anything below is actually compiled and used.

| Component            | Technology                                  | Purpose                                  |
| -------------------- | ------------------------------------------- | ---------------------------------------- |
| **Language**         | Rust 1.93+, 2024 Edition                    | Systems programming, performance, safety |
| **Async Runtime**    | Tokio (`full`)                              | Asynchronous I/O and concurrency         |
| **Web Framework**    | Axum 0.7 (`macros`)                         | HTTP server and routing                  |
| **HTTP middleware**  | tower (`util`), tower-http (`cors`, `trace`, `compression-full`) | CORS / trace / compression / `ServiceExt::oneshot` |
| **Templates**        | Tera 1.20                                   | Server-side rendering for letters + dashboard |
| **Database**         | PostgreSQL 18+                              | Persistence                              |
| **ORM**              | SeaORM 1.1                                  | Async ORM (sqlx-postgres, rustls runtime, chrono/uuid/json types) |
| **Migrations**       | sea-orm-migration 1.1 + `pas-migrate` CLI   | Schema management                        |
| **Search Engine**    | Tantivy 0.22                                | Full-text patient search                 |
| **Streaming**        | `InMemoryEventPublisher` (production + tests) + `Hl7v2MllpPublisher` (outbound ADT over MLLP) + `FluvioEventPublisher` stub | Event publishing |
| **API Docs**         | Utoipa 5.4 + `utoipa-swagger-ui` 8.1        | OpenAPI 3.1 annotations + Swagger UI     |
| **Serialization**    | Serde 1.0, serde_json 1.0, quick-xml 0.36 (`serialize`), csv 1.3 | JSON / XML / CSV / TSV |
| **Logging / Tracing**| `tracing` 0.1, `tracing-subscriber` 0.3 (`env-filter`, `json`) | Structured logging (reads `RUST_LOG`) |
| **Observability**    | OpenTelemetry 0.27 + OTLP HTTP/protobuf via reqwest (`opentelemetry`, `opentelemetry_sdk` with `rt-tokio`, `opentelemetry-otlp` with `http-proto` + `reqwest-client`, `tracing-opentelemetry` 0.28) | Wires when `OTLP_ENDPOINT` is set; v0.7.0. See [observability.md](observability.md) |
| **Utilities**        | uuid 1.19, chrono 0.4, dotenvy 0.15, async-trait 0.1 | UUIDs, timestamps, env, async traits |
| **Error Handling**   | thiserror 2.0                               | Typed errors                             |
| **Money / Decimal**  | rust_decimal 1.36 (`serde-with-str`)        | Exact monetary arithmetic                |
| **Testing**          | assertables 9.8, tempfile 3.24              | Rich asserts + temp directories          |
| **Benchmarking**     | Criterion 0.5 (`html_reports`, `async_tokio`) | Statistical performance benchmarks     |
| **Containerization** | Docker (multi-stage), docker-compose        | Deployment packaging                     |

## Workspace context

This crate is one member of a three-member Cargo workspace at the project root. The other members:

| Crate                                       | Purpose                                                          |
|---------------------------------------------|------------------------------------------------------------------|
| `patient-administration-system-migrations`  | sea-orm migration crate; binary `pas-migrate` (up/down/fresh/status). |
| `patient-administration-system-frontend`                              | Loco-rs 0.14.1 read-mostly UI (Tera + HTMX + axum-extra cookies + reqwest); shares this crate's PostgreSQL DB and migration crate by path. |

Workspace-level profile blocks (`[profile.release]` LTO + strip, `[profile.bench]` inherits release) live in the root `Cargo.toml`; per-member profile blocks are silently ignored by Cargo.
