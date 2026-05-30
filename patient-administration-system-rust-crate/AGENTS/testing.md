# Testing Strategy and Guide

## Overview

The Patient Administration System (PAS) uses a multi-layer testing strategy: unit tests inside the library, integration tests against a real Postgres, and Criterion benchmarks.

## Unit Tests

Run with:

```bash
cargo test --lib
```

**Current count: 497 passing tests, 0 failed, 0 ignored.**

Unit tests are embedded in source files using `#[cfg(test)] mod tests`. They test individual functions and modules without external dependencies (no database, no network). The full suite completes well under a second.

### Coverage by area

| Area                                                                                       | What's covered                                                                |
| ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------- |
| `models::*` (14 model files)                                                               | Constructor defaults, serde round-trip, state-machine transitions, factories  |
| `db::repositories::*`                                                                      | CRUD against a SeaORM mock connection where mockable                          |
| Services (`adt`, `scheduling`, `waitlist`, `rtt`, `billing`, `communication`, `resources`) | Happy-path business rules, state transitions, invariants                      |
| `hl7v2`                                                                                    | Parser, encoder, PID↔Patient mapping, ACK builder, escape sequences, MLLP frame round-trip |
| `interchange`                                                                              | `PatientRow` projection, JSON / XML / TSV / CSV serializers and parsers       |
| `api::dashboard`                                                                           | Tera template renders, Lily class names, HTMX wiring (`hx-get` URLs, poll cadence) |
| `api::openapi`                                                                             | `ApiDoc` includes every annotated path + every component schema               |
| `validation`                                                                               | E.164 phone, address rules, charge amount, appointment times, RTT ordering, **national healthcare identifiers** (NHS Mod 11, NIR Mod 97 + Corsica fix-up, TSI envelope, IHI, HCN — v0.15) |
| `privacy`                                                                                  | Value masking, patient masking, GDPR export, consent gating                   |
| `search`                                                                                   | Tantivy schema, index, query, fuzzy                                           |
| `streaming`                                                                                | InMemory publish/subscribe; Fluvio stub returns the expected `Streaming` error; `Hl7v2MllpPublisher` encoder paths |
| `config`, `observability`, `error`, `api`, `api::rest::auth`                               | Env loading, defaults, response envelope, bearer middleware constructor       |

The "models" share dominates because every model file ships a `#[cfg(test)]` block exercising at minimum: constructor defaults, serde JSON round-trip, and (where applicable) every legal and illegal state-machine transition.

### Running specific tests

```bash
cargo test --lib                                  # All unit tests
cargo test --lib models::rtt::                    # All RTT model tests
cargo test --lib test_compute_active_weeks_       # By name prefix
cargo test --lib -- --nocapture                   # Show stdout
```

## Integration Tests

Run with:

```bash
cargo test                                        # gated; skips without DATABASE_URL
DATABASE_URL=postgres://… cargo test              # actually exercise them
```

**24 integration test files; 54 test functions total.** All tests share `tests/common/mod.rs`, which exposes `database_url()` and `build_state(url)`. Every test that needs a DB calls `database_url()` first and skips silently when `DATABASE_URL` is unset, so `cargo test` in CI without Postgres remains green (just shorter). The DB-free files are `auth_test.rs` (bearer middleware), `rate_limit_test.rs` (per-IP token-bucket middleware), and `webhook_test.rs` (v0.14 — webhook publisher + composite fan-out, exercised against an in-process `TcpListener`).

