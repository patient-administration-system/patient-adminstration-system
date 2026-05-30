# Interchange formats

v0.2 adds bulk JSON / XML / TSV import and export for patient demographic data, on top of the existing JSON-over-HTTP and FHIR R5 surfaces. This document describes the wire shape, the lossy projection, the REST endpoints, and the FHIR R5 collection `Bundle` extension.

## PatientRow — the common projection

Every v0.2 interchange format serializes the same flat row, defined in [`src/interchange/mod.rs`](../src/interchange/mod.rs) as `PatientRow`. Field order is fixed (it's part of the TSV contract):

| Field          | Type           | Source                                                           |
|----------------|----------------|------------------------------------------------------------------|
| `id`           | UUID string    | `Patient.id`                                                     |
| `mrn`          | string         | First `Identifier` with `IdentifierType::MRN`, else empty        |
| `family_name`  | string         | `Patient.name.family`                                            |
| `given_names`  | string         | `Patient.name.given.join(" ")` (space-joined)                    |
| `gender`       | string         | `male` / `female` / `other` / `unknown`                          |
| `birth_date`   | string         | `YYYY-MM-DD`, empty if absent                                    |
| `phone`        | string         | First `ContactPoint` with `system=Phone`                         |
| `email`        | string         | First `ContactPoint` with `system=Email`                         |
| `line1`        | string         | First `Address.line1`                                            |
| `city`         | string         | First `Address.city`                                             |
| `postal_code`  | string         | First `Address.postal_code`                                      |
| `country`      | string         | First `Address.country`                                          |
| `active`       | boolean        | `Patient.active`                                                 |

### Lossy fields

`PatientRow` is intentionally narrow. The following do **not** survive a round-trip through a `PatientRow`:

- Additional names (`Patient.additional_names`).
- Deceased status and timestamp.
- Emergency contacts.
- Marital status.
- MPI id (re-resolution would have to happen out of band).
- More than one identifier (only the first MRN).
- Address line 2, state.
- Telecom values beyond the first phone and first email.

For higher-fidelity exchange use the FHIR R5 endpoints (`GET /fhir/Patient/:id`) or the native PAS JSON shape (`GET /api/patients/:id/export`).

## Formats

### JSON

Top-level JSON array of `PatientRow`. Endpoint emits a compact array; the example in `examples/patients.json` is pretty-printed for readability.

- `interchange::json::patients_to_json_pretty(&rows) -> Result<String>`
- `interchange::json::patients_to_json_compact(&rows) -> Result<String>`
- `interchange::json::patients_from_json(&str) -> Result<Vec<PatientRow>>`

### XML

`<patients><patient>…</patient></patients>` document, one child element per `PatientRow` field. The XML declaration `<?xml version="1.0" encoding="UTF-8"?>` is emitted on the export side. The reader is tolerant of whitespace between elements.

- `interchange::xml::patients_to_xml(&rows) -> Result<String>`
- `interchange::xml::patients_from_xml(&str) -> Result<Vec<PatientRow>>`

### TSV

Tab-separated values with a fixed header row. Tabs are chosen over commas because address lines frequently contain commas; tabs almost never appear in demographic strings and therefore don't need quoting. The `csv` crate is used under the hood with `delimiter(b'\t')`.

- `interchange::tsv::patients_to_tsv(&rows) -> Result<String>`
- `interchange::tsv::patients_from_tsv(&str) -> Result<Vec<PatientRow>>`

### CSV

Comma-separated values, RFC-4180-style. Same row shape as TSV but with `,` as the delimiter and standard CSV quoting (fields containing commas, quotes, or newlines get double-quoted). Useful when the consumer can only accept CSV. Prefer TSV when free-text fields may contain commas — quoting overhead disappears.

- `interchange::csv::patients_to_csv(&rows) -> Result<String>`
- `interchange::csv::patients_from_csv(&str) -> Result<Vec<PatientRow>>`

## REST endpoints

| Method | Path                            | Body / response                                             |
|--------|---------------------------------|-------------------------------------------------------------|
| GET    | `/api/patients/export.json`     | `ApiResponse<[PatientRow]>` (JSON array)                    |
| GET    | `/api/patients/export.xml`      | XML document, `Content-Type: application/xml`               |
| GET    | `/api/patients/export.tsv`      | TSV, `Content-Type: text/tab-separated-values`              |
| GET    | `/api/patients/export.csv`      | CSV, `Content-Type: text/csv`                               |
| POST   | `/api/patients/import`          | Format picked from `Content-Type` header (see below)        |

`POST /api/patients/import` content-type sniffing:

- `application/xml` or `text/xml` → XML
- `text/tab-separated-values` or `text/tsv` → TSV
- `text/csv` → CSV
- anything else (including `application/json`) → JSON

The import is **idempotent**: rows whose `id` already exists are skipped, not overwritten. The response is:

```json
{ "success": true, "data": { "inserted": 2, "skipped": 1, "failed": 0 } }
```

Export is capped at 10,000 active patients. If you have more, page via the FHIR `Bundle` endpoint (`GET /fhir/Patient?_count=N`) or the native `GET /api/patients?limit=N`.

## FHIR R5 expansion

v0.2 also widens the FHIR R5 read surface:

| Method | Path                          | Returns                              |
|--------|-------------------------------|--------------------------------------|
| GET    | `/fhir/Patient?_count=N`      | Collection `Bundle` of patients      |
| GET    | `/fhir/Practitioner/{id}`     | FHIR `Practitioner` resource         |
| GET    | `/fhir/Schedule/{id}`         | FHIR `Schedule` resource             |
| GET    | `/fhir/Slot/{id}`             | FHIR `Slot` resource                 |
| GET    | `/fhir/Location/{id}`         | FHIR `Location` (PAS `Bed`)          |
| POST   | `/fhir`                       | Batch / transaction Bundle of writes |

The read Bundle is a **collection** Bundle (`type: collection`). The write Bundle endpoint accepts `type: batch` or `type: transaction` and returns a corresponding `batch-response` / `transaction-response` Bundle. Supported resource types in write entries: `Patient`, `Encounter`, `Appointment`, `Coverage` (v0.13 — same `CoverageRepository::create` path as `POST /api/coverages`), `Practitioner`, `Schedule`, `Slot` (v0.23 — same paths as the direct `POST /fhir/<Type>` endpoints from v0.21). Each entry must carry `request: { "method": "POST", "url": "<type>" }`.

**Transaction semantics**: `type: transaction` is genuinely all-or-nothing. Every entry runs inside a single `sea_orm::DatabaseTransaction`. On any entry failure (parse, validation, DB error), the transaction is rolled back and the endpoint returns `400 + OperationOutcome` whose `diagnostics` names the offending entry index. Search-index updates (Tantivy) are deferred until after the commit so rolled-back rows are never visible to search.

Errors continue to return FHIR `OperationOutcome` JSON with the same status mapping as v0.1.

## Import semantics

The import path runs these steps per row:

1. Parse the row into a `PatientRow` (validation errors → 400).
2. Project it to a `Patient` via `PatientRow::to_patient()`.
3. If the `id` already exists, increment `skipped` and continue.
4. Otherwise `INSERT` the row. On success, index it in Tantivy if search is enabled.
5. Any database error → `failed += 1`.

There is no audit-log entry written for import (rows are created with a fresh `created_at`/`updated_at` but without going through the standard patient handler). If you need an audit trail, ingest one row at a time via `POST /api/patients` instead — slower, but every insert is audited.

## HL7 v2 ADT (v0.3)

For interop with legacy clinical systems, v0.3 added a pipe-delimited HL7 v2 surface. Scope is intentionally narrow:

- Standard delimiter set only: `|^~\&`. Custom delimiters return `400 + AR`.
- Supported segments: `MSH`, `EVN`, `PID`, `PV1`.
- Supported message types for domain mapping: `ADT^A01`, `ADT^A04`, `ADT^A08`, `ADT^A28`. The handler only consumes the `PID` segment for now (creating a patient); `PV1`-driven admission flow is a future task.
- Outbound: [`encode_adt_a28`](../src/hl7v2/mapping.rs) builds an ADT^A28 message from a [`Patient`].

### Endpoints

| Method | Path                       | Purpose                                                                 |
|--------|----------------------------|-------------------------------------------------------------------------|
| POST   | `/api/hl7/v2/parse`            | Parse and return structured JSON for inspection.                        |
| POST   | `/api/hl7/v2/batch`            | Accept an `FHS`/`BHS`/`BTS`/`FTS` envelope of up to 1000 ADT messages; dispatch each independently; respond with one batch ACK envelope. (*v0.6*) |
| POST   | `/api/hl7/v2/patient`          | Create a Patient from the PID; respond with `ACK^...`.                  |
| POST   | `/api/hl7/v2/update`           | ADT^A08: merge demographics from PID into the existing patient row. (*v0.4*) |
| POST   | `/api/hl7/v2/admit`            | ADT^A01: create Patient from PID, admit to bed identified by PV1-3.3.   |
| POST   | `/api/hl7/v2/transfer`         | ADT^A02: find open admission by MRN (PID-3.1), transfer to PV1-3.3 bed. |
| POST   | `/api/hl7/v2/discharge`        | ADT^A03: find open admission by MRN and discharge.                      |
| POST   | `/api/hl7/v2/cancel-admit`     | ADT^A11: cancel the currently-open admission — bed → Cleaning, encounter → Cancelled. (*v0.4*) |
| POST   | `/api/hl7/v2/cancel-discharge` | ADT^A13: reinstate the most-recently-discharged admission to its original bed. (*v0.4*) |
| POST   | `/api/hl7/v2/merge`            | ADT^A40: merge source patient identified by MRG-1.1 MRN into survivor identified by PID. Same merge logic as `POST /api/patients/{id}/merge-into/{target_id}`. (*v0.18*) |
| POST   | `/api/hl7/v2/dft`              | DFT^P03: post one or more charges from FT1 segments. Dedup-or-creates patient from PID; auto-creates an open account in the FT1-11.2 currency when none exists. Only `FT1-6 = CG` accepted (PY / AJ AE-ACK). v0.20 makes multi-FT1 messages all-or-nothing in one DB transaction; mixed FT1-11.2 currencies → 400. (*v0.19 + v0.20*) |
| POST   | `/api/hl7/v2/mfn-staff`        | MFN^M02: master file notification for the practitioner roster. One or more MFE+STF pairs per message; each pair drives `MAD` (create), `MUP` (replace), or `MDL` (soft-delete via `active=false`). Atomic per message. EMR staff id (MFE-4 / STF-1) stored as `Identifier { type: Other, system: "urn:hl7v2:staff:id" }` for stable round-trip. (*v0.24*) |
| POST   | `/api/hl7/v2/mfn-location`     | MFN^M05: master file notification for the bed roster. One or more MFE+LOC pairs per message; LOC-1 is `PL`-typed (LOC-1.1 = parent room code, LOC-1.3 = bed code), LOC-2 = bed name. `MAD` / `MUP` / `MDL` semantics mirror MFN^M02. `MDL` flips the bed to `OutOfService` via the same operator-bypass used for ADT^A13. Atomic per message. (*v0.26*) |
| POST   | `/api/hl7/v2/schedule-book`        | SIU^S12: book appointment from SCH + PID (start in SCH-11, duration in SCH-9). AA carries the assigned filler uuid in MSA-3. (*v0.16*) |
| POST   | `/api/hl7/v2/schedule-reschedule`  | SIU^S13: change the time of an appointment identified by SCH-2. Runs an overlap-excluding check so the row's own current window doesn't flag itself. 409 + AE on terminal status or sibling conflict. (*v0.17*) |
| POST   | `/api/hl7/v2/schedule-modify`      | SIU^S14: update `Appointment.reason` from SCH-7. SCH-11 ignored — use S13 for time changes. (*v0.17*) |
| POST   | `/api/hl7/v2/schedule-cancel`      | SIU^S15: cancel appointment by filler uuid in SCH-2. 404 + AE if unknown, 409 + AE if already terminal. (*v0.16*) |

### PID segment → Patient mapping

| Field   | Source                                | PAS field                                       |
|---------|---------------------------------------|-------------------------------------------------|
| PID-3   | `value^^^facility^MR`                 | `identifiers[0]` (MRN), `system = urn:oid:facility:<facility>` |
| PID-5   | `family^given^middle^suffix^prefix`   | `name` (Official)                               |
| PID-7   | `YYYYMMDD`                            | `birth_date`                                    |
| PID-8   | `M` / `F` / `O` / `U`                 | `gender`                                        |
| PID-11  | `line1^line2^city^state^postal^country` | `addresses[0]` (Home)                         |
| PID-13  | free text                             | `telecom[]` phone                               |
| PID-14  | `…@…`                                 | `telecom[]` email (only if it contains `@`)     |

### ACK codes

The `/api/hl7/v2/patient` endpoint always responds with a v2 ACK envelope. The MSA-1 code is one of:

- `AA` — Application Accept. Patient was created. HTTP 200.
- `AE` — Application Error. The message parsed but the application rejected it (PID missing, family name empty, unsupported message type, etc.). HTTP 400.
- `AR` — Application Reject. The message failed to parse at all (bad delimiters, missing MSH). HTTP 400 or 500.

### PV1 segment → Admission (ADT^A01)

The `/api/hl7/v2/admit` endpoint reads:

- `PV1-2` (patient class): expected to be `I` for inpatient. Not currently enforced.
- `PV1-3` (assigned patient location), parsed as `point_of_care^room^bed^facility^…`.
- `PV1-3.3` (bed sub-component): the **bed code** the PAS uses to look up the destination bed via [`BedRepository::find_by_code`]. The bed must already exist (the PAS does not auto-create wards or beds from inbound ADT) and must be `Available`.

On success: a new Patient is created (no MPI dedup in v0.3 — duplicate sends produce duplicate patients), an inpatient `Encounter` is opened, the bed flips to `Occupied`, an active `BedAssignment` is inserted, audit + outbox rows are written, and an `EncounterAdmitted` event is published. The full `AdtService::admit` transactional flow runs as if you'd called `POST /api/admissions` directly.

### MLLP framing (optional TCP listener)

Real hospital networks transport HL7 v2 over MLLP — a tiny envelope around the pipe-delimited message:

```text
<SB><payload><EB><CR>
SB = 0x0B  (vertical tab)
EB = 0x1C  (file separator)
CR = 0x0D
```

The PAS exposes this transport as an opt-in TCP listener:

- Set `HL7V2_MLLP_BIND=0.0.0.0:2575` (or any `host:port`) in the environment.
- The binary spawns a [`tokio::net::TcpListener`] alongside the HTTP server.
- Each accepted connection reads one or more MLLP frames; per frame the listener inspects MSH-9 to pick the matching `/api/hl7/v2/...` route and dispatches via `axum::Router::oneshot`. The HTTP response body is the ACK envelope, which the listener writes back framed.
- The same auth (`API_TOKEN` bearer) does **not** apply on the MLLP path. Use network-level controls (private VLAN, firewall, mTLS via a sidecar) to gate MLLP traffic.

The framing primitives (`read_frame` / `write_frame`) live in [`src/hl7v2/mllp.rs`](../src/hl7v2/mllp.rs); the listener in [`src/hl7v2/listener.rs`](../src/hl7v2/listener.rs).

### Outbound HL7 v2 (push to a downstream EMR)

Set `HL7V2_OUTBOUND_PEER=host:port` to make the PAS *emit* ADT messages to a downstream MLLP receiver whenever an admission/transfer/discharge happens. Under the hood:

1. ADT services (`admit`, `transfer`, `discharge`) write an `EncounterAdmitted` / `EncounterTransferred` / `EncounterDischarged` row to the `outbox_events` table inside the same DB transaction as the state change.
2. The outbox dispatcher polls every 2s and calls `EventPublisher::publish` on each unpublished row.
3. When `HL7V2_OUTBOUND_PEER` is set, the configured publisher is [`Hl7v2MllpPublisher`](../src/streaming/hl7v2_publisher.rs), which:
   - Decodes the JSON payload to recover `patient_id` (and `bed_id` / `new_bed_id` when applicable).
   - Hydrates the full Patient + Bed from the database.
   - Builds an `ADT^A01` / `A02` / `A03` via [`encode_adt_a01`](../src/hl7v2/mapping.rs) and friends.
   - Opens a fresh TCP connection to the peer, writes the MLLP frame, reads the ACK.
   - Returns `Ok(())` only on `MSA|AA`. Any other ACK (or a network/timeout error) is reported as `Err`, which leaves the outbox row unpublished — the dispatcher retries on the next tick.

Event types other than the three ADT ones (e.g. `PatientCreated`, `AppointmentBooked`) are silently dropped (publish returns `Ok(())`) so the outbox doesn't fill up with un-forwardable events. The MLLP path is fire-and-forget per event; for higher throughput, pool connections.

```bash
HL7V2_OUTBOUND_PEER=emr.example.com:2575 cargo run --bin patient-administration-system
```

### Limitations

- No batch envelope (`FHS` / `BHS`); one message per POST.
- Escape sequences `\F\`/`\S\`/`\T\`/`\R\`/`\E\` are honored at the domain↔wire boundary by `pid_from_patient` (escape on encode) and `patient_from_pid` (unescape on decode). A patient name like `O^Brien-Jones` round-trips losslessly. Unknown sequences (`\H\`, `\X41\`, vendor extensions) pass through verbatim rather than being silently dropped.
- ADT^A02 / ADT^A03 lookup uses PID-3.1 (the MRN value) — PV1-19 (visit number) is **not** consulted. If two patients share an MRN value (they shouldn't, but the schema doesn't enforce it across systems), the first match wins.
- A02 / A03 require the patient to have exactly one currently-open inpatient admission. If they have none, the ACK is AE with `no currently-open admission`. If they somehow have multiple (an invariant violation), the most-recently-assigned one is chosen.
- MLLP TCP listener is opt-in via `HL7V2_MLLP_BIND` (typically `:2575`). When off, the HTTP endpoints are the only path. The MLLP listener routes incoming frames to the same handlers as the HTTP endpoints — there is no separate dispatch path.
- ADT^A01 / ^A28 ingest deduplicates by **exact MRN match** before insert. If PID-3.1 names an MRN that already exists in the `patients` table (non-deleted), the existing row is reused — no duplicate insert, no audit log entry, no Tantivy re-index. The AA ACK includes `matched existing patient <uuid>` in MSA-3 so the sender can confirm. Probabilistic dedup (name + DOB + SSN) still lives in the sister MPI crate; pair the two when you need it.

### v0.4 lifecycle messages

The three v0.4 message types extend the inbound surface so an upstream
EMR can express the full ADT correction lifecycle, not just the
forward path:

- **ADT^A08 (update patient)** → `/api/hl7/v2/update`. Looks up the
  patient by PID-3.1 and **merges** the inbound demographics over the
  existing row. PID-sourced fields (`identifiers`, `name`, `telecom`,
  `gender`, `birth_date`, `addresses`) are overwritten; everything PID
  does not carry (`mpi_id`, `additional_names`, `deceased*`,
  `emergency_contacts`, `marital_status`, `created_at`, `active`, `id`)
  is preserved. Single DB transaction writes the row + an audit log
  entry (`action = "update_via_hl7v2_a08"`) + an outbox event
  (`event_type = "PatientUpdated"`, payload includes `source:
  "hl7v2_a08"`). Tantivy re-index after commit. AA ACK reports
  `updated patient <uuid>` in MSA-3. Unknown MRN → AE.

- **ADT^A11 (cancel admit)** → `/api/hl7/v2/cancel-admit`. Reverses a
  wrong admit: locates the patient's currently-open admission by
  PID-3.1, releases the active `BedAssignment`, flips the bed to
  `Cleaning` (the regular `Occupied → Cleaning` transition), moves the
  encounter to `Cancelled`. The `admissions` row is **preserved** so
  the erroneous admit stays visible in patient history. Outbox event
  `EncounterCancelled` with `reason: "hl7v2_a11"`.

- **ADT^A13 (cancel discharge)** → `/api/hl7/v2/cancel-discharge`.
  Reverses a wrong discharge: locates the most-recently-discharged
  admission by PID-3.1, deletes the `discharges` row, force-flips the
  bed back to `Occupied`, force-sets the encounter back to
  `InProgress`, inserts a fresh active `BedAssignment` for the original
  bed. The original bed must currently be in `Available` or `Cleaning`
  — anything else (`Occupied` by another patient, `Reserved`,
  `OutOfService`) returns AE because the discharge can't be safely
  undone. The two state-machine bypasses (`Finished → InProgress` on
  the encounter; `Available|Cleaning → Occupied` on the bed) are the
  **only** documented exceptions to spec.md §5.4 and are gated to this
  flow only. Outbox event `EncounterDischargeCancelled`.

### v0.6 batch envelope

`POST /api/hl7/v2/batch` accepts the standard HL7 v2 batch envelope and
dispatches every contained message to the matching single-message
handler. Wire shape:

```text
FHS|^~\&|sender|FAC|recv|FAC|datetime||...     ← optional file header
BHS|^~\&|sender|FAC|recv|FAC|datetime||...     ← optional batch header
MSH|^~\&|...|ADT^A28|MSG-1|P|2.5
[segments]
MSH|^~\&|...|ADT^A01|MSG-2|P|2.5
[segments]
...
BTS|<msg_count>                                ← optional batch trailer
FTS|<batch_count>                              ← optional file trailer
```

Rules:

- Up to **1000 messages per transmission** (`MAX_BATCH_MESSAGES`).
  Anything larger → `400 + AR` envelope with a diagnostic.
- **Per-message independence**: each contained `MSH` is dispatched to
  the same handler the HTTP single-message route would use. A bad
  message returns its own `MSA|AE|<msg-id>` inside the envelope; the
  surrounding messages still process. Matches FHIR `batch` Bundle
  semantics (the FHIR `transaction` Bundle is the all-or-nothing
  option).
- **One batch per request**: more than one `BHS` (multi-batch files)
  → `400 + AR`. Real-world senders almost always emit one batch per
  transmission anyway.
- **`BTS` count is informational**: the parser does not enforce that
  it matches the actual message count. Some senders get it wrong;
  rejecting them helps nobody.
- **HTTP status**: always `200 OK` when the envelope itself parses,
  even if every individual message failed. The per-message AA/AE/AR
  inside the response envelope is the sender's signal. `400` is
  reserved for envelope-level problems (malformed, oversize, multi-
  batch).
- **MLLP path**: the listener routes any payload starting with `FHS`
  or `BHS` to `/api/hl7/v2/batch`, so the batch surface is reachable
  over MLLP without a second listener.
- **Response shape**: one batch ACK envelope:

  ```text
  BHS|^~\&|PAS|FAC|<original_sender>|FAC|<now>||ACK-<original_batch_id>|P|2.5
  MSH|^~\&|PAS|FAC|...|ACK|MSG-1|P|2.5
  MSA|AA|MSG-1
  MSH|^~\&|PAS|FAC|...|ACK|MSG-2|P|2.5
  MSA|AE|MSG-2|<diagnostic>
  ...
  BTS|<n>
  ```

Outbound publisher behavior is **unchanged**: the PAS still emits one
ADT per state change. Bulk batches are an inbound convenience, not an
outbound flush. (A batched outbound flush would be a separate feature.)

Sample payload: [`examples/hl7-batch-adt.txt`](../examples/hl7-batch-adt.txt).

### Outbound mapping (v0.3 + v0.4)

When `HL7V2_OUTBOUND_PEER` is set, the publisher relays:

| Domain event                      | Outbound message |
|-----------------------------------|------------------|
| `EncounterAdmitted`               | `ADT^A01`        |
| `EncounterTransferred`            | `ADT^A02`        |
| `EncounterDischarged`             | `ADT^A03`        |
| `EncounterRegistered` (no `source` tag, OR source ≠ `hl7v2_a04`) | `ADT^A04` (PV1-2 = `O`/`E` from class) |
| `EncounterPreAdmitted` (no `source` tag, OR source ≠ `hl7v2_a05`) | `ADT^A05` (PV1-3 = reserved bed code) |
| `EncounterPromotedToInpatient` (no `source` tag, OR source ≠ `hl7v2_a06`) | `ADT^A06` (PV1-3 = bed code) |
| `EncounterPreAdmitCancelled` (no `source` tag, OR source ≠ `hl7v2_a38`) | `ADT^A38` (PV1-3 = released bed code) |
| `PatientUpdated` (`source = hl7v2_a08`)   | `ADT^A08`        |
| `EncounterCancelled` (no `reason` tag, OR reason ≠ `hl7v2_a11`) | `ADT^A11` (v0.38; previously emitted only on the HL7 path, now source-gated against echo like the rest) |
| `EncounterTransferCancelled` (no `source` tag, OR source ≠ `hl7v2_a12`) | `ADT^A12` (PV1-3 = origin bed code) |
| `EncounterLeaveStarted` (no `source` tag, OR source ≠ `hl7v2_a21`) | `ADT^A21` (PID-only; no PV1) |
| `EncounterLeaveEnded` (no `source` tag, OR source ≠ `hl7v2_a22`) | `ADT^A22` (PID-only; no PV1) |
| `PatientDeleted` (no `source` tag, OR source ≠ `hl7v2_a23`) | `ADT^A23` (PID-only; no PV1) |
| `EncounterDischargeCancelled`     | `ADT^A13`        |
| `PatientMerged` (no `source` tag, OR source ≠ `hl7v2_a40`)            | `ADT^A40`        |
| `ChargePosted` (no `source` tag, OR source ≠ `hl7v2_p03`)             | `DFT^P03`        |
| `PractitionerCreated` (no `source` tag, OR source ≠ `hl7v2_mfn_m02`)  | `MFN^M02` MAD    |
| `PractitionerUpdated` (no `source` tag, OR source ≠ `hl7v2_mfn_m02`)  | `MFN^M02` MUP    |
| `PractitionerDeactivated` (no `source` tag, OR source ≠ `hl7v2_mfn_m02`) | `MFN^M02` MDL |
| `BedCreated` (no `source` tag, OR source ≠ `hl7v2_mfn_m05`)          | `MFN^M05` MAD    |
| `BedUpdated` (no `source` tag, OR source ≠ `hl7v2_mfn_m05`)          | `MFN^M05` MUP    |
| `BedRetired` (no `source` tag, OR source ≠ `hl7v2_mfn_m05`)          | `MFN^M05` MDL    |
| `AppointmentBooked` (no `source` tag, OR source ≠ `hl7v2_s12`)        | `SIU^S12`        |
| `AppointmentRescheduled` (no `source` tag, OR source ≠ `hl7v2_s13`)   | `SIU^S13`        |
| `AppointmentModified` (no `source` tag, OR source ≠ `hl7v2_s14`)      | `SIU^S14`        |
| `AppointmentCancelled` (no `source` tag, OR source ≠ `hl7v2_s15`)     | `SIU^S15`        |

`PatientUpdated` and `EncounterCancelled` are **source-gated**:
REST-driven patient edits and REST-driven encounter cancellations do
**not** echo to the EMR, because the inbound source is typically the
same EMR and we want to avoid boomeranging the change. To explicitly
emit an ADT^A08 from a REST flow, write the source tag onto the
outbox payload yourself (or add a new event variant for "human
correction at PAS").

**SIU (v0.16 + v0.17) gates the other way around.** `AppointmentBooked`
/ `Rescheduled` / `Modified` / `Cancelled` only **skip** when their
payload's `source` is `hl7v2_s12` / `s13` / `s14` / `s15` respectively
— i.e. PAS does relay REST- and FHIR-driven appointments to the EMR
by default, and only refuses to echo back appointments it just
received from that same EMR. The filler appointment id (PAS UUID)
travels in SCH-2 in both directions so the EMR can dedupe on it.

## Limitations & gotchas

- The exporter loads everything into memory before flushing. For 10k+ patient tenants, prefer the paged FHIR Bundle.
- TSV is not Unicode-quoted. If a demographic field ever contains a literal tab, the row will be split incorrectly. Guard at the data-entry layer.
- XML's `<active>` is `true`/`false` lowercase, not `True`/`False`. The deserializer is case-sensitive on booleans.
- The `id` field on import accepts an empty string — a fresh UUID is allocated. Use this to bulk-load new patients without picking UUIDs ahead of time.
- `gender` values outside the four enum tokens silently round to `unknown`.
