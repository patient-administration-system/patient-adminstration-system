# Changelog

Workspace-level release history. Each entry coordinates the state of
the member crates at a point in time. For per-crate detail, see:

- [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md)
- [`patient-administration-system-frontend/CHANGELOG.md`](patient-administration-system-frontend/CHANGELOG.md)

## 2026-05-30 — PAS Axum v0.43.0: HL7 v2 ADT^A07 (Change Inpatient to Outpatient)

- **PAS Axum v0.43.0** — symmetric pair to v0.41 A06. Adds
  inbound `ADT^A07` for the demotion path: an existing
  Inpatient encounter is converted to Outpatient with the bed
  released. Common real-world flow: an admitted patient is
  reclassified to observation status without a full discharge.
- `POST /api/hl7/v2/change-to-outpatient` looks up the
  patient's currently-open admission by PID-3.1 MRN, releases
  the bed assignment (bed → Cleaning, same as discharge),
  reclassifies the encounter from Inpatient to Outpatient,
  leaves the encounter status as InProgress (still active —
  just no longer admitted to a bed). The admissions row is
  preserved as historical record.
- `AdtService::change_to_outpatient(admission_id, source,
  ctx)` is the underlying service method.
- Outbox event `EncounterDemotedToOutpatient` tagged
  `source: "hl7v2_a07"`. MLLP routing arm + 1 lib unit test.
  Lib total 496 → **497 passing**; workspace lib
  **506 passing**.
- 1 new DB-bound integration test covering: A01 admit → A07
  demote → assert encounter Outpatient+InProgress + bed
  Cleaning; second A07 → 400 (no remaining open admission);
  wrong message type → 400. Integration: 61 → **62** functions.

## 2026-05-30 — PAS Axum v0.42.0: outbound ADT^A06 + REST change-to-inpatient

- **PAS Axum v0.42.0** — closes the v0.41 inbound A06 loop.
  New REST endpoint `POST /api/admissions/change-to-inpatient`
  invokes `change_to_inpatient` without a source tag,
  emitting `EncounterPromotedToInpatient`; the HL7 v2
  publisher fans that out as `ADT^A06` with PV1-3 carrying
  the allocated bed code (same shape as A01).
- Outbound is source-gated on `hl7v2_a06` — events that
  came in via the v0.41 inbound A06 path are silently
  dropped on the outbound side.
- New `encode_adt_a06` wire-encoder + 1 lib unit test. Lib
  total 495 → **496 passing**; workspace lib **505 passing**.

## 2026-05-30 — PAS Axum v0.41.0: HL7 v2 ADT^A06 (Change Outpatient to Inpatient)

- **PAS Axum v0.41.0** — adds inbound `ADT^A06`, the trigger
  for promoting an existing ambulatory (Outpatient /
  Emergency) encounter to a full inpatient admission with a
  bed. Common real-world flow: an ED patient (registered via
  v0.28 A04) gets admitted to a ward bed.
- `POST /api/hl7/v2/change-to-inpatient` looks up the patient
  by PID-3.1, finds their most-recent active ambulatory
  encounter (Outpatient or Emergency in Arrived or InProgress
  state), allocates the bed from PV1-3.3 (must be Available),
  reclassifies the encounter as Inpatient, advances to
  InProgress if still Arrived, creates `Admission` +
  `BedAssignment`. All in one DB transaction.
- `AdtService::change_to_inpatient(patient_id, bed_id,
  source, ctx)` is the underlying service method, signature
  follows the v0.31+ source-tag pattern.
- New repo helpers:
  `EncounterRepository::set_class` (administrative class
  change — no state-machine constraint),
  `EncounterRepository::find_latest_active_ambulatory_for_patient`.
- Outbox event `EncounterPromotedToInpatient` tagged
  `source: "hl7v2_a06"`. MLLP routing arm + 1 lib unit test.
  Lib total 494 → **495 passing**; workspace lib **504
  passing**.
- 1 new DB-bound integration test covering: register via
  A04 → A06 promotes (encounter Inpatient + InProgress; bed
  Occupied); second A06 → 404 (no more active ambulatory);
  unknown patient → 404; wrong message type → 400.
  Integration: 60 → **61** functions.

## 2026-05-30 — PAS Axum v0.40.0: outbound ADT^A23 + REST delete-patient retrofit

- **PAS Axum v0.40.0** — closes the v0.39 inbound A23 loop and
  brings the existing REST `DELETE /api/patients/{id}` handler
  into line with the same safety + outbox semantics. The
  handler now:
  - rejects with 409 if the patient has any open admission
    (same constraint as the HL7 A23 path);
  - wraps the soft-delete + audit + `PatientDeleted` outbox
    write in a single DB transaction;
  - drops from Tantivy as a post-commit best-effort step.
- The HL7 v2 publisher fans out `PatientDeleted` events as
  `ADT^A23` (PID-only, no PV1). Source-gated on `hl7v2_a23`
  so v0.39 inbound events don't echo back.
- New `encode_adt_a23` wire-encoder + 1 lib unit test. Lib
  total 493 → **494 passing**; workspace lib **503 passing**.

## 2026-05-30 — PAS Axum v0.39.0: HL7 v2 ADT^A23 (Delete a Patient Record)

- **PAS Axum v0.39.0** — adds inbound `ADT^A23` for
  "this patient record was created in error" workflows.
  `POST /api/hl7/v2/delete-patient` looks up the patient by
  PID-3.1 (MRN), refuses (409 + AE) if the patient has any
  open admission, otherwise soft-deletes the patient row
  (`deleted_at` set), drops them from the Tantivy search
  index, and writes a `PatientDeleted` outbox event tagged
  `source: "hl7v2_a23"`.
- The safety check (no-open-admission) is the key
  distinguishing constraint vs the existing REST
  `DELETE /api/patients/{id}` — A23 should never be used
  to delete a real admitted patient, only to undo a record
  that was sent in error.
- MLLP listener routes `ADT^A23` to the same path. 1 new lib
  test for the routing. Lib total 492 → **493 passing**;
  workspace lib **502 passing**. 1 new DB-bound integration
  test covering: register a fresh patient → A23 soft-deletes;
  unknown MRN → 404; admitted patient → 409; wrong message
  type → 400. Integration: 59 → **60** functions.

## 2026-05-30 — PAS Axum v0.38.0: outbound ADT^A11 retrofit + REST cancel-admit

- **PAS Axum v0.38.0** — fixes the long-standing backwards
  source-gating on outbound `ADT^A11` (set in v0.4 before the
  boomerang-protection pattern was established in v0.25). The
  outbound publisher now correctly **suppresses** events that
  came in via the HL7 v2 inbound path (`reason == "hl7v2_a11"`)
  and **emits** events from REST / other sources — same shape
  as A04 / A05 / A12 / A21 / A22 / A38 outbound matchers.
