# Architecture

## System Architecture

The Patient Administration System (PAS) is a layered Rust service:

```
api → service → repository → db
```

### Layers

**api** — Axum routers for both REST (`/api/...`) and FHIR R5 (`/fhir/...`), with Utoipa-driven OpenAPI annotations. Handlers are thin shells: they extract user context from `X-User-Id` / `X-User-Ip` / `X-User-Agent` headers, decode and validate the JSON body, call a service, and serialize the response into an `ApiResponse<T>` envelope. Errors are mapped to HTTP status codes by the `ApiError` type. The api layer holds no business logic.

**service** — Modules under `src/adt/`, `src/scheduling/`, `src/waitlist/`, `src/resources/`, `src/billing/`, and `src/communication/`. Each service owns the business rules and state-machine transitions for one aggregate cluster. Services receive their dependencies (repositories, event publisher, audit log, clock) by injection and own all transactional boundaries. State changes that touch multiple rows always run inside one `DatabaseTransaction`.

**repository** — Repositories under `src/db/repositories/`, one per aggregate cluster (patient, encounter, admission, appointment, bed, schedule, slot, waitlist, rtt, letter, billing, consent, audit, outbox). Each talks to SeaORM. The `db::with_txn(db, |txn| async { ... })` helper composes multiple repository calls inside one `DatabaseTransaction`. `AuditLogRepository::log` and `OutboxRepository::publish` accept any `&C: ConnectionTrait`, so callers can compose them with the primary writes in the same transaction.

**db** — `src/db/mod.rs` exposes a `connect(url)` helper (pool size 2-10, 8s connect timeout, 60s idle timeout) and the `with_txn` wrapper. `src/db/entities/` holds 29 SeaORM entities, one file per table. `src/db/repositories/audit.rs` and `src/db/repositories/outbox.rs` are the transactional sinks for the cross-cutting compliance trail.

## Aggregates

| Aggregate            | Root                  | Members                                                       |
| -------------------- | --------------------- | ------------------------------------------------------------- |
| Patient              | `Patient`             | identifiers, additional names, addresses, contacts, contacts  |
| Workforce            | `Practitioner`        | `PractitionerRole`, `Department`                              |
| Facility             | `Facility`            | `Ward`, `Room`, `Bed` (each its own table; loaded by id)      |
| Encounter            | `Encounter`           | (state machine root for inpatient/outpatient/ED)              |
| Admission            | `Encounter`           | `Admission`, `Transfer`, `Discharge`, `BedAssignment`         |
| Schedule             | `Schedule`            | `Slot` (one Schedule has many Slots)                          |
| Appointment          | `Appointment`         | optional `Slot` link, optional waitlist provenance            |
| Waitlist             | `WaitlistEntry`       | optional originating `Referral`                               |
| RTT                  | `RTTPathway`          | append-only `RTTClockEvent` log                               |
| Letter               | `LetterTemplate`      | `GeneratedLetter` per render                                  |
| Billing              | `Account`             | `Charge`, `Invoice`, `Payment`                                |
| Consent              | `Consent`             | (one record per patient × `ConsentType`)                      |

Aggregate-roots are loaded and saved through their repository as a single transactional unit. For example, the ADT service loads `Encounter` + active `BedAssignment` + target `Bed` together inside one DB transaction and writes them back atomically.

## Transactional Outbox

Every state change writes three rows in one database transaction:

1. The entity row itself (insert or update).
2. An `audit_log` row recording `(entity_type, entity_id, action, old_value JSONB, new_value JSONB, user_id, user_ip, user_agent, at)`.
3. An `outbox_events` row recording the domain event payload.

This guarantees the audit log and the event stream are never inconsistent with the canonical data: either all three land or none do.

A background dispatcher (`src/streaming/dispatcher.rs`) is spawned in `main.rs` at startup. It polls `outbox_events WHERE published = false` every 2 seconds, batches up to 64 rows, forwards each to the configured [`EventPublisher`], and marks the row `published = true` on success. Failed publishes are logged and retried on the next tick — no row is deleted. The partial index `outbox_events (published) WHERE published = false` keeps the hot path cheap. v0.1 ships an `InMemoryEventPublisher` (used in production and tests); a `FluvioEventPublisher` stub exists but always returns `Error::Streaming`.

Domain event payloads include `PatientCreated`/`Updated`/`Deleted`, `EncounterAdmitted`/`Transferred`/`Discharged`/`Cancelled`, `AppointmentBooked`/`Cancelled`/`CheckedIn`/`Completed`/`NoShow`, `WaitlistAdded`/`Removed`, `RTTClockStarted`/`Paused`/`Resumed`/`Stopped`, `BedStatusChanged`, `LetterGenerated`, `ChargePosted`, `InvoiceFinalized`, `PaymentPosted`.