| File                              | DB | What's tested                                                                                  |
| --------------------------------- | -- | ---------------------------------------------------------------------------------------------- |
| `tests/health_test.rs`            | y  | `/api/health` returns `{ status, database }`                                                   |
| `tests/auth_test.rs`              | n  | Bearer middleware (4 tests): exempt health, missing → 401, wrong → 401, correct → 200          |
| `tests/rate_limit_test.rs`        | n  | Per-IP token-bucket middleware (2 tests): burst exhaustion → 429 + Retry-After + `RATE_LIMITED`; `/api/health` exempt |
| `tests/webhook_test.rs`           | n  | v0.14: `WebhookEventPublisher` end-to-end POST against an in-process `TcpListener` receiver — body shape, `X-PAS-Event-Id` / `X-PAS-Event-Type` / `X-PAS-Signature` headers; `CompositePublisher` fan-out across webhook + in-memory (2 tests) |
| `tests/patient_crud_test.rs`      | y  | Create → get → search → update → list → masked → soft-delete → audit trail asserts             |
| `tests/practitioner_test.rs`      | y  | Create → get → list → update → soft-delete (active flip) → list excludes                       |
| `tests/adt_flow_test.rs`          | y  | Admit → ward-occupancy → transfer → discharge with bed-status assertions at each step          |
| `tests/scheduling_flow_test.rs`   | y  | Book → check-in → complete with slot status assertion                                          |
| `tests/waitlist_rtt_flow_test.rs` | y  | Add → start → pause → resume → stop → `GET /api/rtt/:id/weeks-waiting` + list pathways         |
| `tests/billing_flow_test.rs`      | y  | Open account → re-open conflict → 2 charges → finalize invoice (`200.50`) → partial payments   |
| `tests/letter_flow_test.rs`       | y  | Template create → generate → fetch → missing-variable 400 → list. v0.8: SMS auto-send with `LogSmsProvider` flips status to `sent` + stamps `sent_at`; patient with no phone leaves the letter `pending` (2 tests) |
| `tests/consent_flow_test.rs`      | y  | Two consents → list active → revoke one → mixed status                                         |
| `tests/fhir_write_test.rs`        | y  | Patient + Practitioner + Schedule + Slot CRUD (5 tests): POST → GET → PUT (id preserved) → DELETE; Practitioner DELETE flips `active=false`; Schedule + Slot DELETE hard-delete (v0.21); 404 on PUT/DELETE of unknown id; invalid body → `OperationOutcome` |
| `tests/fhir_bundle_test.rs`       | y  | `POST /fhir` batch + transaction (4 tests): per-entry response shape; Patient transaction rollback; v0.13 — Coverage batch entries (insurance + self-pay); v0.13 — Coverage parse failure rolls a transaction back |
| `tests/concurrency_test.rs`       | y  | Two-way races (bed + slot, 2 tests): exactly one 200, one 409 — proves `SELECT … FOR UPDATE`   |
| `tests/outbox_dispatcher_test.rs` | y  | Happy path: admit writes outbox row → one `dispatcher::tick` → row marked `published=true`. v0.5: failing publisher → `retry_count` rises to budget → row moves to `outbox_dead_letters` → replay puts a fresh row back with `retry_count = 0` → next tick drains it (2 tests) |
| `tests/interchange_test.rs`       | y  | Bulk import in all 4 formats → idempotent skip on re-import → exports round-trip → Tantivy visibility |
| `tests/hl7v2_test.rs`             | y  | HL7 v2 ADT^A28/A01/A02/A03 + A08/A11/A13/A40 + batch envelope + SIU^S12/S13/S14/S15 + DFT^P03 (13 tests): AA happy paths, AE on malformed payload, AR on parse failure, exact-MRN dedup, A08 demographic merge, A11 cancel-admit drops ward occupancy to 0, A13 cancel-discharge reinstates the original bed, batch endpoint dispatches 3 messages independently with mixed AA/AE inside one BTS envelope; v0.16 — SIU^S12 books an appointment (filler uuid returned in AA, overlap → 409), SIU^S15 cancels (double-cancel → 409, unknown filler → 404, non-UUID → 400, missing SCH-2 → 400); v0.17 — SIU^S13 reschedules with overlap-excluding check (own row safe, sibling conflict → 409), SIU^S14 modifies reason; terminal-status conflict, unknown filler, missing SCH-2, missing SCH-11, wrong-trigger-at-endpoint all covered; v0.18 — ADT^A40 merges source MRN into survivor (re-merge → 409, unknown source → 404, self-merge → 409, missing MRG → 400, wrong type at /merge → 400); v0.19 — DFT^P03 posts a charge (account auto-creation, account reuse on 2nd DFT, PY → 400 unsupported, missing FT1 → 400, bad currency → 400, wrong type → 400); v0.20 — multi-FT1 DFT^P03 posts 3 charges in one transaction (mixed currencies → 400; atomicity asserted: bad 2nd FT1 must not leave 1st persisted) |
| `tests/hl7v2_mllp_test.rs`        | y  | MLLP TCP listener on ephemeral port: real framed ADT → AA ACK framed; garbage frame → AR       |
| `tests/hl7v2_outbound_test.rs`    | y  | `Hl7v2MllpPublisher` (2 tests): AA happy path, AE peer leaves outbox row unpublished           |
| `tests/dashboard_test.rs`         | y  | `GET /dashboard` returns 200 + `text/html` with every Lily panel header                        |