- New REST endpoint `POST /api/admissions/{id}/cancel-admit`
  invokes `cancel_admission` without a `reason` tag; the
  publisher fans that out as `ADT^A11` (PID-only, no PV1) so
  the previously-orphan service method is now reachable from
  the REST surface and round-trips to downstream peers.
- `AdtService::cancel_admission` signature changed from
  `(admission_id, ctx)` to `(admission_id, reason, ctx)` where
  `reason` is `Option<&str>`. The HL7 v2 handler passes
  `Some("hl7v2_a11")`; the new REST handler passes `None`. The
  payload now writes the `reason` field only when present.
- Drive-by integration test housekeeping: A11 / A13 / A01
  tests reference the WardOccupancy field as `occupied`
  (not the imaginary `occupied_beds`); A01 facility / ward /
  room codes now carry a random suffix so the test is
  re-runnable against a stale DB.

## 2026-05-30 — PAS Axum v0.37.0: outbound ADT^A21 + A22 + REST leave endpoints

- **PAS Axum v0.37.0** — closes the v0.36 inbound LOA loop.
  Two new REST endpoints
  (`POST /api/admissions/{id}/leave-start` and `/leave-end`)
  invoke `start_leave` / `end_leave` without a source tag;
  the HL7 v2 publisher fans out the matching outbox events
  as `ADT^A21` / `ADT^A22` (MSH + EVN + PID; no PV1 — same
  shape as A03 discharge, the receiver locates the open
  admission by patient identity).
- Outbound is source-gated on `hl7v2_a21` / `hl7v2_a22` —
  events that came in via the v0.36 inbound paths are
  silently dropped on the outbound side. Same boomerang-
  protection pattern as the rest of the inbound/outbound
  pairs.
- New `encode_adt_a21` + `encode_adt_a22` wire-encoders;
  new private `Hl7v2MllpPublisher::emit_loa` helper that
  resolves admission → encounter → patient. 2 new lib unit
  tests for the encoders. Lib total 490 → **492 passing**;
  workspace lib **501 passing**.

## 2026-05-30 — PAS Axum v0.36.0: HL7 v2 ADT^A21 + A22 (Leave of Absence)

- **PAS Axum v0.36.0** — adds the leave-of-absence round-trip.
  `ADT^A21` (`POST /api/hl7/v2/leave-start`) transitions the
  patient's open encounter `InProgress → OnLeave`; `ADT^A22`
  (`POST /api/hl7/v2/leave-end`) transitions it back
  `OnLeave → InProgress`. The bed remains `Occupied` for the
  duration — the patient is expected back.
- Both transitions go through the regular
  `EncounterStatus::try_transition_to` state machine, so
  invalid moves (a second A21 with the encounter already
  `OnLeave`, etc.) come back as 409 + AE with diagnostic.
- Two new `AdtService` methods (`start_leave`, `end_leave`)
  share a private `transition_loa` helper that handles the
  txn shape + audit + outbox. Outbox events
  `EncounterLeaveStarted` (source `hl7v2_a21`) and
  `EncounterLeaveEnded` (source `hl7v2_a22`).
- 2 new lib unit tests (MLLP routing arms for A21 + A22). Lib
  total 488 → **490 passing**; workspace lib **499 passing**.
  1 new DB-bound integration test covering the full
  admit → A21 → A22 round-trip, second A21 → 409, second A22
  → 409, wrong message type → 400. Integration: 58 → **59**
  functions.

## 2026-05-30 — PAS Axum v0.35.0: outbound ADT^A38 + REST cancel-pre-admit

- **PAS Axum v0.35.0** — closes the v0.34 inbound ADT^A38 loop.
  New REST endpoint `POST /api/admissions/cancel-pre-admit`
  invokes `AdtService::cancel_pre_admit` without a source tag,
  emitting `EncounterPreAdmitCancelled`; the HL7 v2 publisher
  fans that out as `ADT^A38` with PV1-3 carrying the released
  bed code.
- Outbound is source-gated on `hl7v2_a38` — events that came
  in via the v0.34 inbound A38 path are silently dropped on
  the outbound side. Same boomerang-protection pattern as
  v0.25, v0.27, v0.29, v0.31, v0.33.
- New `encode_adt_a38` wire-encoder + 1 lib unit test for the
  encoder → lib total 487 → **488 passing**; workspace lib
  **497 passing**.

## 2026-05-30 — PAS Axum v0.34.0: HL7 v2 ADT^A38 (Cancel Pre-admit)

- **PAS Axum v0.34.0** — closes the pre-admit lifecycle that
  v0.32 / v0.33 opened. `POST /api/hl7/v2/cancel-pre-admit`
  ingests `ADT^A38`: looks up the patient by PID-3.1 (must
  already exist — no dedup-or-create on cancel), the bed by
  PV1-3.3 (must be `Reserved`), finds the patient's most-
  recent Planned inpatient encounter, releases the bed
  (`Reserved → Available`) and cancels the encounter
  (`Planned → Cancelled`). All in one DB transaction.
- `AdtService::cancel_pre_admit(patient_id, bed_id, source,
  ctx)` is the underlying service method, signature mirrors
  `cancel_transfer` / `pre_admit` — `source` is `Option<&str>`.
- New repo helper
  `EncounterRepository::find_latest_planned_inpatient_for_patient`.
- Outbox event `EncounterPreAdmitCancelled` tagged
  `source: "hl7v2_a38"`. MLLP routing arm + 1 new lib unit
  test for the routing. Lib total 486 → **487 passing**;
  workspace lib **496 passing**.
- 1 new DB-bound integration test covering A05 → A38 happy
  path; re-cancel when bed is no longer Reserved → 409;
  unknown patient MRN → 404; wrong message type → 400.
  Integration: 57 → **58** functions.

## 2026-05-30 — PAS Axum v0.33.0: outbound ADT^A05 + REST pre-admit

- **PAS Axum v0.33.0** — closes the v0.32 inbound ADT^A05 loop.
  New REST endpoint `POST /api/admissions/pre-admit` invokes
  `AdtService::pre_admit` without a source tag, emitting
  `EncounterPreAdmitted`; the HL7 v2 publisher fans that out
  as `ADT^A05` with PV1-3 carrying the reserved bed code.
- Outbound is source-gated on `hl7v2_a05` — events that came
  in via the v0.32 inbound A05 path are silently dropped on
  the outbound side. Same boomerang-protection pattern as
  v0.25, v0.27, v0.29, and v0.31.