## State Machines

The PAS has four explicit state machines. Each is implemented as an enum with a `try_transition_to(self, next) -> Result<Self>` method that returns `Error::InvalidStateTransition` for any illegal move (including same-state).

### Encounter status

```
Planned    -> Arrived | Cancelled
Arrived    -> InProgress | Cancelled
InProgress -> OnLeave | Finished | Cancelled
OnLeave    -> InProgress | Finished | Cancelled
Finished   -> (terminal)
Cancelled  -> (terminal)
```

### Appointment status

```
Proposed -> Booked | Cancelled
Booked   -> Arrived | Cancelled | NoShow
Arrived  -> Fulfilled | Cancelled
Fulfilled -> (terminal)
Cancelled -> (terminal)
NoShow   -> (terminal)
```

### Slot status

```
Free       -> Busy | BlockedOut
Busy       -> Free
BlockedOut -> Free
```

Same-state transitions are explicitly rejected.

### Bed status

```
Available    -> Occupied | Reserved | OutOfService
Occupied     -> Cleaning | OutOfService
Cleaning     -> Available | OutOfService
Reserved     -> Occupied | Available | OutOfService
OutOfService -> Available
```

Beds cannot go directly from `Occupied` to `Available` — they must pass through `Cleaning`. This is a deliberate invariant of the ADT flow: discharge releases a bed to `Cleaning`, not to `Available`.

The RTT clock has a related lifecycle (`Active` → `Paused` → `Active` → `Stopped`) but it is realized as an append-only event log (`RTTClockEvent`), not as an in-place enum transition. See [matching.md](matching.md) for the clock arithmetic.

## Time Handling

- **Domain layer**: `chrono::DateTime<Utc>` throughout. Naive local times are never accepted.
- **DB layer**: SeaORM `DateTimeWithTimeZone` (a `chrono::DateTime<FixedOffset>`) for columns; conversion to/from `Utc` happens at the repository boundary.
- **Effective time vs system time**: every entity has `created_at` / `updated_at` for system bookkeeping. Entities with their own event time additionally carry domain-time columns: `Encounter.period_start` / `period_end`, `Admission.admitted_at`, `Transfer.transferred_at`, `Discharge.discharged_at`, `Slot.start_datetime` / `end_datetime`, `Appointment.start_datetime` / `end_datetime`, `RTTClockEvent.event_at`. The distinction matters: a corrective backdated entry can move `event_at` without altering `created_at`.

## Soft Delete

Soft-delete (a `deleted_at TIMESTAMPTZ` column, never `DELETE FROM`) is required on the tables that hold administrative records subject to audit retention:

- `patients`
- `encounters`
- `appointments`

Repositories filter out rows with non-null `deleted_at` by default; an explicit `include_deleted` flag is required to surface them. Other tables (e.g. `bed_assignments`, `rtt_clock_events`, `audit_log`, `outbox_events`) are append-only or operational and are never deleted.

## Module Structure

```
src/
├── lib.rs                  Public re-exports, crate docs
├── main.rs                 Binary entry (Axum server)
├── error.rs                Error enum, Result alias
├── config/                 Env/config loader
├── models/                 Pure-Rust domain types (no DB deps)
│   ├── mod.rs              Gender, Address, ContactPoint, Iso4217, Money, TimeRange, ...
│   ├── patient.rs          Patient, HumanName, EmergencyContact
│   ├── identifier.rs       Identifier, IdentifierType (MRN/NHS/SSN/DL/Passport/Other)
│   ├── practitioner.rs     Practitioner, PractitionerRole, Department
│   ├── facility.rs         Facility, Ward, Room, Bed, BedStatus
│   ├── encounter.rs        Encounter, EncounterClass, EncounterStatus
│   ├── admission.rs        Admission, Transfer, Discharge, BedAssignment
│   ├── schedule.rs         Schedule, ScheduleOwner, Slot, SlotStatus
│   ├── appointment.rs      Appointment, AppointmentStatus, CancellationReason
│   ├── waitlist.rs         Referral, WaitlistEntry, Priority, WaitlistStatus
│   ├── rtt.rs              RTTPathway, RTTClockEvent, RTTStatus, RTTEventKind, compute_active_weeks
│   ├── communication.rs    LetterTemplate, GeneratedLetter, DeliveryChannel, LetterStatus
│   ├── billing.rs          Account, Charge, Invoice, Payment + statuses, PaymentMethod
│   └── consent.rs          Consent, ConsentType, ConsentStatus
├── adt/                    Admission/Discharge/Transfer service
├── scheduling/             Slot allocation, conflict detection, booking
├── waitlist/               Priority queueing, RTT clock arithmetic
├── resources/              Bed availability, ward occupancy
├── billing/                Charge accumulation, invoice generation, payment posting
├── communication/          Tera template rendering, letter generation
├── hl7v2/                  HL7 v2 parser, encoder, mapping, ACK, MLLP framing + TCP listener
├── interchange/            Bulk JSON / XML / TSV / CSV import-export via PatientRow
├── bin/                    Auxiliary binaries (currently `pas-seed`)
├── db/
│   ├── mod.rs              DB pool, connect(), with_txn()
│   ├── entities/           SeaORM entity per table (29 files)
│   └── repositories/       One repo file per aggregate, plus audit.rs and outbox.rs
├── api/
│   ├── mod.rs              ApiResponse, ApiError, AppState
│   ├── rest/               Axum router and handlers, Utoipa OpenAPI
│   ├── fhir/               FHIR R5 resource converters and endpoints (incl. batch + transaction Bundles)
│   ├── dashboard.rs        Ops dashboard (Tera + HTMX, Lily Design System markup)
│   └── openapi.rs          ApiDoc aggregating every `#[utoipa::path]` handler
├── search/                 Tantivy patient index
├── streaming/              EventPublisher trait, InMemory + Fluvio stub + Hl7v2MllpPublisher, dispatcher::run/tick
├── validation/             Validators (E.164 phone, postal code, dates, charge amount, RTT order)
├── privacy/                Data masking, GDPR export, consent gating helper
└── observability/          tracing-subscriber init (OTLP exporter documented but not wired)

