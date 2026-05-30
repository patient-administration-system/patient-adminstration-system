# Patient Administration System (PAS)

A production-grade Rust crate implementing a hospital Patient Administration System — the foundational, non-clinical system of record for healthcare workflow.

[![Rust](https://img.shields.io/badge/rust-1.93%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

## Overview

The PAS owns the administrative *who, when, where, what visit type, what bed, what bill* — it does not own the *what diagnosis*. It feeds clinical portals and downstream systems (EMR, billing, analytics).

This crate is the v0.1 tier: a compiling, tested library + binary covering the headline flows end-to-end.

## Features

- Patient identity records (with optional MPI link via `mpi_id`). **Merge tombstones** (v0.11): when the MPI flags two rows as the same person, `POST /api/patients/{id}/merge-into/{target_id}` atomically marks one as `replaced_by` the other, with FHIR R5 `Patient.link[replaced-by]` emitted on the tombstone. **Multinational national-government identifiers** (v0.15): typed `Identifier::nhs / nir / tsi / ihi / hcn` factories with per-country format validators (NHS Mod 11; NIR Mod 97 with Corsica `2A`/`2B` fix-up; IHI 7 digits; HCN 10 digits; TSI alphanumeric envelope) so a single `Patient.identifiers` collection can carry UK, France, España, Ireland, and Northern Ireland identifiers side by side.
- ADT: admission, transfer, discharge with atomic bed allocation under `SELECT … FOR UPDATE`
- Scheduling: slot booking, cancellation, check-in, completion, no-show, bulk slot generation. **Recurring appointment series** (v0.9): RFC 5545 subset (daily/weekly/monthly + INTERVAL + BYDAY + COUNT/UNTIL), atomic-overlap reject, cap 200 occurrences.
- Waitlist + RTT clock (NHS-style, configurable per-pathway breach threshold) with breach detection
- Resources: ward/bed occupancy snapshots, bed status transitions
- Billing: accounts, charges, invoices, payments (multi-currency via ISO 4217). **Coverage** (v0.10): insurance / self-pay / other payer records per patient, optionally linked to a billing account; FHIR R5 `Coverage` read at `/fhir/Coverage/:id` and (v0.13) write via `POST /fhir` Bundle entries — same `CoverageRepository::create` path, same audit trail.
- Communication: Tera template letters with strict required-variable checking + delivery status. **SMS auto-send** (v0.8) via the `SmsProvider` trait — `LogSmsProvider` ships for dev/test; a real gateway is a one-trait-impl swap.
- Audit log (transactional, HIPAA-aligned) with per-entity and recent-activity query endpoints
- Domain events via transactional outbox pattern, polled by a background dispatcher
- Tantivy full-text patient search, kept in sync with create/update/delete
- RESTful API (Axum) with response envelope and bearer-token auth (opt-in)
- FHIR R5 read/write surface for `Patient`, `Encounter`, `Appointment`
- PostgreSQL persistence via SeaORM, with a `pas-migrate` CLI
- Privacy: masked patient view, GDPR export, consent CRUD + revoke

## Quick Start

### Prerequisites

- Rust 1.93+
- PostgreSQL 18+
- Docker (optional, recommended for local Postgres)

### Bootstrap a working server

```bash
git clone https://github.com/sixarm/patient-administration-system-rust-crate.git
cd patient-administration-system-rust-crate
cp .env.example .env

# 1. Start Postgres
docker-compose up -d postgres

# 2. Apply migrations + load demo data
(cd migrations && cargo run --bin pas-migrate -- up)
cargo run --bin pas-seed

# 3. Run the API server (in another terminal)
cargo run --release

# 4. Walk the headline flows
./demo.sh
```

Server defaults to `http://localhost:8080`. With `pas-seed` loaded, the database contains 1 facility, 1 ward, 1 room, 3 beds (`Available`), 2 practitioners, 1 patient (Jane Doe), and 1 letter template — everything `demo.sh` needs.

`pas-migrate` lives in the nested `migrations/` crate; subcommands are `up`, `down`, `fresh`, `status`.

### End-to-end demo

```bash
./demo.sh
```

Discovers the seeded ward, beds, patient, and template via REST, then walks admit → ward-occupancy → transfer → discharge → audit history → letter generation. Requires `jq`.

### Headline curl flow

```bash
# Health check
curl http://localhost:8080/api/health

# Find a patient
PID=$(curl -sS http://localhost:8080/api/patients?limit=1 | jq -r '.data[0].id')

# Find a bed in the seeded ward
WARD=$(curl -sS http://localhost:8080/api/wards | jq -r '.data[0].id')
BED=$(curl -sS http://localhost:8080/api/wards/$WARD/beds | jq -r '.data[0].id')

# Admit
curl -X POST http://localhost:8080/api/admissions \
  -H "Content-Type: application/json" \
  -H "X-User-Id: clinician-1" \
  -d "{\"patient_id\":\"$PID\",\"bed_id\":\"$BED\"}"
```

## Technology Stack

Mirrors `Cargo.toml` exactly. The v0.3.1 dep audit removed 15 unused direct deps (`loco-rs`, `fluvio`, `tonic`, `prost`, `tonic-build`, `openapiv3`, `jsonwebtoken`, `argon2`, `strsim`, `anyhow`, `bigdecimal`, `validator`, `hyper`, plus all five `opentelemetry*` crates); v0.3.4 removed `mockall` + `tokio-test` from dev-deps. Anything below is actually compiled and used.

| Component            | Technology                                  | Purpose                                  |
| -------------------- | ------------------------------------------- | ---------------------------------------- |
| **Language**         | Rust 1.93+, 2024 Edition                    | Systems programming, performance, safety |
| **Async Runtime**    | Tokio (`full`)                              | Asynchronous I/O and concurrency         |
| **Web Framework**    | Axum 0.7 (`macros`)                         | HTTP server and routing                  |
| **HTTP middleware**  | tower (`util`), tower-http (`cors`, `trace`, `compression-full`) | CORS / trace / compression / oneshot |
| **Templates**        | Tera 1.20                                   | Server-side rendering for letters + dashboard |
| **Database**         | PostgreSQL 18+                              | Persistence                              |
| **ORM**              | SeaORM 1.1 (`sqlx-postgres`, `runtime-tokio-rustls`, `with-chrono`, `with-uuid`, `with-json`) | Async ORM |
| **Migrations**       | sea-orm-migration 1.1 + `pas-migrate` CLI   | Schema management                        |
| **Search Engine**    | Tantivy 0.22                                | Full-text patient search                 |
| **Streaming**        | InMemoryEventPublisher (production + tests) + `Hl7v2MllpPublisher` (outbound ADT) + `WebhookEventPublisher` (v0.14, HMAC-signed HTTP POST) + `CompositePublisher` fan-out + `FluvioEventPublisher` stub | Event publishing |
| **API Docs**         | Utoipa 5.4 + `utoipa-swagger-ui` 8.1        | OpenAPI 3.1 annotations + Swagger UI     |
| **Serialization**    | Serde 1.0, serde\_json 1.0, quick-xml 0.36 (`serialize`), csv 1.3 | JSON / XML / CSV / TSV |
| **Logging / Tracing**| `tracing` 0.1, `tracing-subscriber` 0.3 (`env-filter`, `json`) | Structured logging (reads `RUST_LOG`) |
| **Observability**    | OpenTelemetry 0.27 + OTLP HTTP/protobuf via reqwest | Wires when `OTLP_ENDPOINT` is set; v0.7.0 |
| **Utilities**        | uuid 1.19 (`v4`, `serde`), chrono 0.4 (`serde`), dotenvy 0.15, async-trait 0.1 | UUIDs, timestamps, env, async traits |
| **Error Handling**   | thiserror 2.0                               | Typed errors                             |
| **Money / Decimal**  | rust\_decimal 1.36 (`serde-with-str`)       | Exact monetary arithmetic                |
| **Testing**          | `assertables` 9.8, `tempfile` 3.24          | Rich asserts + temp directories          |
| **Benchmarking**     | Criterion 0.5 (`html_reports`, `async_tokio`) | Statistical performance benchmarks     |
| **Containerization** | Docker (multi-stage), docker-compose        | Deployment packaging                     |

## Project Structure

```
patient-administration-system-rust-crate/
├── src/
│   ├── lib.rs                 Library root, public re-exports
│   ├── main.rs                Binary entry (Axum server)
│   ├── error.rs               Error enum, Result alias
│   ├── config/                Env/config loader
│   ├── models/                Pure-Rust domain types (no DB deps)
│   ├── db/                    SeaORM entities, repositories, audit + outbox
│   ├── adt/                   Admission/Transfer/Discharge service
│   ├── scheduling/            Slot booking, overlap detection
│   ├── waitlist/              Waitlist queue, RTT clock service
│   ├── resources/             Bed status, ward occupancy
│   ├── billing/               Charges, invoices, payments
│   ├── communication/         Tera letter rendering
│   ├── api/                   REST + FHIR R5 routers and handlers
│   ├── search/                Tantivy patient index
│   ├── streaming/             Event publisher (InMemory + Fluvio stub)
│   ├── validation/            Validators
│   ├── privacy/               Masking, GDPR export, consent helpers
│   └── observability/         tracing + OpenTelemetry
├── migrations/                SeaORM migration crate + `pas-migrate` CLI
├── tests/                     Integration tests (gated on DATABASE_URL)
├── benches/                   Criterion benchmarks (bed status, time overlap, RTT)
└── AGENTS/                    Architecture, models, API, scheduling, testing docs
```

## API Endpoint Reference

User context for audit logging is taken from headers: `X-User-Id`, `X-User-Ip`, `X-User-Agent`. When `API_TOKEN` is set, all `/api/*` endpoints (except `/api/health`) require `Authorization: Bearer <token>`.

| Group | Method | Path |
|-------|--------|------|
| Health | GET | `/api/health` |
| Patient CRUD | POST · GET | `/api/patients` (`?limit=N`) |
| | GET · PUT · DELETE | `/api/patients/:id` (v0.40: DELETE refuses 409 if patient has open admission; writes outbox `PatientDeleted` that fans out as ADT^A23) |
| | GET | `/api/patients/search?q=…&limit=N` |
| | GET | `/api/patients/:id/masked` |
| | GET | `/api/patients/:id/export` |
| | GET | `/api/patients/:id/audit` |
| | POST | `/api/patients/:id/merge-into/:target_id` (v0.11 — atomic flip to tombstone; drops source from search; audit + outbox `PatientMerged`) |
| | GET | `/api/patients/:id/replaces` (v0.11 — inverse lookup; lists every tombstone that points at this survivor) |
| | GET | `/api/audit/recent` |
| | GET | `/api/audit/entity?entity_type=…&entity_id=…` |
| Ops | GET | `/api/admin/outbox/unpublished` |
| | GET | `/api/admin/outbox/dead-letters` (v0.5) |
| | POST | `/api/admin/outbox/dead-letters/:id/replay` (v0.5) |
| Encounter | POST | `/api/encounters` |
| | GET | `/api/encounters/:id` |
| | GET | `/api/patients/:id/encounters` |
| | POST | `/api/encounters/:id/cancel` |
| | PUT | `/api/encounters/:id/status` |
| ADT | POST | `/api/admissions` |
| | POST | `/api/admissions/pre-admit` (v0.33; reserve bed + open Planned inpatient encounter; outbox fans out as ADT^A05) |
| | POST | `/api/admissions/change-to-inpatient` (v0.42; promote active ambulatory encounter to Inpatient + bed allocation; outbox fans out as ADT^A06) |
| | POST | `/api/admissions/cancel-pre-admit` (v0.35; release bed reservation + cancel Planned encounter; outbox fans out as ADT^A38) |
| | POST | `/api/admissions/:id/transfer` |
| | POST | `/api/admissions/:id/cancel-admit` (v0.38; cancel the open admission; outbox fans out as ADT^A11) |
| | POST | `/api/admissions/:id/cancel-transfer` (v0.31; reverses the most-recent transfer; outbox fans out as ADT^A12) |
| | POST | `/api/admissions/:id/leave-start` (v0.37; encounter InProgress → OnLeave; outbox fans out as ADT^A21) |
| | POST | `/api/admissions/:id/leave-end` (v0.37; encounter OnLeave → InProgress; outbox fans out as ADT^A22) |
| | POST | `/api/admissions/:id/discharge` |
| Scheduling | POST | `/api/appointment-series/preview` (v0.9 — dry-run a recurrence rule; returns datetimes without persisting) |
| | POST | `/api/appointment-series` (v0.9 — create series + every occurrence in one DB tx; atomic overlap reject) |
| | GET | `/api/appointment-series/:id` (v0.9 — series + linked occurrences) |
| | POST | `/api/appointment-series/:id/cancel` (v0.9 — cancel future occurrences) |
| | GET | `/api/patients/:id/appointment-series` (v0.9 — series per patient) |
| | POST | `/api/schedules` |
| | GET · DELETE | `/api/schedules/:id` |
| | DELETE | `/api/slots/:id` |
| | GET · POST | `/api/schedules/:id/slots` (`?start=…&end=…` for free slots) |
| | POST | `/api/schedules/:id/slots/bulk` (generate consecutive slots) |
| | POST | `/api/slots/:id/book` |
| | GET | `/api/appointments?patient_id=…` |
| | GET | `/api/appointments/:id` |
| | POST | `/api/appointments/:id/cancel` |
| | POST | `/api/appointments/:id/check-in` |
| | POST | `/api/appointments/:id/complete` |
| Waitlist + RTT | GET · POST | `/api/waitlist` (`?service=…`) |
| | PUT · DELETE | `/api/waitlist/:id` |
| | GET | `/api/waitlist/breaches` |
| | POST | `/api/rtt/start` |
| | POST | `/api/rtt/:id/pause` · `/api/rtt/:id/resume` · `/api/rtt/:id/stop` |
| | GET | `/api/rtt/:id/weeks-waiting` |
| | GET | `/api/patients/:id/rtt` |
| Resources | POST · GET | `/api/facilities` |
| | POST · GET | `/api/wards` (`?facility_id=…`) |
| | POST | `/api/rooms` |
| | POST | `/api/beds` (v0.27 — adds audit + outbox `BedCreated`) |
| | GET | `/api/beds/:id` · `/api/wards/:id/beds` · `/api/wards/:id/occupancy` |
| | PUT | `/api/beds/:id` (v0.27 — selective update; outbox `BedUpdated`) · `/api/beds/:id/status` (`OutOfService` additionally emits outbox `BedRetired`, v0.27) |
| Workforce | POST · GET | `/api/practitioners` |
| | GET · PUT · DELETE | `/api/practitioners/:id` |
| Billing | POST | `/api/accounts` |
| | POST | `/api/coverages` (v0.10) |
| | GET · PUT · DELETE | `/api/coverages/:id` (v0.10 — DELETE flips status to `cancelled`; never hard-deletes) |
| | GET | `/api/patients/:id/coverages` · `/api/accounts/:id/coverages` (v0.10) |
| | GET | `/api/patients/:id/account` |
| | POST | `/api/charges` · `/api/invoices` · `/api/payments` |
| Communication | GET · POST | `/api/letter-templates` |
| | PUT · DELETE | `/api/letter-templates/:id` |
| | POST | `/api/letters/generate` |
| | GET | `/api/letters/:id` |
| | POST | `/api/letters/:id/sent` · `/api/letters/:id/failed` |
| Consent | POST · GET | `/api/patients/:id/consents` |
| | POST | `/api/consents/:id/revoke` |
| Interchange (v0.2) | GET | `/api/patients/export.json` · `/api/patients/export.xml` · `/api/patients/export.tsv` · `/api/patients/export.csv` |
| | POST | `/api/patients/import` (Content-Type picks JSON / XML / TSV / CSV) |
| HL7 v2 (v0.3) | POST | `/api/hl7/v2/parse` (echoes parsed message as JSON) |
| | POST | `/api/hl7/v2/batch` (v0.6 — accepts a batch envelope of up to 1000 ADT messages; returns per-message ACKs wrapped in one BHS/BTS envelope) |
| | POST | `/api/hl7/v2/patient` (ingest ADT^A28/A01; returns ACK^... text body) |
| | POST | `/api/hl7/v2/admit` (ingest ADT^A01; creates patient + admits to bed from PV1-3.3) |
| | POST | `/api/hl7/v2/pre-admit` (ingest ADT^A05; reserves bed from PV1-3.3 and opens a Planned inpatient encounter; no admission row; v0.32) |
| | POST | `/api/hl7/v2/change-to-inpatient` (ingest ADT^A06; promotes most-recent active ambulatory encounter to Inpatient + bed allocation; v0.41) |
| | POST | `/api/hl7/v2/change-to-outpatient` (ingest ADT^A07; demotes open inpatient admission to Outpatient; releases bed; encounter stays InProgress; v0.43) |
| | POST | `/api/hl7/v2/cancel-pre-admit` (ingest ADT^A38; releases the bed reservation and cancels the Planned inpatient encounter; v0.34) |
| | POST | `/api/hl7/v2/leave-start` (ingest ADT^A21; encounter InProgress → OnLeave; bed remains Occupied; v0.36) |
| | POST | `/api/hl7/v2/leave-end` (ingest ADT^A22; encounter OnLeave → InProgress; v0.36) |
| | POST | `/api/hl7/v2/delete-patient` (ingest ADT^A23; soft-deletes patient + drops from search; refuses if patient has any open admission; v0.39) |
| | POST | `/api/hl7/v2/register` (ingest ADT^A04; dedup-or-create patient, open Outpatient or Emergency encounter in Arrived status — no bed; v0.28) |
| | POST | `/api/hl7/v2/transfer` (ingest ADT^A02; finds open admission by MRN, transfers to bed from PV1-3.3) |
| | POST | `/api/hl7/v2/discharge` (ingest ADT^A03; finds open admission by MRN and discharges) |
| HL7 v2 ADT-lifecycle (v0.4) | POST | `/api/hl7/v2/update` (ingest ADT^A08; merges PID demographics over the existing patient row) |
| | POST | `/api/hl7/v2/cancel-admit` (ingest ADT^A11; releases bed, flips bed to Cleaning, cancels encounter) |
| | POST | `/api/hl7/v2/cancel-transfer` (ingest ADT^A12; reverses the most-recent transfer; restores patient to origin bed; v0.30) |
| | POST | `/api/hl7/v2/cancel-discharge` (ingest ADT^A13; reinstates the most-recent discharge to the original bed) |
| | POST | `/api/hl7/v2/merge` (ingest ADT^A40 — merge source patient identified by MRG-1 MRN into survivor identified by PID; v0.18) |
| | POST | `/api/hl7/v2/dft` (ingest DFT^P03 — post one or many charges from FT1 segments; v0.20 makes all-or-nothing across multi-FT1 messages; auto-creates an open billing account when none exists; v0.19) |
| | POST | `/api/hl7/v2/mfn-staff` (ingest MFN^M02 — master file notification for practitioner roster; MAD/MUP/MDL via MFE+STF pairs, atomic per message; v0.24) |
| | POST | `/api/hl7/v2/mfn-location` (ingest MFN^M05 — master file notification for bed roster; MAD/MUP/MDL via MFE+LOC pairs, LOC-1 PL-typed `<room>^^<bed>`, atomic per message; v0.26) |
| HL7 v2 SIU (v0.16 + v0.17) | POST | `/api/hl7/v2/schedule-book` (ingest SIU^S12 — book appointment from SCH + PID; AA ACK reports the assigned filler id) |
| | POST | `/api/hl7/v2/schedule-reschedule` (ingest SIU^S13 — change start/end of appointment identified by SCH-2 filler uuid; overlap-excluding check) |
| | POST | `/api/hl7/v2/schedule-modify` (ingest SIU^S14 — update reason of appointment identified by SCH-2 filler uuid) |
| | POST | `/api/hl7/v2/schedule-cancel` (ingest SIU^S15 — cancel appointment identified by SCH-2 filler uuid) |
| FHIR R5 | POST | `/fhir` (batch / transaction Bundle of writes — Patient · Encounter · Appointment · Coverage as of v0.13; · Practitioner · Schedule · Slot as of v0.23) |
| | POST | `/fhir/Patient` · `/fhir/Encounter` · `/fhir/Appointment` · `/fhir/Practitioner` · `/fhir/Schedule` · `/fhir/Slot` (last three v0.21) |
| | GET · PUT · DELETE | `/fhir/Patient/:id` · `/fhir/Practitioner/:id` · `/fhir/Schedule/:id` · `/fhir/Slot/:id` (last three v0.21) |
| | GET | `/fhir/Encounter/:id` · `/fhir/Appointment/:id` · `/fhir/Location/:id` |
| | GET | `/fhir/Patient?_count=N` (collection `Bundle`) |
| | GET | `/fhir/Coverage/:id` (v0.10 — FHIR R5 Coverage read; **write via `POST /fhir` Bundle entries as of v0.13**) |

## API Examples

### Health

```bash
curl http://localhost:8080/api/health
```

### ADT — Admit, Transfer, Discharge

```bash
# Admit a patient to a bed
curl -X POST http://localhost:8080/api/admissions \
  -H "Content-Type: application/json" \
  -d '{
    "patient_id": "11111111-1111-1111-1111-111111111111",
    "bed_id":     "22222222-2222-2222-2222-222222222222"
  }'

# Transfer to a new bed
curl -X POST http://localhost:8080/api/admissions/<admission-id>/transfer \
  -H "Content-Type: application/json" \
  -d '{ "new_bed_id": "33333333-3333-3333-3333-333333333333" }'

# Discharge
curl -X POST http://localhost:8080/api/admissions/<admission-id>/discharge \
  -H "Content-Type: application/json" \
  -d '{ "disposition": "home" }'
```

### Scheduling

```bash
# Book a slot
curl -X POST http://localhost:8080/api/slots/<slot-id>/book \
  -H "Content-Type: application/json" \
  -d '{ "patient_id": "<uuid>", "reason": "follow-up" }'

# Cancel an appointment
curl -X POST http://localhost:8080/api/appointments/<id>/cancel \
  -H "Content-Type: application/json" \
  -d '{ "reason": "patient_request" }'
```

### Waitlist + RTT

```bash
# Add to waitlist
curl -X POST http://localhost:8080/api/waitlist \
  -H "Content-Type: application/json" \
  -d '{
    "patient_id":     "<uuid>",
    "target_service": "cardiology",
    "priority":       "routine"
  }'

# Start an RTT clock
curl -X POST http://localhost:8080/api/rtt/start \
  -H "Content-Type: application/json" \
  -d '{ "patient_id": "<uuid>", "target_service": "cardiology" }'

# Pause an RTT pathway
curl -X POST http://localhost:8080/api/rtt/<pathway-id>/pause \
  -H "Content-Type: application/json" \
  -d '{ "reason": "patient_holiday" }'
```

### Resources

```bash
# Ward occupancy snapshot
curl http://localhost:8080/api/wards/<ward-id>/occupancy
```

### Billing

```bash
# Open an account
curl -X POST http://localhost:8080/api/accounts \
  -H "Content-Type: application/json" \
  -d '{ "patient_id": "<uuid>", "currency": "USD" }'

# Post a charge
curl -X POST http://localhost:8080/api/charges \
  -H "Content-Type: application/json" \
  -d '{
    "account_id":  "<uuid>",
    "code":        "99213",
    "description": "Office visit",
    "amount":      { "amount": "120.00", "currency": "USD" }
  }'

# Finalize an invoice
curl -X POST http://localhost:8080/api/invoices \
  -H "Content-Type: application/json" \
  -d '{ "account_id": "<uuid>", "charge_ids": ["<uuid>", "<uuid>"] }'

# Post a payment
curl -X POST http://localhost:8080/api/payments \
  -H "Content-Type: application/json" \
  -d '{
    "invoice_id": "<uuid>",
    "amount":     { "amount": "120.00", "currency": "USD" },
    "method":     "card",
    "reference":  "TXN-9001"
  }'
```

### Communication

```bash
# Generate a letter from a template
curl -X POST http://localhost:8080/api/letters/generate \
  -H "Content-Type: application/json" \
  -d '{
    "template_id":    "<uuid>",
    "patient_id":     "<uuid>",
    "appointment_id": "<uuid>",
    "channel":        "print"
  }'
```

### Interchange — JSON, XML, TSV

v0.2 ships bulk-interchange formats for patient data on top of the standard JSON-over-HTTP API. All three formats share the flat `PatientRow` projection — see [`AGENTS/interchange.md`](AGENTS/interchange.md) for the schema and lossy-field notes, and [`examples/`](examples/) for working payloads.

```bash
# Export every active patient (capped at 10k rows)
curl -s http://localhost:8080/api/patients/export.json
curl -s http://localhost:8080/api/patients/export.xml
curl -s http://localhost:8080/api/patients/export.tsv
curl -s http://localhost:8080/api/patients/export.csv

# Bulk import (idempotent — existing ids are skipped). Format is picked from
# Content-Type: application/json, application/xml, or text/tab-separated-values.
curl -X POST http://localhost:8080/api/patients/import \
  -H 'Content-Type: application/json' \
  --data-binary @examples/patients.json

curl -X POST http://localhost:8080/api/patients/import \
  -H 'Content-Type: application/xml' \
  --data-binary @examples/patients.xml

curl -X POST http://localhost:8080/api/patients/import \
  -H 'Content-Type: text/tab-separated-values' \
  --data-binary @examples/patients.tsv

curl -X POST http://localhost:8080/api/patients/import \
  -H 'Content-Type: text/csv' \
  --data-binary @examples/patients.csv

# Response shape:
# { "success": true, "data": { "inserted": 2, "skipped": 1, "failed": 0 } }
```

### FHIR R5

v0.2 widens the FHIR R5 read surface to cover every administrative resource the PAS persists.

```bash
# Create / read / update / soft-delete a Patient
curl -X POST http://localhost:8080/fhir/Patient \
  -H 'Content-Type: application/json' \
  --data-binary @examples/fhir-patient.json
curl http://localhost:8080/fhir/Patient/{id}

# Collection Bundle of recent patients
curl 'http://localhost:8080/fhir/Patient?_count=20'

# Read other administrative resources
curl http://localhost:8080/fhir/Practitioner/{id}
curl http://localhost:8080/fhir/Schedule/{id}
curl http://localhost:8080/fhir/Slot/{id}
curl http://localhost:8080/fhir/Location/{id}   # PAS bed
```

Run `cargo run --example interchange` to see the three flat formats round-trip without data loss.

```bash
# Batch-create multiple resources in one round-trip via a FHIR Bundle.
# The server returns a transaction-response Bundle with per-entry status + location.
curl -X POST http://localhost:8080/fhir \
  -H 'Content-Type: application/json' \
  --data-binary @examples/fhir-transaction-bundle.json
```

Bundle semantics:

- **`type: batch`** — entries are processed independently against the DB pool. Per-entry success/failure surfaces in the entry's `response.status` (e.g. `"201 Created"` + `location`, or `"400 Bad Request: ..."`). HTTP status is always 200.
- **`type: transaction`** — all-or-nothing. Every entry runs inside a single `sea_orm` transaction. If any entry fails (parse, validation, DB error), the transaction is rolled back and the endpoint returns `400 + OperationOutcome` naming the offending entry index — no partial writes survive. Search-index updates (Tantivy) are deferred until after the commit so rolled-back rows never become searchable.

### HL7 v2 ADT

For interop with legacy clinical systems, v0.3 ships a pipe-delimited HL7 v2 ingest endpoint. The first cut supports `MSH`, `EVN`, `PID`, and `PV1` segments with the standard delimiter set `|^~\&`. See [`AGENTS/interchange.md`](AGENTS/interchange.md) for the segment-to-domain mapping and the [`examples/hl7-adt-a28.txt`](examples/hl7-adt-a28.txt) sample payload.

```bash
# Inspect a message's structure as JSON
curl -X POST http://localhost:8080/api/hl7/v2/parse \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a28.txt

# Ingest an ADT^A28 (add person info); the response body is a v2 ACK envelope
curl -X POST http://localhost:8080/api/hl7/v2/patient \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a28.txt
# →  MSH|^~\&|PAS|FAC|EMR|FAC|...|ACK|MSG-EMR-001|P|2.5\r
#    MSA|AA|MSG-EMR-001\r

# Ingest an ADT^A01 (admit). The PV1-3.3 bed code must match an existing bed.
# On success: creates the patient AND admits them through AdtService::admit,
# so the bed flips to "occupied" and an EncounterAdmitted event is published.
curl -X POST http://localhost:8080/api/hl7/v2/admit \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a01.txt

# ADT^A02 (transfer). Identifies the open admission by MRN (PID-3.1) and
# transfers to the destination bed in PV1-3.3.
curl -X POST http://localhost:8080/api/hl7/v2/transfer \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a02.txt

# ADT^A03 (discharge). Identifies the open admission by MRN and discharges.
curl -X POST http://localhost:8080/api/hl7/v2/discharge \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a03.txt
```

ACK codes: `AA` = accepted, `AE` = application error (well-formed but rejected — e.g. PID-5 missing), `AR` = rejected before processing (parse failure).

#### MLLP TCP listener

Real hospital networks ship HL7 v2 over MLLP, not HTTP. Set `HL7V2_MLLP_BIND=0.0.0.0:2575` (the conventional port) to enable a TCP listener that consumes MLLP-framed (`\x0b<message>\x1c\x0d`) ADT messages and routes them through the same handlers as the HTTP endpoints. The ACK is returned in the same MLLP framing.

```bash
HL7V2_MLLP_BIND=0.0.0.0:2575 cargo run --bin patient-administration-system

# Send a frame using Python:
python3 - <<'PY'
import socket
msg = b"MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A28|MSG1|P|2.5\r" \
      b"EVN|A28|20260523120000\r" \
      b"PID|1||MRN-MLLP-001^^^FAC^MR||Doe^Jane||19900115|F\r"
s = socket.create_connection(("localhost", 2575))
s.sendall(b"\x0b" + msg + b"\x1c\x0d")
print(s.recv(4096))
PY
```

## Configuration

Configuration is loaded from environment variables (or `.env`).

| Variable             | Description                                                       | Default          | Required |
| -------------------- | ----------------------------------------------------------------- | ---------------- | -------- |
| `DATABASE_URL`       | PostgreSQL connection string                                      | -                | Yes      |
| `SERVER_HOST`        | HTTP server bind address                                          | `0.0.0.0`        | No       |
| `SERVER_PORT`        | HTTP server port                                                  | `8080`           | No       |
| `SEARCH_INDEX_PATH`  | Tantivy index directory                                           | `./search_index` | No       |
| `RUST_LOG`           | Logging level (e.g., `info`, `debug`)                             | `info`           | No       |
| `OTLP_ENDPOINT`      | OpenTelemetry OTLP collector endpoint (HTTP/protobuf, e.g. `http://otel-collector:4318/v1/traces`). When set, every `tracing` span is exported to the collector. When unset (default), fmt layer only — no network egress. | - | No |
| `OTEL_SERVICE_NAME`  | `service.name` resource attribute on every exported span. Override to disambiguate replicas / environments in the collector. | `pas-axum` | No |
| `PAS_SMS_PROVIDER`   | SMS provider selector (v0.8). `none` = no auto-send, SMS letters stay `pending`. `log` = log each outbound message to `tracing` and flip the letter to `sent`. Real production gateways are added by the consumer via the `SmsProvider` trait. Unknown values fall back to `none`. | `none` | No |
| `PAS_RATE_LIMIT_RPM` | Per-IP rate-limit (v0.12) sustained refill rate. Set `0` to disable the middleware entirely. | `600` (= 10 req/sec) | No |
| `PAS_RATE_LIMIT_BURST` | Per-IP rate-limit bucket capacity. A client may fire `burst` requests back-to-back before being gated to the sustained rate. Ignored when `PAS_RATE_LIMIT_RPM=0`. | `60` | No |
| `API_TOKEN`          | If set, bearer-auth is enforced on every `/api/*` except `/api/health` | -          | No       |
| `CORS_ORIGINS`       | Comma-separated allowlist; empty/unset = permissive (with warn)   | -                | No       |
| `HL7V2_MLLP_BIND`    | `host:port` for the MLLP TCP listener (HL7 v2 ADT). Unset = off   | -                | No       |
| `HL7V2_OUTBOUND_PEER`| `host:port` of a downstream MLLP receiver for outbound ADT messages | -            | No       |
| `PAS_OUTBOX_MAX_RETRIES`| Per-event retry budget for the outbox dispatcher. After this many consecutive failed publishes, the row is moved to `outbox_dead_letters`. Set to `0` to disable dead-lettering. | `10` | No |
| `PAS_WEBHOOK_URL`    | v0.14 — outbox webhook destination URL. When set, `WebhookEventPublisher` POSTs every `DomainEvent` as JSON. Combines with `HL7V2_OUTBOUND_PEER` via `CompositePublisher` fan-out when both are set. | unset (disabled) | No |
| `PAS_WEBHOOK_SECRET` | v0.14 — HMAC-SHA256 secret for the webhook. When set, every POST carries `X-PAS-Signature: sha256=<hex>` over the raw body. | unset | No |
| `PAS_WEBHOOK_TIMEOUT_SECS` | v0.14 — webhook request timeout in seconds. Keep strictly shorter than the dispatcher 2 s poll for responsive retries. | `10` | No |

When the server is running, the **complete OpenAPI spec** (every REST + FHIR + HL7 v2 endpoint — patients, ADT, scheduling, billing, communication, audit, consent, interchange, FHIR Bundles, HL7 v2) is served at `/api-docs/openapi.json` with Swagger UI at `/swagger-ui`. An at-a-glance ops view (ward occupancy, RTT breaches, outbox unpublished count, recent audit) is served at `/dashboard` — Tera-rendered server-side on first load, then HTMX polls four fragment endpoints every 10s to keep each panel live. Works without JavaScript (initial render embeds every fragment inline); HTMX is a progressive enhancement. Markup uses [Lily Design System](https://github.com/LilyDesignSystem/lily) headless HTML class names (`header`, `footer`, `panel`, `data-table*`, `badge`, `alert`, `code`).

A parallel **Loco-rs front-end** lives at [`../patient-administration-system-frontend`](../patient-administration-system-frontend/) — separate crate, shares this app's PostgreSQL database, ports the same dashboard + adds patient list/detail pages. Builds clean against Loco 0.14.1 (Hooks impl, ViewEngine<TeraView> extractors, sea-orm read-side projections). Boot with `cargo run -- start` from the `patient-administration-system-frontend/` directory; browse to `http://localhost:5150/dashboard`.

## Development

```bash
# Build
cargo build

# Test (library tests only)
cargo test --lib

# Lint
cargo clippy

# Format
cargo fmt

# Format check (CI)
cargo fmt --check
```

## Migrations

Migrations live as a nested SeaORM crate at `migrations/` with a thin CLI binary `pas-migrate`:

```bash
# All subcommands honour DATABASE_URL from .env or the environment.
(cd migrations && cargo run --bin pas-migrate -- up)        # apply pending
(cd migrations && cargo run --bin pas-migrate -- status)    # show state
(cd migrations && cargo run --bin pas-migrate -- fresh)     # drop + re-apply
(cd migrations && cargo run --bin pas-migrate -- down)      # revert all
```

The single migration `m20260520_000001_init.rs` creates 29 tables. To keep the v0.1 surface tractable, repeated child collections on `patients` and `practitioners` (identifiers, names, addresses, contacts, emergency contacts) are stored inline as `JSONB` columns instead of as separate tables.

## Testing

**Unit tests:** 497 passing (`cargo test --lib`). Coverage spans:

- Domain models — state-machine transitions, serde round-trip, factory methods.
- DB repositories — CRUD and aggregate loading via SeaORM mock connections.
- Services — ADT, scheduling, waitlist, RTT, billing, communication, resources.
- Interop — HL7 v2 parser / encoder / mapping / escapes / MLLP framing.
- Interchange — JSON / XML / TSV / CSV round-trip for `PatientRow`.
- FHIR R5 — resource converters, OperationOutcome envelope, OpenAPI spec.
- Dashboard — Tera template render + Lily class-name assertions.
- Cross-cutting — validation, privacy, search, streaming, config, error mapping.

**Integration tests:** 62 test functions across 24 files in `tests/`. Most gated on `DATABASE_URL`; `auth_test.rs`, `rate_limit_test.rs`, and (v0.14) `webhook_test.rs` exercise pure middleware / publisher logic without a database and run every `cargo test`. Live Postgres (e.g. `docker compose up -d postgres` or `docker-compose.test.yml`) is required for the rest.

| File                              | DB | What's tested                                                                |
| --------------------------------- | -- | ---------------------------------------------------------------------------- |
| `tests/health_test.rs`            | y  | `/api/health` returns ok with degraded-on-DB-down signal                     |
| `tests/auth_test.rs`              | n  | Bearer-token middleware (4 tests): exempt health, missing, wrong, ok         |
| `tests/patient_crud_test.rs`      | y  | Create → get → search → update → list → masked → soft-delete → audit         |
| `tests/practitioner_test.rs`      | y  | Create → get → list → update → soft-delete (active flip) → exclusion         |
| `tests/adt_flow_test.rs`          | y  | Admit → transfer → discharge with bed-status assertions                      |
| `tests/scheduling_flow_test.rs`   | y  | Book → check-in → complete with slot status assertion                        |
| `tests/waitlist_rtt_flow_test.rs` | y  | Add → start → pause → resume → stop → weeks_waiting                          |
| `tests/billing_flow_test.rs`      | y  | Open account → conflict on re-open → charges → finalize → partial payments   |
| `tests/letter_flow_test.rs`       | y  | Template create → render → fetch → missing-variable rejection → list. v0.8: SMS auto-send with `LogSmsProvider` flips status to `sent` + stamps `sent_at`; patient with no phone leaves the letter `pending` (2 tests) |
| `tests/appointment_series_test.rs` | y | v0.9: preview → create (weekly count=4) → fetch → cancel → assert all 4 occurrences flipped + series row `cancelled`. Second test: atomic-overlap reject — pre-seed a blocker at week 2, assert create returns 409 and no series row survives (2 tests) |
| `tests/coverage_flow_test.rs`     | y  | v0.10: create coverage → list per patient → link to account → list per account → update (incl. clear group_number via `null`) → FHIR GET → soft-cancel via DELETE → FHIR GET still 200 with `status=cancelled` → 404 on unknown id |
| `tests/patient_merge_test.rs`     | y  | v0.11: create A + B → merge A→B → tombstone fields verified → `GET /api/patients/B/replaces` lists A → FHIR Patient/A carries `link[replaced-by]` → FHIR Patient/B has none → Tantivy no longer surfaces A → reject self-merge (400), double-merge (409), unknown target (404) |
| `tests/rate_limit_test.rs`        | n  | v0.12: tight `burst=2` config → first two requests pass, third 429 with `Retry-After` + `error.code = "RATE_LIMITED"`; `/api/health` exempt — 5 consecutive hits after the bucket is drained all 200 (2 tests, no DB needed) |
| `tests/consent_flow_test.rs`      | y  | Two consents → list active → revoke one → assert mixed status                |
| `tests/fhir_write_test.rs`        | y  | Patient + Practitioner + Schedule + Slot CRUD (5 tests): POST → GET → PUT (id preserved) → DELETE; Practitioner DELETE flips active=false; Schedule + Slot DELETE hard-delete; 404 on PUT/DELETE of unknown id (v0.21) |
| `tests/fhir_bundle_test.rs`       | y  | `POST /fhir` batch + transaction (5 tests): per-entry response shape; transaction rollback assertion; v0.13 — Coverage entries (primary insurance + self-pay) in batch + Coverage parse failure rolls a transaction back; v0.23 — chained Practitioner → Schedule → Slot bundle creation + transaction atomicity (malformed Slot rolls back the prior Schedule) |
| `tests/concurrency_test.rs`       | y  | Two-way races for the same bed and same slot; exactly one 200, the other 409 |
| `tests/outbox_dispatcher_test.rs` | y  | Happy path: outbox row written → one tick → marked `published=true`. v0.5: failing publisher → retry_count rises to budget → row moves to `outbox_dead_letters` → replay puts a fresh row back (retry_count=0) → next tick drains it (2 tests) |
| `tests/interchange_test.rs`       | y  | Bulk import in all 4 formats (JSON/XML/TSV/CSV) → idempotent skip → exports round-trip → Tantivy visibility |
| `tests/hl7v2_test.rs`             | y  | HL7 v2 ADT^A28/A01/A02/A03 + A08/A11/A13/A40 + batch + SIU^S12/S13/S14/S15 + DFT^P03 + MFN^M02 + MFN^M05 (15 tests): AA + AE + AR codes, exact-MRN dedup, update merges, cancel-admit drops ward occupancy to 0, cancel-discharge reinstates bed, 3-message batch with mixed AA/AE inside one BTS envelope; v0.16 — SIU^S12 books an appointment (overlap-protection 409 + filler-uuid round-trip into a subsequent SIU^S15 cancel; double-cancel → 409; unknown filler → 404); v0.17 — SIU^S13 reschedules with overlap-excluding check (own row doesn't flag itself; conflict with sibling → 409), SIU^S14 modifies reason; both 400 / 404 / 409 paths covered; v0.18 — ADT^A40 merges source MRN into survivor PID (re-merge → 409, unknown source → 404, self-merge → 409, missing MRG → 400, wrong message type at /merge → 400); v0.19 — DFT^P03 posts a charge (account auto-creation, account reuse on 2nd DFT, PY unsupported → 400, missing FT1 → 400, bad currency → 400, wrong message type → 400); v0.20 — multi-FT1 DFT^P03 posts 3 charges in one transaction; mixed currencies → 400; **atomicity** (a bad 2nd FT1 must not leave the 1st persisted); v0.24 — MFN^M02 walks MAD/MUP/MDL on a practitioner; duplicate-MAD → 409; unknown-MUP → 404; 2-item MFN with duplicate second item rolls back the first; v0.26 — MFN^M05 walks MAD/MUP/MDL on a bed (bootstrap facility+ward+room via REST first), duplicate-MAD → 409, unknown-bed MUP → 404, orphan-room MAD → 404, MDL flips to `OutOfService`, atomicity holds across multi-item messages; v0.28 — ADT^A04 registers an outpatient (PV1-2 = `O`) and an emergency-class encounter (PV1-2 = `E`) in `Arrived` status with no bed; repeat-MRN dedups patient but opens a fresh encounter; missing PV1 → 400; wrong message type at /register → 400; v0.30 — ADT^A12 cancels the most-recent transfer (admit to A → transfer to B → cancel-transfer → patient back on A, B Cleaning); re-cancel → 404; wrong message type → 400; v0.32 — ADT^A05 pre-admits a patient (bed flips to Reserved, encounter Planned); second A05 on the same bed → 409; unknown bed → 404; wrong message type → 400; v0.34 — ADT^A38 cancels the pre-admit (bed → Available, encounter → Cancelled); re-cancel → 409; unknown patient → 404; wrong message type → 400; v0.36 — ADT^A21/A22 LOA round-trip (encounter InProgress → OnLeave → InProgress; bed stays Occupied throughout); second A21 / A22 → 409; wrong message type → 400; v0.39 — ADT^A23 soft-deletes a patient (no admission) + Tantivy drop + outbox PatientDeleted; admitted-patient A23 → 409; unknown MRN → 404; wrong message type → 400; v0.41 — ADT^A06 promotes an active Outpatient encounter to Inpatient (bed Occupied, encounter Inpatient+InProgress); second A06 → 404 (no remaining ambulatory); unknown patient → 404; wrong message type → 400; v0.43 — ADT^A07 demotes an open Inpatient admission to Outpatient (bed Cleaning, encounter Outpatient+InProgress); second A07 → 400 (no remaining open admission); wrong message type → 400 |
| `tests/hl7v2_mllp_test.rs`        | y  | MLLP TCP listener (ephemeral port): real framed ADT → AA ACK, garbage → AR   |
| `tests/hl7v2_outbound_test.rs`    | y  | Outbound `Hl7v2MllpPublisher` (2 tests): AA happy path, AE → unpublished retry. v0.25 — `PractitionerCreated`/`Updated`/`Deactivated` map to `MFN^M02` MAD/MUP/MDL; v0.27 — `BedCreated`/`BedUpdated`/`BedRetired` map to `MFN^M05` MAD/MUP/MDL; v0.29 — `EncounterRegistered` maps to `ADT^A04` with PV1-2 = `O`/`E`; v0.31 — `EncounterTransferCancelled` maps to `ADT^A12` with PV1-3 = origin bed code; v0.33 — `EncounterPreAdmitted` maps to `ADT^A05` with PV1-3 = reserved bed code; v0.35 — `EncounterPreAdmitCancelled` maps to `ADT^A38` with PV1-3 = released bed code; v0.37 — `EncounterLeaveStarted` / `EncounterLeaveEnded` map to `ADT^A21` / `ADT^A22` (PID-only, no PV1); v0.40 — `PatientDeleted` maps to `ADT^A23` (PID-only); v0.42 — `EncounterPromotedToInpatient` maps to `ADT^A06` (PV1-3 = bed code); all source-gated on the inbound `hl7v2_*` tags to prevent boomerangs |
| `tests/webhook_test.rs`           | n  | v0.14: end-to-end POST against an in-process `TcpListener` receiver — verifies body shape, `X-PAS-Event-Id` + `X-PAS-Event-Type` headers, and `X-PAS-Signature` HMAC matches the raw body; `CompositePublisher` fans out to webhook + in-memory subscriber (2 tests, no DB needed) |
| `tests/dashboard_test.rs`         | y  | `GET /dashboard` returns 200 + `text/html` with every Lily panel header      |

**Benchmarks:** three Criterion suites under `benches/`.

| File                          | What's measured                                                |
| ----------------------------- | -------------------------------------------------------------- |
| `benches/adt_bench.rs`        | `BedStatus::try_transition_to` full cycle                      |
| `benches/scheduling_bench.rs` | `TimeRange::overlaps` across linear scans (16…1024 ranges)     |
| `benches/waitlist_bench.rs`   | `compute_active_weeks` at varying event counts (4…256)         |

Run with `cargo bench` or e.g. `cargo bench -- adt`.

**Quality gates:**

```bash
cargo test --lib                                          # 365 tests
cargo test                                                # add the 53 integration tests (most need DATABASE_URL; 8 don't)
cargo clippy --workspace --all-targets -- -D warnings     # zero warnings
cargo fmt --check --all                                   # formatting clean
```

## Status

**Version:** v0.43.0 — library + `patient-administration-system` binary + `pas-seed` binary + `pas-migrate` binary. See [`CHANGELOG.md`](CHANGELOG.md) for the per-release summary.

**Implemented:**

- Domain models for all entities (identity, workforce, resources, ADT, scheduling, waitlist + RTT, communication, billing, consent).
- SeaORM entities, repositories, transactional audit + outbox.
- Service layer (ADT, scheduling, waitlist, RTT, resources, billing, communication).
- REST API (~85 routes) + FHIR R5 read/write for Patient, Encounter, Appointment, plus reads for Practitioner / Schedule / Slot / Location.
- FHIR Bundle writes (`POST /fhir`) with genuine all-or-nothing transaction semantics.
- Bulk patient interchange in JSON / XML / TSV / CSV (`/api/patients/export.*` + `/api/patients/import`).
- HL7 v2 ADT — A01 (admit), A02 (transfer), A03 (discharge), A28 (add person), **A08 (update patient)**, **A11 (cancel admit)**, **A13 (cancel discharge)** — over both HTTP and MLLP TCP. **Batch envelope** (`FHS`/`BHS`/`BTS`/`FTS`, v0.6) carries up to 1000 messages per transmission. Outbound MLLP publisher when `HL7V2_OUTBOUND_PEER` is set (source-gated so REST-driven edits don't echo). HL7 v2 escape sequences honored at the boundary. **Exact-MRN dedup** on inbound A01 / A28 — re-sending the same PID reuses the existing patient row and reports `matched existing patient <uuid>` in MSA-3.
- **Complete OpenAPI 3.1 spec** for every REST + FHIR + HL7 v2 endpoint at `/api-docs/openapi.json` + Swagger UI at `/swagger-ui`. 105 handlers tagged across 19 categories.
- Bearer-token auth (opt-in via `API_TOKEN`), configurable CORS, compression, request tracing. **Per-IP rate-limit** (v0.12, default 600 req/min with 60-burst, `/api/health` exempt; tunable via `PAS_RATE_LIMIT_RPM` / `PAS_RATE_LIMIT_BURST`).
- Background outbox dispatcher polling every 2s, forwarding to the configured `EventPublisher`. Bounded retry budget (`PAS_OUTBOX_MAX_RETRIES`, default 10); rows that exceed it move to `outbox_dead_letters` and can be replayed via `POST /api/admin/outbox/dead-letters/{id}/replay`. Built-in publishers: HL7 v2 MLLP outbound (v0.4), HTTP webhook with HMAC-SHA256 signing (v0.14), in-memory; combine via `CompositePublisher` when more than one is configured.
- Ops dashboard at `GET /dashboard` (Tera + HTMX live-refresh + Lily Design System markup); a parallel Loco-rs front-end sibling crate `patient-administration-system-frontend` (port `:5150`, v0.1.0) also serves it plus patient pages and three write flows that proxy back through this API.
- **497 unit tests** + **24 integration test binaries** (62 test functions); clean `cargo fmt --check --all`, clean `cargo clippy --workspace --all-targets -- -D warnings`.
- Lean dependency tree (v0.3.1 audit removed 15 unused deps; v0.3.4 removed two unused dev-deps).

**Deferred to later phases:**

- Real JWT / OAuth (bearer-token middleware exists; full identity-provider integration does not).
- gRPC server (no longer scaffolded — was removed in the v0.3.1 dep audit; reintroduce `tonic` + `prost` + a `build.rs` when actually needed).
- Fluvio event producer (in-memory + HL7 v2 publishers work; `FluvioEventPublisher` is a stub that returns `Error::Streaming`).
- (Recurring appointment series landed in v0.9.)
- Production SMS gateway integrations (Twilio, MessageBird, …) — v0.8 ships the `SmsProvider` trait + `LogSmsProvider` for dev; consumers implement the trait against their gateway of choice.
- ADT^A02/A03 dedup via MPI; HL7 v2 ORM (orders are clinical, out of PAS scope). HL7 v2 batch envelope landed in v0.6.
- DICOM Modality Worklists.
- Probabilistic patient matching — handled by the sister MPI crate.

## License

Dual-licensed under either of:

- MIT License
- Apache License 2.0

at your option.