- `AdtService::pre_admit` signature changed from
  `(patient_id, bed_id, ctx)` to `(patient_id, bed_id, source,
  ctx)` where `source` is `Option<&str>`. The HL7 v2 handler
  passes `Some("hl7v2_a05")`; the new REST handler passes
  `None`. The reason tag is now embedded in the outbox payload
  only when present (was previously hardcoded).
- New `encode_adt_a05` wire-encoder + 1 lib unit test for the
  encoder → lib total 485 → **486 passing**; workspace lib
  **495 passing**.

## 2026-05-30 — PAS Axum v0.32.0: HL7 v2 ADT^A05 (Pre-admit Patient)

- **PAS Axum v0.32.0** — adds inbound `ADT^A05` (pre-admit
  patient) on the same HTTP + MLLP surface as the rest of the
  ADT family. Dedups (or creates) the patient from `PID`,
  looks up the destination bed by `PV1-3.3`, locks it (must be
  `Available`), flips it to `Reserved` via the regular state-
  machine transition, and opens an inpatient `Encounter` in
  `Planned` status — all in one DB transaction. No `Admission`
  row or `BedAssignment` is created — the patient hasn't
  physically arrived yet.
- `POST /api/hl7/v2/pre-admit` is the HTTP entry-point; MLLP
  listener routes `ADT^A05` to the same path.
- Each successful A05 writes an `audit_log` row
  (`action = "pre_admit"`) plus an `EncounterPreAdmitted`
  outbox event tagged `source: "hl7v2_a05"`.
- 1 new lib test (MLLP routing arm) + 1 new DB-bound
  integration test covering the happy path + 409 on already-
  reserved bed + 404 on unknown bed + 400 on wrong message
  type. Lib total 484 → **485 passing**; workspace lib
  **494 passing**. Integration: 56 → **57** functions.

## 2026-05-30 — PAS Axum v0.31.0: outbound ADT^A12 + REST cancel-transfer

- **PAS Axum v0.31.0** — closes the v0.30 inbound ADT^A12 loop.
  New REST endpoint `POST /api/admissions/{id}/cancel-transfer`
  invokes `AdtService::cancel_transfer` without a source tag,
  emitting `EncounterTransferCancelled`; the HL7 v2 publisher
  fans that out as `ADT^A12` with PV1-3 carrying the origin bed
  code (PL.3 sub-component) so the receiver knows where to put
  the patient back.
- Outbound is source-gated on `hl7v2_a12` — events that came
  in via the v0.30 inbound A12 path are silently dropped on
  the outbound side. Same boomerang-protection pattern as
  v0.25, v0.27, and v0.29.
- `AdtService::cancel_transfer` signature changed from
  `(admission_id, ctx)` to `(admission_id, source, ctx)` where
  `source` is `Option<&str>`. The HL7 v2 handler passes
  `Some("hl7v2_a12")`; the new REST handler passes `None`.
  The reason tag is now embedded in the outbox payload only
  when present (was previously hardcoded).
- New `encode_adt_a12` wire-encoder + 1 lib unit test for the
  encoder + the existing v0.30 routing test → lib total
  483 → **484 passing**; workspace lib **493 passing**.

## 2026-05-29 — PAS Axum v0.30.0: HL7 v2 ADT^A12 (Cancel Transfer)

- **PAS Axum v0.30.0** — completes the A11/A12/A13 cancel trio.
  `POST /api/hl7/v2/cancel-transfer` reverses the most-recent
  bed transfer for the patient identified by PID-3.1 (MRN):
  releases the destination bed assignment, restores the patient
  to the origin bed, flips destination → `Cleaning` and origin
  → `Occupied`, deletes the cancelled `transfers` row, writes
  the matching audit + outbox events. All in one DB transaction.
- `AdtService::cancel_transfer(admission_id, ctx)` is the
  underlying service method, mirroring the shape of
  `cancel_admission` (A11) and `cancel_discharge` (A13).
- Two new repo helpers in `AdmissionRepository`:
  `find_latest_transfer_for_admission` and `delete_transfer`.
  The `transfers` table is treated as operational (same as
  `discharges`) — cancelled rows are physically removed.
- Origin-bed must be `Available` or `Cleaning` (anything else
  → 409 + AE) — same safety constraint as A13.
- Outbox event `EncounterTransferCancelled` with
  `reason: "hl7v2_a12"`. MLLP routing arm + 1 new lib unit
  test for the routing. Lib total 482 → **483 passing**;
  workspace lib **492 passing**.
- 1 new DB-bound integration test covering the round-trip
  (admit → transfer → cancel-transfer; re-cancel → 404; wrong
  message type → 400). Integration: 55 → **56** functions.

## 2026-05-29 — PAS Axum v0.29.0: outbound ADT^A04 + encounter outbox

- **PAS Axum v0.29.0** — closes the v0.28 inbound A04 loop.
  REST-driven encounter creation for **Outpatient** or
  **Emergency** classes now fires an `EncounterRegistered`
  outbox event; the HL7 v2 publisher fans those out as
  `ADT^A04` with PV1-2 matching the class (`O` / `E`).
  Inpatient encounters continue to use the v0.4 admit flow
  and its `EncounterAdmitted → ADT^A01` mapping.
- Outbound is source-gated on `hl7v2_a04` — events that came
  in via the v0.28 inbound A04 path are silently dropped on
  the outbound side. Same boomerang-protection pattern as
  v0.25 (MFN^M02) and v0.27 (MFN^M05).
- `create_encounter` REST handler now wraps the insert in a
  transaction with audit + outbox writes (it was previously
  audit-only with the audit row written non-transactionally
  after the insert).
- New `encode_adt_a04` wire-encoder helper; 2 new lib unit
  tests cover the encoder. Lib total 480 → **482 passing**;
  workspace lib total **491 passing**.

## 2026-05-29 — PAS Axum v0.28.0: HL7 v2 ADT^A04 (Register Outpatient)

- **PAS Axum v0.28.0** — adds inbound `ADT^A04` (Register
  Outpatient / Emergency) on the same HTTP + MLLP surface as the
  rest of the ADT family. Dedups (or creates) the patient from
  `PID`, then opens an `Encounter` in `Arrived` status with
  `class` derived from `PV1-2` (`E` → `Emergency`, anything else
  → `Outpatient`). No bed allocation — A04 is the orthogonal
  ambulatory complement to A01's bed-allocating admit path.
- `POST /api/hl7/v2/register` is the HTTP entry-point; MLLP
  listener routes `ADT^A04` to the same path.