migrations/                 SeaORM migration crate + `pas-migrate` CLI binary
tests/                      Integration tests (most gated on DATABASE_URL — 19 files, 28 tests)
benches/                    Criterion benchmarks (adt, scheduling, waitlist)
AGENTS/                     This documentation tree
```

## Error Handling

A single `Error` enum in `src/error.rs` covers eleven variants (`Database`, `NotFound`, `Validation`, `InvalidStateTransition`, `Conflict`, `Search`, `Streaming`, `Fhir`, `Render`, `Config`, `Internal`). Helper constructors keep call sites terse (`Error::not_found(...)`, `Error::validation(...)`, `Error::invalid_transition(...)`). Every public API returns `Result<T>` (the crate-local alias for `std::result::Result<T, Error>`).

The api layer converts errors to HTTP status codes:

| Variant                  | Status |
| ------------------------ | -----: |
| `Validation`             |    422 |
| `NotFound`               |    404 |
| `Conflict`               |    409 |
| `InvalidStateTransition` |    409 |
| `Database`               |    500 |
| `Internal`               |    500 |

## Database Schema

29 tables, grouped: identity (1: `patients`), workforce (3: `practitioners`, `practitioner_roles`, `departments`), resources (4: `facilities`, `wards`, `rooms`, `beds`), ADT (4: `encounters`, `admissions`, `transfers`, `discharges`) + `bed_assignments`, scheduling (3: `schedules`, `slots`, `appointments`), waitlist + RTT (4: `referrals`, `waitlist_entries`, `rtt_pathways`, `rtt_clock_events`), communication (2: `letter_templates`, `generated_letters`), billing (4: `accounts`, `charges`, `invoices`, `payments`), consent (1: `consents`), audit + events (2: `audit_log`, `outbox_events`).

v0.1 trade-off: repeated child collections on `patients` and `practitioners` (`identifiers`, `additional_names`, `telecom`, `addresses`, `emergency_contacts`) are stored inline as `JSONB` columns instead of as separate tables. The migration comment in `migrations/src/m20260520_000001_init.rs` flags this for future normalization.

Strategic indexes target the hot paths:

- `appointments (patient_id, start_datetime)` — per-patient overlap check
- `slots (schedule_id, start_datetime) WHERE status = 'Free'` — slot search
- `bed_assignments (bed_id) WHERE released_at IS NULL` — at most one active per bed (partial UNIQUE)
- `waitlist_entries (target_service, priority, created_at)` — priority queue scan
- `rtt_clock_events (pathway_id, event_at)` — clock arithmetic ordering
- `outbox_events (published) WHERE published = false` — dispatcher hot path

Authoritative source: `migrations/src/m20260520_000001_init.rs`.

## See Also

- [models.md](models.md) — every field on every model
- [restful.md](restful.md) — REST envelope, error mapping, request shapes
- [matching.md](matching.md) — scheduling overlap, slot/bed concurrency, RTT clock
- [interchange.md](interchange.md) — bulk import-export, FHIR Bundle write semantics, HL7 v2 surface
- [testing.md](testing.md) — unit + integration test layout
- [../README.md](../README.md) — full endpoint table
