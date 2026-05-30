# RESTful API Reference

This document is the architectural reference for the PAS HTTP layer: the response envelope, error mapping, authentication contract, and the headline request/response shapes. **For the full endpoint route list, see the table in [`README.md`](../README.md#api-endpoint-reference).**

## Library API

The PAS crate exposes a Rust library API mirroring the module layout under `src/`. See `AGENTS/models.md` for domain types.

- **Models** (`src/models/`): `Patient`, `HumanName`, `Identifier`, `Practitioner`, `Department`, `Facility`, `Ward`, `Room`, `Bed`, `BedStatus`, `Encounter`, `EncounterClass`, `EncounterStatus`, `Admission`, `Transfer`, `Discharge`, `BedAssignment`, `Schedule`, `ScheduleOwner`, `Slot`, `SlotStatus`, `Appointment`, `AppointmentStatus`, `Referral`, `WaitlistEntry`, `Priority`, `RTTPathway`, `RTTClockEvent`, `compute_active_weeks`, `LetterTemplate`, `GeneratedLetter`, `Account`, `Charge`, `Invoice`, `Payment`, `Money`, `Iso4217`, `Consent`.
- **Services** (`src/{adt, scheduling, waitlist, resources, billing, communication}/`): each takes a `DatabaseConnection` and `Arc<dyn EventPublisher>` and orchestrates the state machine inside one transaction with audit + outbox writes.
- **Persistence** (`src/db/`): SeaORM entities, repositories, transactional outbox, audit log.
- **Search** (`src/search/`): Tantivy `SearchEngine::{new, index_patient, delete_patient, search}`.
- **Streaming** (`src/streaming/`): `EventPublisher` trait, `InMemoryEventPublisher`, `FluvioEventPublisher` stub, `DomainEvent`, `dispatcher::run` / `dispatcher::tick`.
- **Validation, privacy, observability, config**: in their respective modules.

## Response Envelope

All REST endpoints (everything under `/api/`) return:

```json
{ "success": true,  "data": { ... }, "error": null }
{ "success": false, "data": null,    "error": { "code": "...", "message": "..." } }
```

FHIR endpoints (`/fhir/...`) return canonical FHIR resources on success and FHIR `OperationOutcome` JSON on failure.

## HTTP Status Code Mapping

The handler maps `crate::Error` variants deterministically:

| `Error` variant            | REST status | REST code                  | FHIR status |
| -------------------------- | ----------- | -------------------------- | ----------- |
| `NotFound`                 | 404         | `NOT_FOUND`                | 404 (`not-found`)  |
| `Validation`               | 400         | `VALIDATION`               | 400 (`invalid`)    |
| `Fhir`                     | 400         | `FHIR`                     | 400 (`invalid`)    |
| `Conflict`                 | 409         | `CONFLICT`                 | 409 (`conflict`)   |
| `InvalidStateTransition`   | 409         | `INVALID_STATE_TRANSITION` | 409 (`conflict`)   |
| `Database`                 | 500         | `DATABASE`                 | 500 (`exception`)  |
| `Search`                   | 500         | `SEARCH`                   | 500 (`exception`)  |
| `Streaming`                | 500         | `STREAMING`                | 500 (`exception`)  |
| `Render`                   | 500         | `RENDER`                   | 500 (`exception`)  |
| `Config`                   | 500         | `CONFIG`                   | 500 (`exception`)  |
| `Internal`                 | 500         | `INTERNAL`                 | 500 (`exception`)  |

Success responses use `200 OK` (or `204 No Content` for `DELETE /fhir/Patient/:id`).

## Authentication

When the `API_TOKEN` env var is set, every `/api/*` request — except `/api/health` — must carry `Authorization: Bearer <token>` matching the configured value. Mismatched or missing tokens return `401 + ApiResponse::err("UNAUTHORIZED", ...)`. The bearer middleware is closer to the handlers than CORS/trace/compress, so a 401 still has the response middleware applied to it.

When `API_TOKEN` is unset, the API runs in **trusted-caller mode** with a startup `warn!` log. Full JWT/OAuth identity-provider integration is out of scope for v0.1; bearer-token is the implemented mechanism.

## User Context

User context for the audit log is taken from headers on every state-changing request:

- `X-User-Id` — caller identity string
- `X-User-Ip` — caller IP
- `X-User-Agent` — caller user-agent string

All three are optional and recorded as-is on the `audit_log` row.

## CORS

Configurable via `CORS_ORIGINS` env var (comma-separated allowlist). Empty/unset = permissive with a startup warn; otherwise restricts to the listed origins.

## Headline Request / Response Shapes

### Admit

`POST /api/admissions`

```json
{ "patient_id": "<uuid>", "bed_id": "<uuid>" }
```

Response:

```json
{
  "success": true,
  "data": {
    "encounter":      { "id": "...", "status": "in_progress", "class": "inpatient" },
    "admission":      { "id": "...", "bed_id": "...", "admitted_at": "..." },
    "bed_assignment": { "id": "...", "bed_id": "...", "released_at": null }
  }
}
```

### Transfer

`POST /api/admissions/:id/transfer`

```json
{ "new_bed_id": "<uuid>" }
```

### Discharge

`POST /api/admissions/:id/discharge` — empty body.

### Book a slot

`POST /api/slots/:id/book`

```json
{ "patient_id": "<uuid>" }
```

Response: the created `Appointment` (status=`booked`, slot_id populated).

### Add to waitlist

`POST /api/waitlist`

```json
{
  "patient_id":     "<uuid>",
  "target_service": "cardiology",
  "priority":       "urgent",
  "referral_id":    null
}
```

### Start an RTT clock

`POST /api/rtt/start`

```json
{ "patient_id": "<uuid>", "target_service": "cardiology" }
```

Response: the created `RTTPathway`. `breach_weeks` defaults to 18.

### Generate a letter

`POST /api/letters/generate`

```json
{
  "template_id":    "<uuid>",
  "patient_id":     "<uuid>",
  "appointment_id": "<uuid>",
  "channel":        "email",
  "extra":          { "appointment_date": "2026-06-15" }
}
```

The Tera render uses the patient object as a top-level context variable plus any keys in `extra`. Any value referenced in `template.required_variables` must be present in `extra` or render fails with `400 VALIDATION`.

### Post a charge

`POST /api/charges`

```json
{
  "account_id":      "<uuid>",
  "encounter_id":    null,
  "appointment_id":  "<uuid>",
  "code":            "99213",
  "description":     "Office visit, established patient",
  "amount_value":    "120.00",
  "amount_currency": "USD"
}
```

Money fields are split into `amount_value` (string decimal) + `amount_currency` (ISO-4217 code) at the wire layer so JSON numbers can never lose precision. The handler reconstitutes `crate::models::Money` server-side.

## Source Files

- `src/api/mod.rs` — `ApiResponse`, `ApiError`.
- `src/api/rest/mod.rs` — re-exports `router`, `AppState`, `RequireBearerToken`, `require_bearer`.
- `src/api/rest/handlers.rs` — every REST handler.
- `src/api/rest/routes.rs` — the route table (single source of truth in code).
- `src/api/rest/state.rs` — `AppState` (services + DB + publisher + optional Tantivy).
- `src/api/rest/auth.rs` — bearer-token middleware.
- `src/api/fhir/handlers.rs` — FHIR R5 handlers.
- `src/api/fhir/resources.rs` — bidirectional converters between FHIR JSON and `crate::models`.
- `src/api/fhir/operation_outcome.rs` — FHIR `OperationOutcome` error envelope.
- `src/api/fhir/routes.rs` — FHIR route table.

## See Also

- [../README.md](../README.md#api-endpoint-reference) — full endpoint table (the single source of truth for routes)
- [architecture.md](architecture.md) — layered design + error-variant-to-HTTP mapping
- [models.md](models.md) — domain types behind the JSON shapes
- [matching.md](matching.md) — scheduling overlap, booking concurrency, RTT clock
- [testing.md](testing.md) — integration test catalogue
