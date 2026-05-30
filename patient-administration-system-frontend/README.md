# patient-administration-system-frontend

Loco-rs front-end for the Patient Administration System (PAS).

Stack: **Loco-rs + Tera + HTMX + [Lily Design System](https://github.com/LilyDesignSystem/lily)**.

## What this is

A separate web app — *not* a replacement for the existing PAS Axum API server. The two work together:

| Role | Crate | Port | Owns |
|------|-------|------|------|
| System of record (REST + FHIR + HL7 v2) | `patient-administration-system` (Axum) | `8080` | All writes, audit, outbox dispatcher |
| Human-facing UI | `patient-administration-system-frontend` (Loco-rs) | `5150` | Dashboard, patient pages, server-side Tera, HTMX live-refresh |

Both binaries share:
- The same PostgreSQL database
- The same migration crate (`patient-administration-system-migrations`) — apply migrations once via `pas-migrate up` and both apps see the schema

## Layout

```
patient-administration-system-frontend/
├── Cargo.toml              loco-rs + axum-extra + sea-orm + reqwest + …
├── config/
│   ├── development.yaml    bind 5150, dev Postgres
│   ├── production.yaml     env-driven config for prod
│   └── test.yaml
├── src/
│   ├── main.rs             loco_rs::cli::main entry
│   ├── lib.rs              module declarations
│   ├── app.rs              `Hooks` impl — routes, name, version
│   ├── csrf.rs             double-submit-cookie middleware for POST forms
│   ├── controllers/
│   │   ├── dashboard.rs    full page + 3 HTMX fragment endpoints
│   │   ├── patients.rs     list + detail
│   │   ├── wards.rs        ward detail
│   │   ├── rtt.rs          RTT cockpit (active pathways, breaches)
│   │   ├── admissions.rs   GET form + POST submit (calls PAS API)
│   │   ├── appointments.rs GET form + POST submit
│   │   ├── letters.rs      GET compose form + POST generate
│   │   └── health.rs       /_health
│   ├── models/             read-only sea-orm projections
│   ├── services/pas_api.rs reqwest client for PAS REST writes
│   ├── views/              Rust-side template helpers (currently empty)
│   └── initializers/       boot-time wiring (currently empty)
├── assets/
│   ├── views/
│   │   ├── layouts/main.html
│   │   ├── dashboard/{index,_wards,_outbox,_audit}.html
│   │   ├── patients/{index,show}.html
│   │   ├── wards/show.html
│   │   ├── rtt/index.html
│   │   ├── admissions/new.html
│   │   ├── appointments/new.html
│   │   └── letters/new.html
│   └── static/                   served at /static/...
└── tests/
    └── dashboard_smoke.rs        13 integration tests (gated on DATABASE_URL)
```

## Run

```bash
# 1. Apply migrations once via the existing PAS migration CLI.
cd ../patient-administration-system-rust-crate
DATABASE_URL=postgres://pas:pas_dev_password@localhost:5432/pas \
  cargo run --bin pas-migrate -- up

# 2. Boot the Loco front-end.
cd ../patient-administration-system-frontend
DATABASE_URL=postgres://pas:pas_dev_password@localhost:5432/pas \
  cargo run -- start

# Browse to http://localhost:5150/dashboard
```

## Routes

| Method | Path | Description |
|--------|------|-------------|
| GET    | `/_health` | Liveness probe (`{ service, version, database }`) |
| GET    | `/dashboard` | Ops dashboard — full page render |
| GET    | `/dashboard/wards` | HTMX fragment: ward occupancy |
| GET    | `/dashboard/outbox` | HTMX fragment: unpublished outbox count |
| GET    | `/dashboard/audit` | HTMX fragment: last 10 audit rows |
| GET    | `/patients` | Patient list |
| GET    | `/patients/{id}` | Patient detail |
| GET    | `/wards/{id}` | Ward detail (beds, occupants) |
| GET    | `/rtt` | RTT cockpit — active pathways with weeks-waiting + breach flags |
| GET    | `/admissions/new` | Admission form (patient + bed pickers) |
| POST   | `/admissions/new` | Submit admission (CSRF-protected; calls `POST /api/admissions`) |
| GET    | `/appointments/new` | Booking form (slot + patient pickers) |
| POST   | `/appointments/new` | Submit booking (CSRF-protected; calls `POST /api/slots/{id}/book`) |
| GET    | `/letters/new` | Letter compose form (template + patient + channel pickers) |
| POST   | `/letters/new` | Generate letter (CSRF-protected; calls `POST /api/letters/generate`) |

All write flows (POST endpoints) go through the PAS Axum API so audit + outbox events land in the system of record. patient-administration-system-frontend never writes directly.

## Environment variables

| Var | Default | Purpose |
|-----|---------|---------|
| `DATABASE_URL` | from `config/*.yaml` | PostgreSQL connection string (same DB as PAS Axum) |
| `PAS_API_URL` | `http://localhost:8080` | Base URL for the PAS Axum API — every write flow routes through it |
| `PAS_COOKIE_SECURE` | unset (off) | Set to `1` / `true` / `yes` in production so the `pas_csrf` cookie carries the `Secure` flag (HTTPS-only). Off in dev so plaintext-HTTP localhost works. |

## Security — CSRF on write forms

The three POST routes are protected by a double-submit-cookie pattern in `src/csrf.rs`:

1. On every form `GET`, the handler reads or mints a `pas_csrf` cookie (`SameSite=Strict; HttpOnly; Path=/`, value = 128-bit UUID) and embeds the same token as a hidden `csrf_token` input.
2. On `POST`, the handler compares the form field against the cookie via a constant-time byte comparison. Any mismatch (or missing cookie) returns HTTP 400 *before* any side effects.
3. In production, set `PAS_COOKIE_SECURE=1` so the cookie also carries the `Secure` flag.

`SameSite=Strict` + `HttpOnly` mean a third-party site can neither read the cookie nor cause it to be sent on a cross-origin POST, so an attacker has no way to supply a matching token.

## Design system

Every template uses Lily Design System headless HTML class names. Lily provides:

- `header` / `footer` for page chrome
- `panel` (with `role="region"`) for dashboard cards
- `data-table` / `data-table-head` / `data-table-body` / `data-table-row` / `data-table-th` / `data-table-td` for tables
- `badge` (with `data-status="ok|warn|error"`) for status indicators
- `alert` (with `role="status"`) for inline diagnostics + empty states
- `code` for inline code

Lily is *headless* — zero CSS — so all the styling lives in `assets/views/layouts/main.html`'s `<style>` block. Swap that block for Tailwind, a Sass build, or any other styling without touching the templates.

## Why a separate crate

- Lets the PAS Axum binary stay focused on the API/integration surface (~85 REST routes, FHIR R5, HL7 v2 over HTTP + MLLP) without growing a templating dependency tree.
- The Loco app can be deployed independently (different replicas, different scaling profile).
- Front-end iteration doesn't risk the integration test gauntlet (`cargo test --lib` in the PAS crate stays at 308 passing).

## Status — v0.1.0

**Workspace gates all green**: `cargo build --workspace --all-targets`, `cargo test --workspace --lib`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check --all`. Tested against Loco 0.14.1 with sea-orm 1.1.

- 9 csrf unit tests + 13 integration smoke tests in `tests/dashboard_smoke.rs` (gated on `DATABASE_URL` so `cargo test` works offline).
- Dep audit clean — no unused direct or dev dependencies.

Loco-side patterns the scaffold uses:
- **`Hooks` trait** fully implemented in `src/app.rs` (incl. `boot`, `routes`, `register_tasks`, `connect_workers`, `truncate`, `seed`). The DB-mutating hooks (`truncate`, `seed`) are intentionally no-ops — patient-administration-system-frontend never mutates the shared PAS schema.
- **`ViewEngine<TeraView>` extractor** on every page handler: `pub async fn index(ViewEngine(v): ViewEngine<TeraView>, State(ctx): State<AppContext>) -> Result<Response>`.
- **`format::render().view(&v, "template.html", data)`** with `data` as `serde_json::json!({...})`. Tera reads the template from `assets/views/`.
- **`Routes::new().prefix("/x").add("/y", get(handler))`** for the routing DSL; routes are aggregated in `Hooks::routes`.
- **Crate-level `#![allow(clippy::result_large_err)]`** in `src/lib.rs` and `src/main.rs` because `loco_rs::Error` is a wide enum that trips the lint on every handler signature.

Not in v0.1.0:
- No login / session middleware. Add via `axum-login` or Loco's auth helpers when you need it. CSRF is in place so the foundation for authenticated write flows is ready.
- Static asset folder (`assets/static/`) is empty — drop a favicon / custom CSS file in if you want one.

## See also

- [`../README.md`](../README.md) — workspace-level README.
- [`../CHANGELOG.md`](../CHANGELOG.md) — workspace-level changelog coordinating member-crate releases.
- [`../patient-administration-system-rust-crate/AGENTS.md`](../patient-administration-system-rust-crate/AGENTS.md) — orientation for the PAS Axum API server; the system-of-record contract (response envelopes, audit log, outbox events) that every write flow here proxies into.
- [`../patient-administration-system-rust-crate/AGENTS/restful.md`](../patient-administration-system-rust-crate/AGENTS/restful.md) — REST shape that this front-end's reqwest client consumes (`POST /api/admissions`, `POST /api/slots/{id}/book`, `POST /api/letters/generate`).