- Each successful A04 writes an `audit_log` row
  (`action = "register_via_hl7v2_a04"`) plus an
  `EncounterRegistered` outbox event tagged
  `source: "hl7v2_a04"`. Multiple visits with the same MRN dedup
  the patient row but always create a fresh encounter.
- 1 new lib test (MLLP routing arm) + 1 new DB-bound integration
  test covering outpatient + emergency class derivation, repeat
  visit dedup, missing PV1 → 400, wrong message type → 400. Lib
  total 479 → 480.

## 2026-05-29 — PAS Axum v0.27.0: outbound MFN^M05 + bed audit/outbox

- **PAS Axum v0.27.0** — closes the v0.26 inbound MFN^M05 loop.
  PAS-side bed roster changes (REST `create_bed`, new `update_bed`,
  status flips to `OutOfService`) now fire `BedCreated` /
  `BedUpdated` / `BedRetired` outbox events, and the HL7 v2
  publisher fans those out as `MFN^M05` MAD / MUP / MDL.
- Outbound is source-gated on `hl7v2_mfn_m05` — events that came
  in via the v0.26 inbound MFN path are silently dropped on the
  outbound side, same boomerang-protection pattern as v0.25.
- LOC-1.1 = the bed's parent-room code, LOC-1.3 = bed code,
  LOC-2 = bed display name — the same shape the v0.26 inbound
  parser expects, so the contract round-trips.
- REST `create_bed` now wraps its insert in a transaction that
  writes an `audit_log` `create` row plus a `BedCreated` outbox
  event. New `PUT /api/beds/{id}` (`update_bed`) handler at the
  REST surface mirrors v0.25's `update_practitioner` — selective
  field update of `room_id` / `name` / `code` with the same
  audit + outbox semantics. `set_bed_status` additionally emits
  `BedRetired` when the new status is `OutOfService` (alongside
  the pre-existing `BedStatusChanged`).
- No new lib tests, no new integration tests — the v0.26 inbound
  test already exercises the source-gating contract through the
  `source: "hl7v2_mfn_m05"` tag, same pattern as v0.25. Lib total
  remains **488 passing**.

## 2026-05-28 — PAS Axum v0.26.0: MFN^M05 (Master File — Patient Location)

- **PAS Axum v0.26.0** — bed roster sync release. `POST /api/hl7/v2/
  mfn-location` ingests MFN^M05 messages where each MFE+LOC pair
  drives a bed `MAD` / `MUP` / `MDL`. LOC-1 is `PL`-typed —
  LOC-1.1 = parent room code, LOC-1.3 = bed code; LOC-2 = name.
- `MAD` resolves the parent room by code and inserts; `MUP` does
  a full-replace via the new `BedRepository::update`; `MDL` flips
  the bed to `OutOfService` via the existing `set_status_unchecked`
  (operator-authorised bypass, same pattern as v0.4 ADT^A13).
- Atomic per message — same contract as v0.20 multi-FT1 DFT and
  v0.24 multi-MFE MFN^M02.
- New outbox event types `BedCreated`, `BedUpdated`, `BedRetired`,
  all tagged `source: "hl7v2_mfn_m05"`.
- 8 new lib unit tests + 1 new DB-bound integration test. Lib
  total: 471 → 479. Workspace lib: **488 passing**.

## 2026-05-28 — PAS Axum v0.25.0: outbound MFN^M02 + practitioner audit/outbox

- **PAS Axum v0.25.0** — closes the v0.24 inbound MFN^M02 loop.
  PAS-side practitioner roster changes (REST + FHIR POST / PUT /
  DELETE) now fire `PractitionerCreated` / `PractitionerUpdated`
  / `PractitionerDeactivated` outbox events, and the HL7 v2
  publisher fans those out as `MFN^M02` MAD / MUP / MDL.
- Outbound is source-gated on `hl7v2_mfn_m02` (events that came
  in via the v0.24 inbound MFN path are silently dropped on the
  outbound side — no boomerang).
- The outbound MFE-4 / STF-1 primary key reuses the EMR's staff
  id when the row carries an `urn:hl7v2:staff:id` identifier;
  otherwise the PAS UUID is used.
- REST `create_practitioner` / `update_practitioner` /
  `delete_practitioner` now also write **audit_log** entries
  (action = `create` / `update` / `deactivate`). They previously
  bypassed the audit trail entirely — fixed as part of this loop
  closure.
- No new lib tests in this release; the v0.24 inbound MFN
  integration test already exercises both directions of the
  contract through the `source: "hl7v2_mfn_m02"` tag. Workspace
  lib unchanged at **480 passing**.

## 2026-05-28 — PAS Axum v0.24.0: HL7 v2 MFN^M02 (Master File — Staff)

- **PAS Axum v0.24.0** — provider-directory sync release. `POST
  /api/hl7/v2/mfn-staff` ingests MFN^M02 messages with one or
  more MFE+STF pairs. Each pair maps to:
  - `MAD` → create a new practitioner.
  - `MUP` → replace name/gender/birth-date/active on an existing
    practitioner identified by the EMR's staff id.
  - `MDL` → soft-delete via `active = false` (practitioner rows
    are referenced by encounter / appointment / schedule and
    hard delete would orphan them).
