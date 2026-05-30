# Patient Administration System

Cargo workspace housing the PAS API server and its Loco-rs front-end.

## Layout

```
.
├── Cargo.toml                                 workspace manifest
├── patient-administration-system-rust-crate/  PAS Axum API server (v0.43.0)
│   ├── Cargo.toml
│   ├── src/
│   ├── tests/
│   ├── migrations/                            sea-orm migrations crate
│   ├── README.md
│   ├── CHANGELOG.md
│   └── spec.md
└── patient-administration-system-frontend/                              Loco-rs front-end (v0.1.0)
    ├── Cargo.toml
    ├── src/                                   incl. csrf.rs middleware
    ├── assets/views/                          Tera + Lily + HTMX
    ├── config/
    ├── tests/                                 dashboard_smoke.rs
    ├── README.md
    └── CHANGELOG.md
```

## Workspace commands

```bash
cargo build --workspace
cargo test --workspace --lib
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## What each crate does

| Crate | Role | Port | Stack |
|-------|------|------|-------|
| `patient-administration-system` | System of record. REST + FHIR R5 + HL7 v2 over HTTP and MLLP. Audit, outbox, search, scheduling. | `8080` | Axum, sea-orm, Tantivy, utoipa |
| `patient-administration-system-frontend` | Web UI. Server-side rendered Tera with HTMX live-refresh and Lily Design System markup. Read-mostly view layer over the shared database. | `5150` | Loco-rs, Tera, HTMX, Lily |
| `patient-administration-system-migrations` | sea-orm migration crate. Shared between the two app crates. | n/a | sea-orm-migration |

## Quick start

```bash
# 1. Start the database.
cd patient-administration-system-rust-crate
docker compose up -d postgres

# 2. Apply migrations once — both apps see the same schema.
DATABASE_URL=postgres://pas:pas_dev_password@localhost:5432/pas \
    cargo run --bin pas-migrate -- up

# 3. (Optional) Seed demo rows.
DATABASE_URL=… cargo run --bin pas-seed

# 4. Start the PAS Axum API on :8080.
DATABASE_URL=… cargo run --bin patient-administration-system

# 5. In another terminal, start the Loco front-end on :5150.
cd ../patient-administration-system-frontend
DATABASE_URL=… cargo run -- start
```

Then browse to:
- `http://localhost:8080/swagger-ui` — OpenAPI explorer for the API
- `http://localhost:8080/dashboard` — Axum-served ops dashboard
- `http://localhost:5150/dashboard` — Loco-served ops dashboard (parallel)
- `http://localhost:5150/patients` — patient list + detail pages

## See also

- [`CHANGELOG.md`](CHANGELOG.md) — workspace-level release history.
- [`patient-administration-system-rust-crate/README.md`](patient-administration-system-rust-crate/README.md) — full PAS API documentation, endpoint reference, configuration, status.
- [`patient-administration-system-frontend/README.md`](patient-administration-system-frontend/README.md) — Loco-rs architecture notes + route table + CSRF design.
- [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md) — PAS Axum release history.
- [`patient-administration-system-frontend/CHANGELOG.md`](patient-administration-system-frontend/CHANGELOG.md) — patient-administration-system-frontend release history.

## License

MIT or Apache-2.0 or BSD-3-Clause or GPL-2.0 or GPL-3.0.