A local Postgres is most easily provided via `docker compose up -d postgres` (see `docker-compose.yml`), then `cargo run --bin pas-migrate -- up` from the workspace root (or from this crate, since the `pas-migrate` binary lives in the sibling migration crate).

## Benchmarks

Three Criterion suites under `benches/`.

| File                          | What's measured                                                |
| ----------------------------- | -------------------------------------------------------------- |
| `benches/adt_bench.rs`        | `BedStatus::try_transition_to` full cycle                      |
| `benches/scheduling_bench.rs` | `TimeRange::overlaps` linear scans across 16…1024 ranges       |
| `benches/waitlist_bench.rs`   | `compute_active_weeks` across 4…256 events                     |

Run with:

```bash
cargo bench                                       # all suites
cargo bench -- adt                                # one suite
cargo bench -- compute_active_weeks               # one benchmark inside a suite
```

The `bench` profile inherits from `release` (LTO, single codegen unit, stripped).

## Quality Gates

Run from the workspace root so all three member crates (PAS, migrations, patient-administration-system-frontend) are checked together:

```bash
cargo test --workspace --lib                           # 497 PAS lib tests + 9 patient-administration-system-frontend csrf tests = 506 passing
cargo clippy --workspace --all-targets -- -D warnings  # zero warnings (one historical `too_many_arguments` was cleared)
cargo fmt --check --all                                # formatting clean
```

`cargo build --workspace --all-targets` must succeed with zero errors and zero warnings.

## Test Utilities

### Creating test patients

```rust
use patient_administration_system::models::patient::{HumanName, Patient};
use patient_administration_system::models::{Gender, NameUse};

let name = HumanName {
    use_type: Some(NameUse::Official),
    family:   "Doe".into(),
    given:    vec!["Jane".into()],
    prefix:   vec![],
    suffix:   vec![],
};
let patient = Patient::new(name, Gender::Female);
```

### Temporary search index

```rust
let temp_dir = tempfile::tempdir().unwrap();
let engine = patient_administration_system::search::SearchEngine::new(
    temp_dir.path().to_str().unwrap()
).unwrap();
```

### Money

```rust
use patient_administration_system::models::{Iso4217, Money};
use rust_decimal::Decimal;

let usd = Iso4217::new("USD").unwrap();
let m   = Money::new(Decimal::new(12345, 2), usd); // $123.45
```

### Time helpers

```rust
use chrono::{TimeZone, Utc};
let t = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
```

### Integration-test boilerplate

```rust
mod common;

#[tokio::test]
async fn test_something() {
    let Some(url) = common::database_url() else { return };
    let state = common::build_state(&url).await;
    // …
}
```

## Writing New Tests

### Unit-test guidelines

1. Place tests in a `#[cfg(test)] mod tests` block at the bottom of the source file.
2. Use descriptive names: `test_<function>_<scenario>` (e.g. `test_bed_status_occupied_to_available_fails`).
3. Test both success and failure paths. State machines need at least one test per legal and illegal transition.
4. For enums whose serde rename is meaningful (e.g. snake_case wire format), assert the literal string.
5. Use the model factory methods (`Patient::new`, `Identifier::mrn`, `RTTClockEvent::started`, ...) over building structs by hand.
6. For Tantivy: use `tempfile::tempdir()` for the index path so the OS cleans up.

### Integration-test guidelines

1. Place tests in the `tests/` directory.
2. Share setup via `tests/common/mod.rs` (`database_url`, `build_state`).
3. Always gate on `DATABASE_URL` — return early when unset so CI without Postgres stays green.
4. Test full HTTP request/response cycles end-to-end where possible (use `tower::ServiceExt::oneshot` against the router).
5. Verify both status codes and JSON shape.
6. Cover error paths (404, 409, 422).

## See Also

- [architecture.md](architecture.md) — layered design, transactional outbox
- [models.md](models.md) — model factory methods used in tests
- [matching.md](matching.md) — concurrency primitives exercised by `concurrency_test.rs`
- [restful.md](restful.md) — handler shapes asserted by integration tests
- [../README.md](../README.md#testing) — public testing summary