- All items in one message are applied in a single DB transaction
  (atomic per message — same contract as v0.20's multi-FT1 DFT).
- New `PractitionerRepository` (additive — the existing inline
  ActiveModel REST handlers are unchanged) with a `find_by_
  identifier_value` mirror of the patient-side query.
- New outbox events: `PractitionerCreated`, `PractitionerUpdated`,
  `PractitionerDeactivated`, all tagged `source: "hl7v2_mfn_m02"`.
- 9 new lib unit tests + 1 new DB-bound integration test. Lib
  total: 460 → 471. Workspace lib: **480 passing**. Integration:
  52 → 53.

## 2026-05-28 — PAS Axum v0.23.0: FHIR Bundle entries for Practitioner / Schedule / Slot

- **PAS Axum v0.23.0** — Bundle-write extension. `POST /fhir`
  Bundle entries now accept `Practitioner`, `Schedule`, and
  `Slot` resources in addition to the existing Patient /
  Encounter / Appointment / Coverage types.
- A single transaction Bundle can now provision a new
  practitioner, their schedule, and slots in one atomic write —
  useful for new-clinic onboarding.
- Transaction semantics unchanged: any failing entry rolls the
  whole bundle back, with the `OperationOutcome` diagnostic
  naming the offending entry index.
- 1 new DB-bound integration test exercises the chained
  Practitioner → Schedule → Slot creation path and the atomicity
  guarantee. Workspace lib unchanged at **469 passing**.
  Integration: 51 → 52.

## 2026-05-27 — PAS Axum v0.22.0: HL7 v2 PID-29/30 + outbound test fix

- **PAS Axum v0.22.0** — wire + cleanup release. Two related
  touches to the HL7 v2 path.
- **Fixes** the pre-existing `tests/hl7v2_outbound_test.rs`
  regression: `CreateFacilityRequest.address` and
  `CreateWardRequest.capacity` are now both optional (the
  request bodies the tests had been sending haven't carried
  those fields), and the test's `"gender": "Female"` /
  `"Other"` cases were corrected to lowercase (matching the
  serde rename). Both outbound publisher tests now pass.
- **Adds** PID-29 (Patient Death Date and Time) and PID-30
  (Patient Death Indicator) round-trip to the PID encoder +
  decoder. `patient_from_pid` reads both; when PID-30 is absent
  but PID-29 is set, deceased is inferred. `pid_from_patient`
  emits PID-29 + PID-30 for deceased patients and stays compact
  (14 fields, no change to the existing wire shape) for living
  patients.
- 6 new lib unit tests on the PID round-trip. Lib total:
  454 → 460. Workspace lib: **469 passing**.

## 2026-05-26 — PAS Axum v0.21.0: FHIR Practitioner/Schedule/Slot write

- **PAS Axum v0.21.0** — FHIR-write release. `Practitioner`,
  `Schedule`, and `Slot` were read-only at `GET /fhir/<Type>/{id}`;
  v0.21 adds the matching `POST` / `PUT` / `DELETE` so FHIR
  clients can fully manage those resources without dropping to the
  `/api/*` REST surface.
- `DELETE /fhir/Practitioner/{id}` flips `active = false` (soft-
  delete via the FHIR `active` flag — practitioner rows are
  referenced by encounter / appointment / schedule and a hard
  delete would orphan those join paths).
- `DELETE /fhir/Schedule/{id}` and `DELETE /fhir/Slot/{id}` are
  hard deletes (invariant §5.3 restricts soft-delete to patients
  / encounters / appointments).
- New repo methods: `ScheduleRepository::{update, delete}` and
  `SlotRepository::{update, delete}`.
- **Fixed**: `PUT /fhir/Patient/{id}` no longer rejects a non-UUID
  client-supplied body id. Per FHIR semantics the URL id is
  canonical; v0.21 strips `body.id` before parsing on all four
  PUT paths (Patient, Practitioner, Schedule, Slot). Unblocks the
  pre-existing `tests/fhir_write_test.rs::fhir_patient_crud` test.
- 2 new DB-bound integration tests (Practitioner CRUD + chained
  Schedule + Slot CRUD). Workspace lib unchanged at **463 passing**.
  Integration: 49 → 51.

## 2026-05-26 — PAS Axum v0.20.0: multi-FT1 DFT^P03 (atomic batch billing)

- **PAS Axum v0.20.0** — billing-interop release. A single
  `POST /api/hl7/v2/dft` message can now carry many FT1 segments;
  PAS posts all charges in one DB transaction. If any FT1 fails
  parsing or the transaction fails, nothing lands — partial
  visits never half-bill.
- All FT1 in one message must share the same FT1-11.2 currency.
  Mixed currencies → 400 + AE (avoids ambiguous account-split
  semantics).
- MSA-3 diag stays `charge=<uuid> account=<uuid>` for single-FT1
  (backwards-compatible with v0.19 senders), switches to
  `charges_posted=<N> account=<uuid>` for multi-FT1.
- Per-FT1 error messages now name the 1-based segment index
  (`FT1[2]-11.1 ...`) so senders with one bad row in a long batch
  can locate it without trial and error.
- 2 new lib unit tests + 1 new DB-bound integration test (with an
  explicit atomicity assertion — a bad second FT1 must not leave
  the first one persisted). Lib total: 452 → 454. Workspace lib:
  **463 passing**. Integration: 48 → 49.

## 2026-05-26 — PAS Axum v0.19.0: HL7 v2 DFT^P03 + billing schema fix

- **PAS Axum v0.19.0** — billing-interop release. New `POST /api/hl7/
  v2/dft` ingests `DFT^P03` (post detail financial transaction)
  charges. The handler parses PID + FT1, dedup-or-creates the
  patient, auto-creates an open billing account in the FT1-11.2
  currency when none exists, and posts the charge with audit + outbox.
  AA's MSA-3 reports the assigned `charge=<uuid> account=<uuid>`.
- Outbound: `ChargePosted → DFT^P03` with source-gate on
  `hl7v2_p03`. New `BillingRepository::find_charge_by_id` +
  `find_account_by_id` enable the publisher to resolve
  `charge → account → patient`.
- Only `FT1-6 = CG` (charge) is accepted; `PY` (payment) and `AJ`
  (adjustment) AE-ACK as unsupported.
- **Schema fix**: `charges.amount_value`, `payments.amount_value`,
  and `invoices.total_value` columns are now `text` (matching the
  SeaORM entities). The previous `decimal(20,4)` declaration
  silently failed at runtime for every write because Postgres
  rejects `text → numeric` implicit conversion. No data migration
  is needed — the bug prevented any rows from being written.
- 11 new lib unit tests + 1 new DB-bound integration test. Lib
  total: 441 → 452. Workspace lib: **461 passing**. Integration:
  47 → 48. The pre-existing `tests/billing_flow_test.rs` now
  passes too.

## 2026-05-26 — PAS Axum v0.18.0: HL7 v2 ADT^A40 (merge patient)

- **PAS Axum v0.18.0** — identity-interop release. New ADT^A40 (merge
  patient — patient ID) surface on both sides of the HL7 v2 wire.
- Inbound: `POST /api/hl7/v2/merge` parses PID (survivor) and MRG-1.1
  (source MRN), looks up the source by MRN, and applies the same
  merge logic as `POST /api/patients/{id}/merge-into/{target_id}`
  (set `replaced_by`, audit, outbox, drop from Tantivy). MLLP
  listener auto-routes `ADT^A40` to it.
- Outbound: `PatientMerged → ADT^A40`. Skips relaying when payload
  `source == "hl7v2_a40"` so we don't echo a merge back to the EMR
  that just sent it. REST-driven merges (no source tag) relay
  normally.
- Bootstrap-friendly: an unknown survivor PID gets created on the
  fly (dedup-on-MRN like A01/A28); the source MRN must already
  exist in PAS.
- 6 new lib unit tests + 1 new DB-bound integration test. Lib
  total: 435 → 441. Workspace lib: **450 passing**. Integration:
  46 → 47.

## 2026-05-25 — PAS Axum v0.17.0: SIU lifecycle completion (S13 + S14)

- **PAS Axum v0.17.0** — closes the v0.16 SIU loop with inbound +
  outbound SIU^S13 (reschedule) and SIU^S14 (modify). PAS now speaks
  the full book → reschedule → modify → cancel lifecycle on both
  sides of the HL7 v2 wire.
- Inbound: `POST /api/hl7/v2/schedule-reschedule` and `POST /api/hl7/
  v2/schedule-modify`. MLLP listener auto-routes `SIU^S13` and
  `SIU^S14`. Reschedule runs an **overlap-excluding** check so a
  row's own time window doesn't flag itself. Modify rewrites only
  `Appointment.reason`; time changes go through S13.
- Outbound: `AppointmentRescheduled → SIU^S13`, `AppointmentModified
  → SIU^S14`. Both source-gated on `hl7v2_s13` / `hl7v2_s14`.
- New helper `AppointmentRepository::find_overlapping_for_patient_
  excluding(_, _, _, _, exclude_id)` so reschedule overlap checks can
  ignore the row being rescheduled.
- 9 new lib unit tests + 1 new DB-bound integration test. Lib
  total: 426 → 435. Workspace lib: **444 passing**. Integration:
  45 → 46.

## 2026-05-25 — PAS Axum v0.16.0: HL7 v2 SIU scheduling messages

- **PAS Axum v0.16.0** — scheduling-interop release. Completes the HL7
  v2 lifecycle alongside the existing ADT coverage: inbound + outbound
  SIU^S12 (notification of new appointment) and SIU^S15 (notification
  of cancellation).
- Inbound: `POST /api/hl7/v2/schedule-book` and `POST /api/hl7/v2/
  schedule-cancel`. The MLLP listener auto-routes `SIU^S12` and
  `SIU^S15` payloads to the new handlers; unknown SIU triggers
  (S13/S14/S17/S26) fall through to `/patient` and AE-ACK as
  unsupported.
- Outbound: `Hl7v2MllpPublisher` learns `AppointmentBooked → SIU^S12`
  and `AppointmentCancelled → SIU^S15`. Skips relaying when the
  outbox payload carries `source: "hl7v2_s12"` / `"hl7v2_s15"` —
  boomerang protection so SIU we just received from one EMR isn't
  echoed back as if PAS originated it.
- SCH-2 (filler appointment id) carries the PAS appointment UUID end-
  to-end. The AA ACK for S12 reports the assigned filler id in MSA-3
  so the sender can record it for subsequent S15.
- 15 new lib unit tests + 1 new DB-bound integration test. Lib
  total: 411 → 426. Workspace lib: **435 passing**. Integration:
  44 → 45.

## 2026-05-25 — PAS Axum v0.15.0: Multinational national-identifier coverage

- **PAS Axum v0.15.0** — identity-interop release. The `Identifier`
  model carries typed factories and per-country format validators for
  the five national healthcare-identifier schemes the workspace cares
  about: United Kingdom NHS Number (existing, now formally validated),
  France Numéro d'Identification au Répertoire (NIR / INSEE), España
  Tarjeta Sanitaria Individual (TSI / SNS CIP), Ireland Individual
  Health Identifier (IHI), and Northern Ireland Health & Care Number
  (HCN). Variants serialize UPPERCASE so existing FHIR / bulk / REST
  payloads using `"NHS"` keep working byte-for-byte.
- New `IdentifierType` variants: `NIR`, `TSI`, `IHI`, `HCN`. New typed
  factories: `Identifier::nir`, `::tsi`, `::ihi`, `::hcn`. New
  system-URI constants alongside the existing `NHS_SYSTEM_URI` /
  `SSN_SYSTEM_URI`.
- New format validators in `src/validation/`: `validate_nhs_number`
  (Mod 11), `validate_nir` (Mod 97 with Corsica `2A`/`2B` substitution),
  `validate_tsi` (alphanumeric envelope), `validate_ihi` (7 digits),
  `validate_hcn` (10 digits), plus the dispatch helper
  `validate_identifier(&Identifier)`.
- `validate_patient` now calls the dispatch helper for every entry in
  `Patient.identifiers`, so invalid national-identifier values surface
  through the existing `Error::Validation` → REST `400 VALIDATION` /
  FHIR `OperationOutcome { invalid }` mapping.
- 30 new lib unit tests (6 in `identifier.rs`, 24 in `validation/`).
  Lib total: 381 → 411. Workspace lib: **420 passing**. Integration
  count unchanged at 44; no new tables, no new migrations, no new env
  vars.

## 2026-05-25 — PAS Axum v0.14.0: Outbox webhook publisher

- **PAS Axum v0.14.0** — interop release. New
  `WebhookEventPublisher` POSTs each outbox `DomainEvent` as JSON to
  a configured URL. New `CompositePublisher` fans out to many
  subscribers so the existing HL7 v2 MLLP peer and a webhook
  receiver can both be active at the same time.
- Webhook wire format: `Content-Type: application/json` body that
  serialises the full `DomainEvent` (`id`, `event_type`, `payload`,
  `at`). Always sends `X-PAS-Event-Id` + `X-PAS-Event-Type` headers.
  When `PAS_WEBHOOK_SECRET` is set, also sends
  `X-PAS-Signature: sha256=<hex>` — HMAC-SHA256 over the raw body.
- Failure semantics align with the HL7 v2 publisher: 2xx ⇒
  published, anything else ⇒ outbox row stays pending and the
  dispatcher retries it. After `PAS_OUTBOX_MAX_RETRIES`, the row
  moves to `outbox_dead_letters` like every other publish failure.
- New env vars: `PAS_WEBHOOK_URL`, `PAS_WEBHOOK_SECRET`,
  `PAS_WEBHOOK_TIMEOUT_SECS` (default 10).
- New direct deps: `reqwest = "0.12"`, `sha2 = "0.10"`, `hmac =
  "0.12"` — all already in the transitive dep tree, zero compile
  cost.
- 12 new lib unit tests (8 webhook + 3 composite + 1 config) +
  2 new DB-free integration tests. Lib total: 369 → 381. Workspace
  lib: **390 passing**. Integration: 42 → 44 (`tests/webhook_test.rs`
  is new and DB-free).

## 2026-05-25 — PAS Axum v0.13.0: FHIR Coverage Bundle write

- **PAS Axum v0.13.0** — interop release. `POST /fhir` Bundle entries
  (both `type: batch` and `type: transaction`) now accept the FHIR R5
  `Coverage` resource, completing the read/write surface introduced in
  v0.10. Coverage rows arrive on the same `CoverageRepository::create`
  path as `POST /api/coverages` — same audit trail, same outbox
  events are produced downstream.
- New `FhirCoverage::into_domain()` parses the wire shape back to the
  domain model. Required fields: `beneficiary` (Patient ref),
  `subscriberId` (policy number), `payor[0].display`, `period.start`.
  Optional: `subscriber`, `type.text`, `relationship.text`,
  `period.end`, `payor[0].identifier.value`.
- Status accepts both FHIR kebab-case (`entered-in-error`) and the
  domain snake_case form. Unknown statuses / unknown `type.text`
  values are rejected with a 400.
- Transaction semantics unchanged: any failing Coverage entry rolls
  the entire bundle back; nothing is left half-written.
- 4 new lib unit tests + 2 new integration tests (DB-bound, gated on
  `DATABASE_URL` like every other DB test). Lib total: 365 → 369.
  Workspace lib: **378 passing**. Integration: 40 → 42 (`fhir_bundle_
  test.rs` grew).

## 2026-05-25 — PAS Axum v0.12.0: per-IP rate-limit middleware

- **PAS Axum v0.12.0** — operational hygiene release. New tower
  middleware caps incoming HTTP request rate per peer IP using a
  hand-rolled token-bucket (no new dep). Slotted outside bearer auth
  so brute-force token guessing is throttled, inside trace so 429s
  still get logged. `/api/health` is exempt.
- New env vars `PAS_RATE_LIMIT_RPM` (default 600 = 10 req/sec) and
  `PAS_RATE_LIMIT_BURST` (default 60). Set RPM to `0` to disable.
- On cap: 429 + `Retry-After` header + standard `ApiResponse` envelope
  with `error.code = "RATE_LIMITED"`.
- `axum::serve` now uses `into_make_service_with_connect_info`
  so peer IP is available to the middleware.
- 8 new lib unit tests + 2 new integration tests (both DB-free —
  they exercise the middleware directly). Lib total: 357 → 365.
  Workspace totals: 365 PAS lib + 9 patient-administration-system-frontend csrf = **374 passing**.
  Integration: 38 → 40.
- See
  [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md#0120--2026-05-25).

## 2026-05-25 — PAS Axum v0.11.0: Patient merge / replaced_by tombstone

- **PAS Axum v0.11.0** — closes the long-standing handoff with the
  sister MPI crate. New `Patient.replaced_by: Option<Uuid>` column +
  partial index. `POST /api/patients/{id}/merge-into/{target_id}`
  atomically flips the source to a tombstone (sets `replaced_by`,
  `active = false`), drops it from Tantivy search, writes audit +
  outbox. `GET /api/patients/{id}/replaces` lists the inverse direction
  for a survivor. FHIR `Patient.link[type=replaced-by]` is emitted on
  tombstones so downstream FHIR clients can chase the survivor without
  an extra round-trip.
- New migration `m20260528_000005_patient_replaced_by`.
- New outbox event `PatientMerged { source_id, target_id }`.
- 1 new lib unit test + 1 new integration test. Lib total: 356 → 357.
  Workspace totals: 357 PAS lib + 9 patient-administration-system-frontend csrf = **366 passing**.
  Integration: 37 → 38. OpenAPI: 103 → 105 handlers.
- See
  [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md#0110--2026-05-25).

## 2026-05-25 — PAS Axum v0.10.0: Coverage (insurance) FHIR resource

- **PAS Axum v0.10.0** — the Coverage release. New `Coverage` aggregate
  records insurance / self-pay / other payer info per patient with
  optional linkage to a billing `Account`. FHIR R5 `Coverage` read
  surface (`GET /fhir/Coverage/{id}`) lets downstream payer
  integrations consume PAS data through the standard interop path.
- New migration `m20260527_000004_coverage` adds the `coverages` table
  (32 tables total). Partial unique index prevents active-row dupes;
  partial index on `account_id` keeps the linkage lookup lean.
- Six new REST endpoints + one FHIR R5 read endpoint. Coverage rows
  are never hard-deleted — DELETE flips status to `Cancelled`.
- New outbox events `CoverageCreated` / `CoverageUpdated` /
  `CoverageCancelled`.
- 9 new lib unit tests + 1 new integration test. Lib total: 347 → 356.
  Workspace totals: 356 PAS lib + 9 patient-administration-system-frontend csrf = **365 passing**.
  Integration: 36 → 37. OpenAPI: 97 → 103 handlers.
- See
  [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md#0100--2026-05-25).

## 2026-05-25 — PAS Axum v0.9.0: recurring appointment series

- **PAS Axum v0.9.0** — the recurring appointment release. New
  `AppointmentSeries` aggregate + `RecurrenceRule` (RFC 5545 subset:
  daily/weekly/monthly, INTERVAL, BYDAY for weekly, COUNT|UNTIL
  termination, cap 200 occurrences). Five new endpoints
  (`/api/appointment-series` × 4 + per-patient list).
- New migration `m20260526_000003_appointment_series` adds the
  `appointment_series` table + `series_id` column on `appointments`.
- Series create is **atomic** — a single per-patient overlap on any
  occurrence rolls back the whole transaction. Cancel cascades through
  the appointment state machine (touches only `Proposed` / `Booked`
  rows; leaves terminal statuses alone).
- New outbox events `AppointmentSeriesCreated` / `AppointmentSeriesCancelled`.
- 15 new lib tests + 2 new integration tests. Lib total: 332 → 347.
  Workspace totals: 347 PAS lib + 9 patient-administration-system-frontend csrf = **356 passing**.
  Integration: 34 → 36.
- See
  [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md#090--2026-05-25).

## 2026-05-25 — PAS Axum v0.8.0: SMS letter channel

- **PAS Axum v0.8.0** — the SMS letter channel release.
  `DeliveryChannel::Sms` has been declared but undelivered since v0.1;
  v0.8 wires it via a new `SmsProvider` trait + two first-party
  implementations (`NoopSmsProvider` default; `LogSmsProvider` for
  dev). When the provider reports `is_enabled()`,
  `CommunicationService::generate_letter` auto-dispatches the SMS to
  the patient's first Phone telecom and flips the letter row to
  `Sent` (or `Failed` on error) with corresponding audit entries.
- New env var `PAS_SMS_PROVIDER` (`none` / `log`, default `none`).
  Unknown values fall back to `none` with a startup `warn!` so a typo
  can't accidentally enable auto-send.
- 4 new lib unit tests + 1 new integration test. Lib total: 328 → 332.
  Workspace totals: 332 PAS lib + 9 patient-administration-system-frontend csrf = **341 passing**.
  Integration: 33 → 34.
- See
  [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md#080--2026-05-25).

## 2026-05-25 — PAS Axum v0.7.0: OpenTelemetry OTLP exporter

- **PAS Axum v0.7.0** — the OpenTelemetry release. The `OTLP_ENDPOINT`
  env var has been documented since v0.3.1 but unwired; v0.7 actually
  wires it. When set, the PAS now exports every `tracing` span to the
  configured OTLP collector (HTTP/protobuf transport via reqwest — no
  tonic). Service name comes from `OTEL_SERVICE_NAME` (default
  `pas-axum`). When unset, behavior is unchanged from v0.6 (fmt layer
  only). Setup failures are non-fatal — server still boots, observ-
  ability falls back to local-only.
- Four crates reintroduced (`opentelemetry`, `opentelemetry_sdk`,
  `opentelemetry-otlp`, `tracing-opentelemetry`) — all on the 0.27/
  0.28 series.
- 3 new lib unit tests. Lib total: 326 → 328. Workspace totals: 328
  PAS lib + 9 patient-administration-system-frontend csrf = **337 passing**.
- See
  [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md#070--2026-05-25).

## 2026-05-25 — PAS Axum v0.6.0: HL7 v2 batch envelope

- **PAS Axum v0.6.0** — the HL7 v2 batch release. New endpoint
  `POST /api/hl7/v2/batch` accepts the standard `FHS`/`BHS`/`BTS`/`FTS`
  envelope around up to 1000 ADT messages, dispatches each one
  independently to the matching single-message handler, and returns one
  batch ACK envelope with a per-message MSA inside. Lets a sender ship
  end-of-day batches, backfills, or migration loads in one transmission.
- MLLP listener routes any payload starting with `FHS` or `BHS` to the
  new endpoint — no separate listener required.
- New `src/hl7v2/batch.rs` carries `Batch`, `parse_batch`,
  `encode_batch_ack`, `looks_like_batch`, `MAX_BATCH_MESSAGES = 1000`.
- Lib tests 315 → 326 (+9 batch parser/encoder + 2 MLLP routing).
  Integration 32 → 33. Workspace totals: 326 PAS lib + 9 patient-administration-system-frontend
  csrf = **335 passing**.
- See
  [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md#060--2026-05-25).

## 2026-05-24 — PAS Axum v0.5.0: outbox dead-letter queue

- **PAS Axum v0.5.0** — the outbox dead-letter release. Bounds the
  dispatcher's retry behavior so a chronically-failing peer can no
  longer pile up retries forever. Two new admin endpoints
  (`GET /api/admin/outbox/dead-letters`, `POST /api/admin/outbox/dead-letters/{id}/replay`)
  let an operator review and replay events that got dead-lettered.
- New migration `m20260525_000002_outbox_dlq` adds three retry-tracking
  columns to `outbox_events` and creates the `outbox_dead_letters`
  table. Existing rows back-fill cleanly with `retry_count = 0`.
- New env var `PAS_OUTBOX_MAX_RETRIES` (default `10`).
- Lib tests 314 → 315; integration tests 31 → 32. Workspace totals:
  315 PAS lib + 9 patient-administration-system-frontend csrf = **324 passing**.
- See
  [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md#050--2026-05-24).

## 2026-05-24 — PAS Axum v0.4.0: HL7 v2 ADT lifecycle

- **PAS Axum v0.4.0** — the ADT-lifecycle release. Adds three new HL7 v2
  message types that close the obvious "update / undo" gap in the
  inbound surface:
  - `ADT^A08` (update patient information) → `POST /api/hl7/v2/update`.
    Merges the inbound PID over the existing patient row, preserving
    fields PID doesn't carry (`mpi_id`, `additional_names`, `deceased*`,
    `emergency_contacts`, `marital_status`, `created_at`).
  - `ADT^A11` (cancel admit) → `POST /api/hl7/v2/cancel-admit`. Releases
    the bed assignment, flips the bed to `Cleaning`, moves the encounter
    to `Cancelled`.
  - `ADT^A13` (cancel discharge) → `POST /api/hl7/v2/cancel-discharge`.
    Reinstates the most-recently-discharged admission to its original
    bed if still free; fails with AE if the bed has been taken.
- MLLP listener routes the three new message types; outbound
  `Hl7v2MllpPublisher` relays them when configured (source-gated so
  REST-driven edits don't boomerang to the EMR).
- New domain event `EncounterDischargeCancelled`.
- 6 new lib unit tests + 3 new integration tests. Workspace totals: 314
  PAS lib + 9 patient-administration-system-frontend csrf = **323 passing**.
- See
  [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md#040--2026-05-24).

## 2026-05-24 — patient-administration-system-frontend v0.1.0 + repo unification

- **patient-administration-system-frontend v0.1.0** cut — first release of the Loco-rs front-end.
  Dashboard, patient/ward/RTT pages, three write flows (admit/book/letter)
  proxying through the PAS Axum API, CSRF middleware on every POST,
  HTMX live-refresh, Lily Design System markup throughout. 9 csrf unit
  tests + 13 dashboard smoke tests. See
  [`patient-administration-system-frontend/CHANGELOG.md`](patient-administration-system-frontend/CHANGELOG.md#010--2026-05-24).
- **CSRF security audit** (T7.37) — double-submit-cookie pattern with
  `SameSite=Strict; HttpOnly`. Production HTTPS deployments set
  `PAS_COOKIE_SECURE=1` to add the `Secure` flag.
- **Dep audit** — removed unused dev-deps (`mockall`, `tokio-test`) from
  the PAS Axum crate; removed unused direct deps (`tera`, `tracing`,
  `tracing-subscriber`) and dev-deps (`tokio-test`, `tempfile`) from
  patient-administration-system-frontend.
- **Unified git repo** — the workspace root is now itself a git repo, and
  the `patient-administration-system-rust-crate/` history was merged in
  via `git subtree`. One repo for the whole workspace; `git log` shows
  every commit from the initial v0.1 PAS implementation forward.

PAS Axum bumped to **v0.3.4** to capture the dev-dep cleanup. No
behavior change — pure hygiene. See
[`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md#034--2026-05-24).

## 2026-05-23 — Cargo workspace promotion + v0.3.3

- **Cargo workspace** created at the project root with three members:
  `patient-administration-system-rust-crate`,
  `patient-administration-system-rust-crate/migrations`, and the new
  `patient-administration-system-frontend` sibling. Single shared `Cargo.lock` + `target/`.
- **PAS Axum v0.3.3** released alongside — the "Lily + workspace
  release". Adopted Lily Design System headless markup across the
  dashboard templates. See
  [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md#033--2026-05-23).
- **patient-administration-system-frontend** scaffolded as a sibling Loco-rs 0.14.1 app
  (unreleased at this point — see today's v0.1.0 entry).

For PAS Axum's full release history (v0.1.0 → v0.3.3) see
[`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md).
