# Changelog

All notable changes to the Patient Administration System are listed below.
This project follows [Semantic Versioning](https://semver.org/).

## [0.43.0] — 2026-05-30

The ADT^A07 (Change Inpatient to Outpatient) release. Symmetric
to v0.41 A06: an admitted inpatient is reclassified to
ambulatory status with the bed released, but without a full
discharge. The encounter remains active (InProgress).

### Added — service method

- `AdtService::change_to_outpatient(admission_id, source,
  ctx)` in `src/adt/mod.rs`. Single DB transaction:
    1. Load admission (404 if missing).
    2. Find active `BedAssignment` for the encounter
       (404 if none).
    3. Release the bed_assignment row.
    4. Flip bed → `Cleaning` (regular `Occupied → Cleaning`
       transition).
    5. Reclassify encounter via
       `EncounterRepository::set_class` to `Outpatient`.
       Status stays `InProgress`.
    6. Write `audit_log` (`action = "change_to_outpatient"`)
       and `outbox_events`
       (type = `EncounterDemotedToOutpatient`, payload tagged
       with the supplied `source`).
- The `admissions` row is preserved as historical record so
  subsequent A13 (cancel-discharge) or audit queries can
  still locate it.

### Added — inbound handler

- `POST /api/hl7/v2/change-to-outpatient` in
  `src/api/rest/handlers.rs::hl7_v2_change_to_outpatient`.
  Locates the open admission via the existing
  `locate_open_admission_for_v2` helper with
  `expected_event = "A07"`, delegates to
  `state.adt.change_to_outpatient` with
  `source = Some("hl7v2_a07")`.

### Added — MLLP routing

- `("ADT", "A07") => "/api/hl7/v2/change-to-outpatient"` arm
  in `src/hl7v2/listener.rs::route_for_payload` + matching
  unit test (`test_route_for_payload_a07`).

### Added — wiring

- `src/api/rest/routes.rs`: mounts the new route.
- `src/api/openapi.rs`: registers
  `hl7_v2_change_to_outpatient`.

### Tests

- 1 new lib test. Lib total 496 → **497 passing**; workspace
  lib **506 passing**.
- 1 new DB-bound integration test
  (`hl7v2_adt_a07_demotes_inpatient_to_outpatient`): A01
  admits Inpatient + Occupied → A07 demotes → assert
  encounter Outpatient+InProgress + bed Cleaning; second
  A07 → 400 (no remaining open admission); wrong message
  type → 400. Integration: 61 → **62** functions.

### Other

- Cargo: bumped to `0.43.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.42.0] — 2026-05-30

The outbound ADT^A06 release. Closes the v0.41 inbound loop and
adds the PAS-native entry-point for promoting an ambulatory
encounter to an inpatient admission: REST
`POST /api/admissions/change-to-inpatient`.

### Added — wire encoder

- `encode_adt_a06(patient, bed, sending_app, receiving_app,
  message_control_id)` in `src/hl7v2/mapping.rs`. Builds
  MSH + EVN + PID + PV1 with PV1-2 = `I` + PV1-3 =
  `^^<bed.code>` (same shape as A01 admit, but with the A06
  trigger code signalling a class change rather than a
  fresh admit). Re-exported from `src/hl7v2/mod.rs`.

### Added — REST endpoint

- `POST /api/admissions/change-to-inpatient` →
  `handlers::change_to_inpatient`. Body
  `{ patient_id, bed_id }`. Calls
  `state.adt.change_to_inpatient(patient_id, bed_id, None,
  &ctx)` so the outbox payload omits the `source` field
  and the publisher relays the event downstream.
- Route wired in `src/api/rest/routes.rs`; `ApiDoc`
  registration in `src/api/openapi.rs`.

### Added — outbound publisher arm

- `Hl7v2MllpPublisher::publish` learns one new match arm:
  `EncounterPromotedToInpatient → ADT^A06`. Resolves
  patient + bed via the payload's `patient_id` / `bed_id`,
  then calls `encode_adt_a06`. Source-gated on
  `hl7v2_a06` so v0.41 inbound events don't echo back.
- New `EncounterPromotedToInpatientPayload` deserialise
  struct in `src/streaming/hl7v2_publisher.rs`.

### Tests

- 1 new lib test (`test_encode_adt_a06_includes_bed_code`).
  Lib total 495 → **496 passing**; workspace lib
  **505 passing**.
- No new integration test — the v0.41 inbound A06 test
  continues to exercise both directions through the
  `source: "hl7v2_a06"` boomerang-protection tag.

### Other

- Cargo: bumped to `0.42.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.41.0] — 2026-05-30

The ADT^A06 (Change Outpatient to Inpatient) release. Promotes
an existing Outpatient or Emergency encounter to a fresh
Inpatient admission with bed allocation. Closes a gap the
ambulatory-registration path (v0.28 A04) opened: the natural
follow-on when an ED patient is admitted to a ward.

### Added — service method

- `AdtService::change_to_inpatient(patient_id, bed_id, source,
  ctx)` in `src/adt/mod.rs`. Single DB transaction:
    1. Lock destination bed (404 if missing; 409 if not
       `Available`).
    2. Find patient's most-recent active ambulatory encounter
       (404 + AE if none).
    3. Reclassify via `EncounterRepository::set_class`
       (Inpatient).
    4. Advance encounter from `Arrived → InProgress` via the
       regular state machine if still `Arrived`.
    5. Flip bed → `Occupied`.
    6. Insert `BedAssignment` + `Admission`.
    7. Write audit + outbox `EncounterPromotedToInpatient`
       tagged with the supplied `source`.

### Added — repo helpers

- `EncounterRepository::set_class(conn, id, new_class)` —
  administrative class change (the `EncounterClass` enum
  carries no state-machine constraint, so this is a plain
  field update). Used by A06 here and the future A07.
- `EncounterRepository::find_latest_active_ambulatory_for_patient(conn,
  patient_id)` — returns `Option<Encounter>` filtered to
  `class IN ('outpatient', 'emergency')` and `status IN
  ('arrived', 'in_progress')`, ordered by `created_at DESC`.

### Added — inbound handler

- `POST /api/hl7/v2/change-to-inpatient` in
  `src/api/rest/handlers.rs::hl7_v2_change_to_inpatient`.
  Parses MSH + PID + PV1, looks up patient by PID-3.1, bed
  by PV1-3.3, delegates to
  `state.adt.change_to_inpatient` with
  `source = Some("hl7v2_a06")`.

### Added — MLLP routing

- `("ADT", "A06") => "/api/hl7/v2/change-to-inpatient"` arm
  in `src/hl7v2/listener.rs::route_for_payload` + matching
  unit test (`test_route_for_payload_a06`).

### Added — wiring

- `src/api/rest/routes.rs`: mounts the new route.
- `src/api/openapi.rs`: registers
  `hl7_v2_change_to_inpatient`.

### Tests

- 1 new lib test. Lib total 494 → **495 passing**; workspace
  lib **504 passing**.
- 1 new DB-bound integration test
  (`hl7v2_adt_a06_promotes_outpatient_to_inpatient`): A04
  registers Outpatient → A06 promotes → assert encounter
  Inpatient + InProgress + bed Occupied; second A06 → 404
  (no more active ambulatory); unknown patient → 404; wrong
  message type → 400. Integration: 60 → **61** functions.

### Other

- Cargo: bumped to `0.41.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.40.0] — 2026-05-30

The outbound ADT^A23 release. Closes the v0.39 inbound loop and
brings the long-standing REST `DELETE /api/patients/{id}`
handler into line with the same safety constraints + outbox
write semantics that v0.39's HL7 path uses.

### Changed — REST DELETE /api/patients/{id}

- Adds the v0.39 safety check: refuses (409) when the patient
  has any open admission via
  `AdmissionRepository::find_open_for_patient`.
- Wraps the soft-delete + audit + `PatientDeleted` outbox
  write in a single `DatabaseTransaction`. The audit row
  preserves its existing `soft_delete` action; the new
  outbox event carries `{patient_id}` with **no** `source`
  tag (REST-driven, so the publisher will relay it).
- Post-commit: best-effort Tantivy drop (unchanged).
- 200 / 404 / 409 response mapping replaces the previous
  silent-success behavior.

### Added — wire encoder

- `encode_adt_a23(patient, sending_app, receiving_app,
  message_control_id)` in `src/hl7v2/mapping.rs`. Builds
  MSH + EVN + PID; no PV1. Re-exported from
  `src/hl7v2/mod.rs`.

### Added — outbound publisher arm

- `Hl7v2MllpPublisher::publish` learns one new match arm:
  `PatientDeleted → ADT^A23`. Source-gated on `hl7v2_a23`
  so v0.39 inbound events don't echo back. Resolves the
  patient via `PatientRepository::find_by_id` — which
  returns soft-deleted rows by primary key, so the PID
  payload remains encodable post-deletion.
- New `PatientDeletedPayload` deserialise struct accepts
  the `{patient_id, source?}` payload shape.

### Tests

- 1 new lib test (`test_encode_adt_a23_omits_pv1`). Lib
  total 493 → **494 passing**; workspace lib **503 passing**.
- No new integration test — the v0.39 inbound A23 test
  continues to exercise both directions through the
  `source: "hl7v2_a23"` boomerang-protection tag.

### Other

- Cargo: bumped to `0.40.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.39.0] — 2026-05-30

The ADT^A23 (Delete a Patient Record) release. Maps the HL7 v2
"this patient was created in error" trigger to a soft-delete on
the patient row, with a safety check that refuses to destroy a
patient who currently has an open admission.

### Added — inbound handler

- `POST /api/hl7/v2/delete-patient` in
  `src/api/rest/handlers.rs::hl7_v2_delete_patient`. Parses
  MSH + PID, looks up the patient by PID-3.1 (MRN). Order
  of operations:
    1. PID parse / MRN-present validation (`400 + AE` on
       failure).
    2. Patient lookup
       (`PatientRepository::find_by_identifier_value`); 404
       + AE if not found.
    3. Safety check: refuse if any open admission exists for
       the patient (`AdmissionRepository::find_open_for_
       patient` returns Some); 409 + AE with the open
       admission's UUID in the diagnostic.
    4. Single DB transaction:
       `PatientRepository::soft_delete` (sets `deleted_at`),
       `AuditLogRepository::log` (`action =
       "delete_via_hl7v2_a23"`),
       `OutboxRepository::publish` (`PatientDeleted` event,
       payload tagged `source: "hl7v2_a23"`).
    5. Post-commit: best-effort drop from the Tantivy search
       index (same pattern as the REST
       `DELETE /api/patients/{id}` handler).
- AA ACK on success carries `patient <uuid> soft-deleted` in
  MSA-3.

### Added — MLLP routing

- `("ADT", "A23") => "/api/hl7/v2/delete-patient"` arm in
  `src/hl7v2/listener.rs::route_for_payload` + matching
  unit test (`test_route_for_payload_a23`).

### Added — wiring

- `src/api/rest/routes.rs`: mounts `POST /api/hl7/v2/delete-
  patient` next to `/leave-end`.
- `src/api/openapi.rs`: registers `hl7_v2_delete_patient` in
  the `ApiDoc` paths so Swagger UI picks it up.

### Tests

- 1 new lib test. Lib total 492 → **493 passing**; workspace
  lib **502 passing**.
- 1 new DB-bound integration test
  (`hl7v2_adt_a23_deletes_patient_soft_with_safety_check`):
  register patient via A28 → A23 deletes → subsequent
  `find_by_identifier_value` returns None; unknown MRN →
  404 + AE; admitted-patient A23 → 409 + AE; wrong message
  type → 400 + AE. Integration: 59 → **60** functions.

### Other

- Cargo: bumped to `0.39.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.38.0] — 2026-05-30

The outbound ADT^A11 retrofit release. Brings the v0.4
cancel-admit lifecycle into line with the boomerang-protection
pattern that v0.25 onwards established for every other
inbound/outbound pair, and adds the missing REST entry-point.

### Changed — publisher source-gate inverted

- `Hl7v2MllpPublisher::publish`'s `EncounterCancelled` arm
  previously only relayed events where `reason ==
  "hl7v2_a11"` — that boomeranged HL7-driven cancels back to
  the EMR that just sent them, and silenced everything else.
  v0.38 inverts the gate: events with `reason ==
  "hl7v2_a11"` are now silently dropped (boomerang
  protection), and REST-driven cancels emit downstream.
- Doc-header bullet updated accordingly:
  `EncounterCancelled → ADT^A11 (v0.38 retrofit; skipped
  when reason == "hl7v2_a11")`.

### Changed — AdtService::cancel_admission signature

- Signature changed from `(admission_id, ctx)` to
  `(admission_id, reason: Option<&str>, ctx)`. The HL7 v2
  handler passes `Some("hl7v2_a11")`; the new REST handler
  passes `None`. The previously-hardcoded
  `reason: "hl7v2_a11"` in the payload is now conditionally
  written only when the parameter is `Some`.

### Added — REST endpoint

- `POST /api/admissions/{id}/cancel-admit` →
  `handlers::cancel_admit`. Calls
  `state.adt.cancel_admission(admission_id, None, &ctx)`.
  ACK mapping: `Conflict` /
  `InvalidStateTransition` → 409; `NotFound` → 404;
  success → 200. Route wired in
  `src/api/rest/routes.rs`; `ApiDoc` registration in
  `src/api/openapi.rs`.

### Tests

- No new lib tests — the change is a signature refactor plus
  a one-line gate inversion. Lib total remains **492 passing**;
  workspace lib **501 passing**.
- Integration test housekeeping (drive-by):
  - `hl7v2_adt_a01_admits_patient_to_bed`: facility / ward /
    room codes now carry a random suffix so the test is
    re-runnable against a stale DB (previously failed on
    second invocation with a unique-constraint violation).
  - `hl7v2_adt_a11_cancels_admit` /
    `hl7v2_adt_a13_cancels_discharge`: ward-occupancy
    assertions now read the correct `occupied` field
    (`WardOccupancy.occupied: usize`) instead of the
    imaginary `occupied_beds`; both tests were silently
    failing on a freshly-cleaned DB.

### Other

- Cargo: bumped to `0.38.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.37.0] — 2026-05-30

The outbound LOA release. Closes the v0.36 inbound A21 / A22
loop and adds the first PAS-native (non-HL7) entry-points for
toggling leave of absence: REST
`POST /api/admissions/{id}/leave-start` and `/leave-end`.

### Added — wire encoders

- `encode_adt_a21(patient, sending_app, receiving_app,
  message_control_id)` and
  `encode_adt_a22(patient, sending_app, receiving_app,
  message_control_id)` in `src/hl7v2/mapping.rs`. Build
  MSH + EVN + PID; no PV1 segment is emitted (the receiver
  locates the open admission by patient identity — same
  shape as A03 discharge). Re-exported from
  `src/hl7v2/mod.rs`.

### Added — REST endpoints

- `POST /api/admissions/{id}/leave-start` →
  `handlers::leave_start`. Calls
  `state.adt.start_leave(admission_id, None, &ctx)`. The
  `None` passes through to the outbox payload as the
  absence of a `source` field — which is what the
  publisher's boomerang gate wants.
- `POST /api/admissions/{id}/leave-end` →
  `handlers::leave_end`. Same shape, calls
  `state.adt.end_leave`.
- Routes wired in `src/api/rest/routes.rs`; `ApiDoc`
  registration in `src/api/openapi.rs`.

### Added — outbound publisher arms

- `Hl7v2MllpPublisher::publish` learns two new match arms:
  `EncounterLeaveStarted → emit_loa(.., "A21", "hl7v2_a21")`
  and `EncounterLeaveEnded → emit_loa(.., "A22",
  "hl7v2_a22")`.
- New private `emit_loa` helper resolves admission →
  encounter → patient, then calls the matching encoder.
  Source-gated on the supplied tag so v0.36 inbound events
  don't echo back.
- New `EncounterLoaPayload` deserialise struct accepts the
  `{admission_id, source?}` payload shape that
  `start_leave` / `end_leave` emit.

### Tests

- 2 new lib tests (`test_encode_adt_a21_omits_pv1`,
  `test_encode_adt_a22_omits_pv1`). Lib total 490 →
  **492 passing**; workspace lib **501 passing**.
- No new integration tests — the v0.36 inbound LOA test
  continues to exercise both directions through the
  source-gating tag (its HL7 path passes the inbound source,
  the REST path passes `None`).

### Other

- Cargo: bumped to `0.37.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.36.0] — 2026-05-30

The leave-of-absence release. ADT^A21 (patient goes on LOA) and
ADT^A22 (patient returns from LOA) bracket a temporary
interruption in an active inpatient stay without releasing the
bed — the patient is expected back so the bed is held.

### Added — service methods

- `AdtService::start_leave(admission_id, source, ctx)` — moves
  the encounter `InProgress → OnLeave`. Source-tag pattern
  follows v0.31 / v0.33 / v0.35.
- `AdtService::end_leave(admission_id, source, ctx)` — moves
  the encounter `OnLeave → InProgress`.
- Both share a private `transition_loa` helper in
  `src/adt/mod.rs` that owns the transaction shape (validate
  state via `EncounterRepository::set_status` —
  `try_transition_to` enforces the state machine; write audit
  + outbox; emit best-effort in-memory event after commit).
- Outbox event types: `EncounterLeaveStarted` (source
  `hl7v2_a21` from the HL7 path, `None` otherwise);
  `EncounterLeaveEnded` (source `hl7v2_a22`).
- Bed is intentionally not touched — `Occupied` is the correct
  HL7 semantic during LOA so the bed isn't reassigned.

### Added — inbound handlers

- `POST /api/hl7/v2/leave-start` →
  `handlers::hl7_v2_leave_start`. Parses MSH + PID, locates
  the patient's currently-open admission via the existing
  `locate_open_admission_for_v2` helper with
  `expected_event = "A21"`, delegates to
  `state.adt.start_leave` with `source = Some("hl7v2_a21")`.
- `POST /api/hl7/v2/leave-end` →
  `handlers::hl7_v2_leave_end`. Same shape with
  `expected_event = "A22"` and
  `source = Some("hl7v2_a22")`.
- Both share a private `handle_loa_transition` helper that
  maps service errors to ACK codes: `Conflict` /
  `InvalidStateTransition` → 409 + AE; `NotFound` → 404 + AE;
  parse / wrong-type → 400 + AE; success → 200 + AA.

### Added — MLLP routing

- `("ADT", "A21") => "/api/hl7/v2/leave-start"` and
  `("ADT", "A22") => "/api/hl7/v2/leave-end"` arms in
  `src/hl7v2/listener.rs::route_for_payload` + 2 matching
  unit tests (`test_route_for_payload_a21`,
  `test_route_for_payload_a22`).

### Added — wiring

- `src/api/rest/routes.rs`: mounts both new routes next to
  the existing `/cancel-pre-admit`.
- `src/api/openapi.rs`: registers both handlers in the
  `ApiDoc` paths so Swagger UI picks them up.

### Tests

- 2 new lib tests. Lib total 488 → **490 passing**; workspace
  lib **499 passing**.
- 1 new DB-bound integration test
  (`hl7v2_adt_a21_a22_leave_of_absence_round_trip`):
  bootstrap → A01 admit → A21 leave-start → assert encounter
  OnLeave + bed still Occupied → second A21 → 409 + AE → A22
  leave-end → assert encounter InProgress → second A22 →
  409 + AE → wrong message type at /leave-start → 400 + AE.
  Integration: 58 → **59** functions.

### Other

- Cargo: bumped to `0.36.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.35.0] — 2026-05-30

The outbound ADT^A38 release. Closes the v0.34 inbound loop
and adds the PAS-native (non-HL7) entry-point for cancelling
a pre-admission: REST `POST /api/admissions/cancel-pre-admit`.

### Added — wire encoder

- `encode_adt_a38(patient, released_bed, sending_app,
  receiving_app, message_control_id)` in
  `src/hl7v2/mapping.rs`. Builds MSH + EVN + PID + PV1 with
  PV1-2 = `I` + PV1-3 = `^^<released_bed.code>` so the
  receiver can locate the prior A05 it sent. Re-exported
  from `src/hl7v2/mod.rs`.

### Added — REST endpoint

- `POST /api/admissions/cancel-pre-admit` →
  `handlers::cancel_pre_admit`. Body `{ patient_id, bed_id }`.
  Calls `state.adt.cancel_pre_admit(patient_id, bed_id, None,
  &ctx)`. The `None` passes through to the outbox payload as
  the absence of a `source` field — which is what the
  publisher's boomerang gate wants.
- Route wired in `src/api/rest/routes.rs`; `ApiDoc`
  registration in `src/api/openapi.rs`.

### Added — outbound publisher arm

- `Hl7v2MllpPublisher::publish` learns one new match arm:
  `EncounterPreAdmitCancelled → ADT^A38`. Resolves patient +
  bed from the payload's `patient_id` / `bed_id`, then calls
  `encode_adt_a38`. Source-gated on `hl7v2_a38` so v0.34
  inbound events don't echo back.
- New `EncounterPreAdmitCancelledPayload` deserialise struct
  in `src/streaming/hl7v2_publisher.rs` accepts the
  `{patient_id, bed_id, source?}` payload shape that
  `AdtService::cancel_pre_admit` emits.

### Tests

- 1 new lib test
  (`test_encode_adt_a38_includes_released_bed_code`). Lib
  total 487 → **488 passing**; workspace lib **497 passing**.
- No new integration test — the v0.34 inbound A38 test
  continues to exercise both directions through the
  `source: "hl7v2_a38"` boomerang-protection tag.

### Other

- Cargo: bumped to `0.35.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.34.0] — 2026-05-30

The ADT^A38 (Cancel Pre-admit) release. Closes the lifecycle
that v0.32 / v0.33 opened: an EMR can now signal to PAS that a
planned admission has fallen through, and PAS releases the bed
back to `Available` and cancels the planned encounter.

### Added — service method

- `AdtService::cancel_pre_admit(patient_id, bed_id, source,
  ctx)` in `src/adt/mod.rs`. Single DB transaction:
    1. Lock the bed (404 if missing); must be `Reserved`
       (409 + AE otherwise).
    2. Find the patient's most-recent Planned inpatient
       encounter (404 + AE if none).
    3. Flip bed → `Available` via the regular state-machine
       transition (`Reserved → Available` is legal).
    4. Set encounter → `Cancelled` via the regular state
       machine (`Planned → Cancelled` is legal).
    5. Write `audit_log` (`action = "cancel_pre_admit"`) and
       `outbox_events` (type = `EncounterPreAdmitCancelled`,
       `source` written conditionally — `Some("hl7v2_a38")`
       from the HL7 path, `None` from any future REST path).
- Signature matches the v0.31 / v0.33 source-tag pattern so a
  future REST `POST /api/admissions/cancel-pre-admit` can
  trigger the outbound `ADT^A38` (out of scope for v0.34).

### Added — repo helper

- `EncounterRepository::find_latest_planned_inpatient_for_patient(conn,
  patient_id)` returns `Option<Encounter>`. Filters by
  `status = "planned"`, `class = "inpatient"`,
  `deleted_at IS NULL`; orders by `created_at DESC` to pick
  the most-recent reservation if more than one exists.

### Added — inbound handler

- `POST /api/hl7/v2/cancel-pre-admit` in
  `src/api/rest/handlers.rs::hl7_v2_cancel_pre_admit`. Parses
  MSH + PID + PV1, looks up the patient by PID-3.1 (must
  already exist — 404 + AE if not; unlike A05 there is no
  dedup-or-create for cancellation), resolves the bed by
  PV1-3.3, delegates to `state.adt.cancel_pre_admit` with
  `source = Some("hl7v2_a38")`. ACK mapping: AA on success;
  AE on missing PV1 / bed code / MRN (400); AE on unknown
  patient / bed / planned encounter (404); AE on bed not
  Reserved (409).

### Added — MLLP routing

- `("ADT", "A38") => "/api/hl7/v2/cancel-pre-admit"` arm in
  `src/hl7v2/listener.rs::route_for_payload` + a matching
  unit test (`test_route_for_payload_a38`).

### Added — wiring

- `src/api/rest/routes.rs`: mounts
  `POST /api/hl7/v2/cancel-pre-admit` next to `/pre-admit`.
- `src/api/openapi.rs`: registers `hl7_v2_cancel_pre_admit`
  in the `ApiDoc` paths so Swagger UI picks it up.

### Tests

- 1 new lib test (`test_route_for_payload_a38`). Lib total
  486 → **487 passing**; workspace lib **496 passing**.
- 1 new DB-bound integration test
  (`hl7v2_adt_a38_cancels_pre_admit`): bootstrap → A05
  reserves bed → A38 releases bed and cancels encounter →
  re-cancel (bed now Available) → 409 + AE → unknown
  patient MRN → 404 + AE → wrong message type at
  /cancel-pre-admit → 400 + AE. Integration: 57 → **58**
  functions.

### Other

- Cargo: bumped to `0.34.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.33.0] — 2026-05-30

The outbound ADT^A05 release. Closes the v0.32 inbound loop and
adds the first PAS-native (non-HL7) entry-point for pre-admitting
a patient: REST `POST /api/admissions/pre-admit`.

### Added — wire encoder

- `encode_adt_a05(patient, reserved_bed, sending_app,
  receiving_app, message_control_id)` in `src/hl7v2/mapping.rs`.
  Builds MSH + EVN + PID + PV1 with PV1-2 = `I` (inpatient) +
  PV1-3 = `^^<reserved_bed.code>` so the receiver knows which
  bed PAS has reserved. Re-exported from `src/hl7v2/mod.rs`.

### Added — REST endpoint

- `POST /api/admissions/pre-admit` →
  `handlers::pre_admit`. Body `{ patient_id, bed_id }`.
  Calls `state.adt.pre_admit(patient_id, bed_id, None, &ctx)`.
  The `None` passes through to the outbox payload as the
  absence of a `source` field — which is what the publisher's
  boomerang gate wants.
- Route wired in `src/api/rest/routes.rs`; `ApiDoc`
  registration in `src/api/openapi.rs`.

### Added — outbound publisher arm

- `Hl7v2MllpPublisher::publish` learns one new match arm:
  `EncounterPreAdmitted → ADT^A05`. Resolves patient + bed
  from the payload's `patient_id` / `bed_id`, then calls
  `encode_adt_a05`. Source-gated on `hl7v2_a05` so v0.32
  inbound events don't echo back.
- New `EncounterPreAdmittedPayload` deserialise struct in
  `src/streaming/hl7v2_publisher.rs` accepts the
  `{patient_id, bed_id, source?}` payload shape that
  `AdtService::pre_admit` emits.

### Changed — AdtService::pre_admit signature

- Signature changed from `(patient_id, bed_id, ctx)` to
  `(patient_id, bed_id, source: Option<&str>, ctx)`. The HL7
  v2 handler passes `Some("hl7v2_a05")`; the new REST handler
  passes `None`. The previous hardcoded
  `source: "hl7v2_a05"` in the payload is now conditionally
  written only when the parameter is `Some`.

### Tests

- 1 new lib test (`test_encode_adt_a05_includes_reserved_bed_code`).
  Lib total 485 → **486 passing**; workspace lib **495 passing**.
- No new integration test — the v0.32 inbound A05 test continues
  to exercise both directions through the
  `source: "hl7v2_a05"` boomerang-protection tag (its
  pre_admit call now passes `Some("hl7v2_a05")` through the
  new signature, same end-to-end behaviour).

### Other

- Cargo: bumped to `0.33.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.32.0] — 2026-05-30

The ADT^A05 (Pre-admit Patient) release. Adds the planned-
admission path: an EMR can now signal to PAS that a patient is
expected at a particular bed at some future time, and PAS
reserves the bed without yet creating an admission row.

### Added — service method

- `AdtService::pre_admit(patient_id, bed_id, ctx)` in
  `src/adt/mod.rs`. Single DB transaction:
    1. Lock the destination bed (404 if missing).
    2. Assert bed is `Available` (409 + AE otherwise — the
       reservation can't ride over an existing Occupied /
       Reserved / Cleaning / OutOfService bed).
    3. Flip bed → `Reserved` (regular `Available → Reserved`
       transition).
    4. Create an `Encounter` in `Planned` status with
       `class = Inpatient`.
    5. Write `audit_log` (`action = "pre_admit"`) and
       `outbox_events` (type = `EncounterPreAdmitted`, payload
       tagged `source: "hl7v2_a05"`).
- Deliberately does *not* create `Admission` or
  `BedAssignment` rows — the patient isn't physically in the
  bed yet. The reservation lives in the bed's status alone.
  A subsequent ADT^A01 admit on the same bed would currently
  be rejected because the bed is `Reserved` not `Available`;
  promotion of a pre-admission to an active admission is
  future work.

### Added — inbound handler

- `POST /api/hl7/v2/pre-admit` in
  `src/api/rest/handlers.rs::hl7_v2_pre_admit`. Parses
  MSH + PID + PV1, validates message type is `ADT^A05`,
  resolves the bed by `PV1-3.3`, dedup-or-creates the
  patient via the existing `dedup_or_create_patient_from_pid`
  helper, then delegates to `state.adt.pre_admit`. ACK code
  mapping: AA on success; AE on PV1 missing / bed code
  missing / wrong message type (400); AE on unknown bed
  (404); AE on bed not Available (409).

### Added — MLLP routing

- `("ADT", "A05") => "/api/hl7/v2/pre-admit"` arm in
  `src/hl7v2/listener.rs::route_for_payload` + a matching
  unit test (`test_route_for_payload_a05`).

### Added — wiring

- `src/api/rest/routes.rs`: mounts `POST /api/hl7/v2/pre-admit`
  next to the existing `/admit` route.
- `src/api/openapi.rs`: registers `hl7_v2_pre_admit` in the
  `ApiDoc` paths so Swagger UI picks it up.

### Tests

- 1 new lib test (`test_route_for_payload_a05`). Lib total
  484 → **485 passing**; workspace lib **494 passing**.
- 1 new DB-bound integration test in `tests/hl7v2_test.rs`
  (`hl7v2_adt_a05_pre_admits_reserves_bed_plans_encounter`)
  covering: bootstrap facility / ward / room / bed, A05
  reserves bed and plans encounter (`Reserved` + `Planned`),
  second A05 on the same bed → 409 + AE, unknown bed → 404 +
  AE, wrong message type at /pre-admit → 400 + AE.
  Integration total **57** functions across **24** files.

### Other

- Cargo: bumped to `0.32.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.31.0] — 2026-05-30

The outbound ADT^A12 release. Closes the v0.30 inbound loop and
also adds the first PAS-native (non-HL7) entry-point for cancelling
a transfer: REST `POST /api/admissions/{id}/cancel-transfer`.

### Added — wire encoder

- `encode_adt_a12(patient, origin_bed, sending_app, receiving_app,
  message_control_id)` in `src/hl7v2/mapping.rs`. Builds MSH +
  EVN + PID + PV1 with PV1-3 = `^^<origin_bed.code>` so the
  receiver knows which bed to restore the patient to.
  Re-exported from `src/hl7v2/mod.rs`.

### Added — REST endpoint

- `POST /api/admissions/{id}/cancel-transfer` →
  `handlers::cancel_transfer`. Calls
  `state.adt.cancel_transfer(admission_id, None, &ctx)`. The
  `None` passes through to the outbox payload as the absence
  of a `source` field — which is exactly what the publisher's
  boomerang gate wants.
- Route wired in `src/api/rest/routes.rs`; `ApiDoc`
  registration in `src/api/openapi.rs`.

### Added — outbound publisher arm

- `Hl7v2MllpPublisher::publish` learns one new match arm:
  `EncounterTransferCancelled → ADT^A12`. Resolves admission →
  encounter → patient + the origin bed from the payload's
  `from_bed_id`, then calls `encode_adt_a12`. Source-gated on
  `hl7v2_a12` so v0.30 inbound events don't echo back.
- New `EncounterTransferCancelledPayload` deserialise struct
  in `src/streaming/hl7v2_publisher.rs` accepts the
  `{admission_id, from_bed_id, source?}` payload shape that
  `AdtService::cancel_transfer` emits.

### Changed — AdtService::cancel_transfer signature

- Signature changed from `(admission_id, ctx)` to
  `(admission_id, source: Option<&str>, ctx)`. The HL7 v2
  handler passes `Some("hl7v2_a12")`; the new REST handler
  passes `None`. The previous hardcoded `reason: "hl7v2_a12"`
  in the payload is now conditionally written as `source` only
  when the parameter is `Some`.

### Tests

- 1 new lib test (`test_encode_adt_a12_includes_origin_bed_code`).
  Lib total 483 → **484 passing**; workspace lib **493 passing**.
- No new integration test — the v0.30 inbound A12 test continues
  to exercise both directions through the
  `source: "hl7v2_a12"` boomerang-protection tag (its
  cancel_transfer call now passes `Some("hl7v2_a12")` through
  the new signature, same end-to-end behaviour).

### Other

- Cargo: bumped to `0.31.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.30.0] — 2026-05-29

The ADT^A12 (Cancel Transfer) release. Completes the
A11 / A12 / A13 cancel trio in the ADT surface — PAS can now
undo any of the three discrete ADT state-change actions (admit,
transfer, discharge) on receipt of the matching cancel message.

### Added — service method

- `AdtService::cancel_transfer(admission_id, ctx)` in
  `src/adt/mod.rs`. Single DB transaction:
  1. Find the admission (404 if missing).
  2. Find the most-recent `Transfer` row for that admission
     (404 if none).
  3. Find the currently-active `BedAssignment` (must be on
     the transfer's `to_bed_id`; 409 if invariant broken).
  4. Lock the origin bed; must be `Available` or `Cleaning`
     (409 + AE if not — same safety constraint as A13
     cancel-discharge).
  5. Release the destination `BedAssignment`, flip destination
     bed → `Cleaning` (regular `Occupied → Cleaning`
     transition), flip origin bed → `Occupied` via the
     `_unchecked` helper (the `Cleaning → Occupied` transition
     is outside the regular state-machine flow, same
     exception documented in spec.md §6.4 for A13).
  6. Insert a new active `BedAssignment` on the origin bed.
  7. Physically delete the cancelled `transfers` row (the
     `transfers` table is operational, not append-only —
     same treatment as `discharges` for A13).
  8. Write `audit_log` (`action = "cancel_transfer"`) and
     `outbox_events` (type = `EncounterTransferCancelled`,
     payload includes `reason: "hl7v2_a12"`).

### Added — repository helpers

- `AdmissionRepository::find_latest_transfer_for_admission(conn,
  admission_id)` returns `Option<Transfer>`. Used by the new
  service method.
- `AdmissionRepository::delete_transfer(conn, transfer_id)`
  returns `u64`. Mirrors `delete_discharge` for A13.
- Private `transfer_from_model` converts the entity to the
  domain model (mirrors `discharge_from_model`).

### Added — HTTP + MLLP

- `POST /api/hl7/v2/cancel-transfer` in
  `src/api/rest/handlers.rs::hl7_v2_cancel_transfer`. Locates
  the open admission via the existing `locate_open_admission_
  for_v2` helper (same PID-3.1 lookup as A11) with
  `expected_event = "A12"`, then delegates to
  `state.adt.cancel_transfer`. 400 / 404 / 409 / 200 ACK code
  mapping matches A11.
- Route wired in `src/api/rest/routes.rs`; `ApiDoc`
  registration in `src/api/openapi.rs`.
- MLLP routing arm `("ADT","A12") => "/api/hl7/v2/cancel-
  transfer"` in `src/hl7v2/listener.rs::route_for_payload` +
  unit test (`test_route_for_payload_a12`).

### Tests

- 1 new lib test (MLLP routing). Lib total 482 → **483
  passing**; workspace lib **492 passing**.
- 1 new DB-bound integration test
  (`hl7v2_adt_a12_cancels_transfer`) in `tests/hl7v2_test.rs`:
  bootstrap facility + ward + room + 2 beds → admit to A →
  transfer to B → cancel-transfer → assert A is `Occupied` /
  B is `Cleaning` → re-cancel → 404 + AE → wrong message type
  at /cancel-transfer → 400 + AE. Integration: 55 → **56**
  functions across 24 files.

### Other

- Cargo: bumped to `0.30.0`.
- Quality gates: cargo build clean; clippy clean; fmt clean.

## [0.29.0] — 2026-05-29

The outbound ADT^A04 release. Closes the v0.28 inbound loop:
when a REST consumer creates an outpatient or emergency-class
encounter, the outbox now carries an `EncounterRegistered`
event which the HL7 v2 publisher fans out as `ADT^A04` to the
configured MLLP peer. Same boomerang protection as the other
inbound/outbound pairs.

### Added — wire encoder

- `encode_adt_a04(patient, class_code, sending_app,
  receiving_app, message_control_id)` in
  `src/hl7v2/mapping.rs`. Builds MSH + EVN + PID + PV1 with
  PV1-2 = the supplied class code (typically `"O"` outpatient
  or `"E"` emergency — but the helper takes any literal v2
  patient class code so day-case / home-care / virtual could
  use the same path later). Re-exported from
  `src/hl7v2/mod.rs`.

### Added — outbound publisher arm

- `Hl7v2MllpPublisher::publish` learns one new match arm:
  `EncounterRegistered → ADT^A04`. The class field in the
  outbox payload is mapped to the PV1-2 code (`"Emergency"`
  → `"E"`, anything else → `"O"`). Source-gated on
  `hl7v2_a04` so v0.28 inbound events don't echo back.
- A new `EncounterRegisteredPayload` deserialise struct in
  `src/streaming/hl7v2_publisher.rs` accepts the
  `{patient_id, class, source?}` payload shape both v0.28's
  inbound A04 handler and v0.29's REST `create_encounter`
  emit.

### Changed — REST create_encounter

- `src/api/rest/handlers.rs::create_encounter` now wraps the
  encounter insert in a `DatabaseTransaction` that writes:
    1. `EncounterRepository::create` (unchanged).
    2. An `audit_log` row (`action = "create"`) inside the
       same transaction (previously written best-effort after
       the insert).
    3. An `outbox_events` row of type `EncounterRegistered`
       — but **only** for `Outpatient` or `Emergency` classes.
       Inpatient classes still flow through the
       `AdtService::admit` path, which emits its own
       `EncounterAdmitted` event; emitting both would
       double-publish.
- The encounter row returned to the REST caller is unchanged.

### Tests

- 2 new lib unit tests in `src/hl7v2/mapping.rs`
  (`test_encode_adt_a04_outpatient_pv1_class_o`,
  `test_encode_adt_a04_emergency_pv1_class_e`). Lib total
  480 → **482 passing**.
- No new integration test — the v0.28 inbound A04 test
  already exercises both directions of the contract through
  the `source: "hl7v2_a04"` boomerang-protection tag, same
  pattern as v0.25 and v0.27.

### Other

- Cargo: bumped to `0.29.0`.
- Quality gates: clippy + fmt clean; cargo build clean.

## [0.28.0] — 2026-05-29

The ADT^A04 (Register Outpatient) release. Adds the orthogonal
ambulatory-visit registration path that complements v0.4's A01
inpatient admit. EMRs can now push outpatient and ED registrations
to PAS without needing to allocate a bed first.

### Added — inbound handler

- `POST /api/hl7/v2/register` in `src/api/rest/handlers.rs`
  (`hl7_v2_register`). Parses MSH+PID+PV1, dedup-or-creates the
  patient from PID (same helper as A01 / A28), then opens an
  `Encounter` in `Arrived` status. Class is derived from PV1-2:
  `"E"` → `EncounterClass::Emergency`, anything else (incl. `"O"`,
  empty, unknown) → `EncounterClass::Outpatient`.
- The entire encounter write — repository insert + audit row +
  outbox event — runs in a single `DatabaseTransaction`. The
  audit row records `action = "register_via_hl7v2_a04"`. The
  outbox event is `EncounterRegistered` tagged
  `source: "hl7v2_a04"`.
- AA ACK body reports `encounter=<uuid>` and either
  `matched existing patient <id>` (dedup) or
  `created patient <id>` (fresh).
- 400 + AE on missing PV1; 400 + AE on wrong message type at
  this route (`expected ADT^A04, got X^Y`); 400 + AE on
  bare PID rejection.

### Added — MLLP routing

- `("ADT", "A04") => "/api/hl7/v2/register"` arm in
  `src/hl7v2/listener.rs::route_for_payload` + a matching unit
  test (`test_route_for_payload_a04`).

### Added — wiring

- `src/api/rest/routes.rs`: mounts `POST /api/hl7/v2/register`
  next to the existing `/admit` / `/transfer` / `/discharge`
  routes.
- `src/api/openapi.rs`: registers `hl7_v2_register` in the
  `ApiDoc` paths so Swagger UI picks it up.

### Tests

- 1 new lib test (`test_route_for_payload_a04`), bringing the
  PAS lib total to **480** (workspace lib **489 passing**).
- 1 new DB-bound integration test in `tests/hl7v2_test.rs`
  (`hl7v2_adt_a04_registers_outpatient_and_emergency`) covering:
  outpatient + emergency class derivation, repeat visit dedup
  (same MRN reuses the patient row but always opens a fresh
  encounter), missing PV1 → 400, wrong message type → 400.
  Integration total **55** test functions across **24** files.

### Other

- Cargo: bumped to `0.28.0`.
- Quality gates: clippy + fmt clean; cargo build clean.

## [0.27.0] — 2026-05-29

The outbound MFN^M05 release. Closes the v0.26 inbound loop the
same way v0.25 closed v0.24's: PAS-side bed roster changes now
flow back out as MFN^M05 MAD / MUP / MDL on the same MLLP peer,
with the boomerang protection in place so a bed update we just
received from the EMR doesn't get reflected straight back.

### Added — outbound publisher

- `Hl7v2MllpPublisher::emit_bed_mfn(&self, event,
  mfe_event_code)` private helper in
  `src/streaming/hl7v2_publisher.rs`. Resolves the bed via
  `BedRepository::find_by_id`, the parent room via
  `room::Entity::find_by_id`, then builds a single-item
  `MfnM05Item { event_code, bed_code, room_code: Some(room.code),
  name: Some(bed.name) }` and sends via `send_frame`.
- Three new match arms in `EventPublisher::publish`:
  - `BedCreated`  → `MFN^M05` with `MFE-1 = MAD`
  - `BedUpdated`  → `MFN^M05` with `MFE-1 = MUP`
  - `BedRetired`  → `MFN^M05` with `MFE-1 = MDL`
  All three short-circuit when `payload.source == "hl7v2_mfn_m05"`
  so v0.26 inbound events don't echo back to the same peer.

### Added — REST bed handlers gain audit + outbox writes

- `create_bed` in `src/api/rest/handlers.rs` now wraps its
  insert in a `DatabaseTransaction` that writes:
    1. `BedRepository::create` (unchanged).
    2. An `audit_log` row (`action = "create"`) with the standard
       `UserContext` from request headers.
    3. An `outbox_events` row of type `BedCreated` (no `source`
       tag — REST-driven, so the publisher will relay it).
- New `update_bed` handler at `PUT /api/beds/{id}` accepting a
  selective `UpdateBedRequest { room_id?, name?, code? }`.
  Same transactional shape (`audit_log` action `update`,
  `outbox_events` type `BedUpdated`). Mirrors v0.25's
  `update_practitioner` ergonomics.
- `ResourcesService::set_bed_status` in `src/resources/mod.rs`
  additionally emits `BedRetired` in the same transaction when
  the new status is `OutOfService` (alongside the pre-existing
  `BedStatusChanged`). REST status flips to `OutOfService` now
  produce an MFN^M05 MDL outbound.

### Added — route + OpenAPI

- `routes.rs`: `PUT /api/beds/{id}` mounted next to the existing
  `GET /api/beds/{id}` (single route chain on the same path).
- `openapi.rs`: `update_bed` registered alongside `create_bed`.

### Notes

- No new unit or integration tests. The v0.26 inbound test
  already exercises both directions of the contract via the
  `source: "hl7v2_mfn_m05"` boomerang-protection tag — same
  rationale as v0.25's outbound release.
- Cargo: bumped to `0.27.0`.
- Quality gates: 479 PAS lib tests + 9 patient-administration-system-frontend = **488
  passing**; clippy + fmt clean; build clean.

## [0.26.0] — 2026-05-28

The MFN^M05 (Master File Notification — Patient Location) release.
Mirrors the v0.24 staff-master-file surface for the bed roster: an
EMR can push bed adds, updates, and retirements to PAS in one
atomic transaction per message.

### Added — wire encoders + parser

- `MfnM05Item { event_code, bed_code, room_code, name }` +
  `MfnM05Message { master_file_id, items }` in
  `src/hl7v2/mapping.rs`.
- `parse_mfn_m05(&Message)` walks every `MFE` segment, pairs it
  with the immediately-following `LOC` segment, and returns one
  `MfnM05Item` per pair. Honored fields:
  - MFI-1 = `"LOC"` (other master file ids AE-ACK).
  - MFE-1 = `MAD` / `MUP` / `MDL`.
  - MFE-4 = optional fallback bed code.
  - LOC-1 is `PL`-typed: LOC-1.1 = parent room code, LOC-1.3 =
    bed code.
  - LOC-2 = free-form name / description.
- Required-field invariants per event: `MAD` requires room code
  (LOC-1.1) + name (LOC-2); `MUP` requires only the bed code;
  `MDL` requires only the bed code.
- `encode_mfn_m05(items, sending_app, receiving_app,
  message_control_id)` builds MSH + MFI + (MFE+LOC){1..N}.

### Added — REST handler

- `POST /api/hl7/v2/mfn-location` ingests MFN^M05. Pre-check
  pass returns `409 + AE` for MAD on duplicate bed code, `404 +
  AE` for MUP/MDL on unknown bed code or MAD on unknown room
  code — all before the DB transaction opens.
- All items in one message are applied in a single DB
  transaction. Per-item:
  - `MAD` resolves the parent `room_id` from LOC-1.1 then calls
    `BedRepository::create`.
  - `MUP` calls `BedRepository::update` (v0.26 — new method,
    full replace of `name` + `room_id` + `code`).
  - `MDL` calls `BedRepository::set_status_unchecked(_,
    OutOfService)`. The `unchecked` bypass mirrors the v0.4
    ADT^A13 path: bed state machine doesn't model "any status
    → OutOfService" for occupied beds, but MFN-driven
    retirement is a trusted-master-file operator bypass.
- Per-item audit (`action = "{mad|mup|mdl}_via_hl7v2_mfn_m05"`)
  and outbox (`BedCreated` / `BedUpdated` / `BedRetired` with
  `source: "hl7v2_mfn_m05"`) are written inside the transaction.

### Added — MLLP routing

- `route_for_payload` learns `("MFN", "M05") →
  /api/hl7/v2/mfn-location`.

### Added — repository

- `BedRepository::update` (v0.26 — new full-replace method) for
  the MUP path. Operational-status flips continue to go through
  the state-machine-protected `update_status`.

### Added — tests

- 7 new lib unit tests in `hl7v2::mapping`:
  - MAD happy path.
  - MAD requires room code.
  - MAD requires name.
  - MUP / MDL don't require name or room.
  - Reject non-LOC master file id.
  - MFE-4 falls back when LOC-1.3 is empty.
  - encode + parse round-trip.
- 1 new lib unit test in `hl7v2::listener` for M05 routing.
- 1 new DB-bound integration test
  (`hl7v2_mfn_m05_walks_add_update_delete_on_beds`): bootstrap
  facility + ward + room via REST, then MAD → MUP → MDL on the
  same bed; duplicate-MAD → 409; unknown-bed MUP → 404; orphan-
  room MAD → 404; 2-item MFN with duplicate second item rolls
  back the first.

### Quality gates

- `cargo build --workspace --all-targets` → clean.
- `cargo test --workspace --lib` → 479 PAS + 9 patient-administration-system-frontend = 488
  passing.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.

### Version

- `patient-administration-system` 0.25.0 → 0.26.0.

## [0.25.0] — 2026-05-28

The outbound MFN^M02 release. Closes the v0.24 loop: practitioner
roster changes that originate on the PAS side (REST + FHIR
`POST` / `PUT` / `DELETE` against `/api/practitioners` and
`/fhir/Practitioner`) now produce outbox events that the HL7 v2
publisher fans out as `MFN^M02` messages. Also adds the audit
trail that those handlers were silently missing.

### Added — audit + outbox on practitioner CRUD

REST `create_practitioner` / `update_practitioner` /
`delete_practitioner` were inserting / updating practitioner rows
directly via SeaORM `ActiveModel` with no `audit_log` row and no
`outbox_events` row. v0.25 wraps each in a `DatabaseTransaction`
that writes:

- The row change (insert / update).
- An `audit_log` entry with the standard `UserContext` (action =
  `create` / `update` / `deactivate`).
- An `outbox_events` row: `PractitionerCreated` /
  `PractitionerUpdated` / `PractitionerDeactivated` carrying just
  `{ practitioner_id }`. No `source` tag — that's what makes them
  eligible for outbound fan-out.

The FHIR `/fhir/Practitioner` POST / PUT / DELETE handlers gain
the same outbox write (still no audit on the FHIR side; that's a
broader gap not addressed by this release).

### Added — outbound publisher mappings

`Hl7v2MllpPublisher` learns three new event types:

- `PractitionerCreated → MFN^M02` with `MFE-1 = MAD`.
- `PractitionerUpdated → MFN^M02` with `MFE-1 = MUP`.
- `PractitionerDeactivated → MFN^M02` with `MFE-1 = MDL`.

All three are **source-gated on `hl7v2_mfn_m02`**: events that
came in via the v0.24 inbound MFN^M02 path are silently dropped on
the outbound side, so the EMR that just sent us an update doesn't
get the same update echoed straight back.

The outbound `primary_key` (MFE-4 + STF-1) is chosen as follows:

- If the practitioner row carries an `Identifier { type: Other,
  system: "urn:hl7v2:staff:id" }` — i.e. it originally came in
  via MFN — that value is reused. The downstream EMR sees the id
  it sent us, which keeps its dedup-on-staff-id clean.
- Otherwise the PAS UUID is used as the primary key. The receiver
  still gets a stable handle for subsequent `MUP` / `MDL`.

A small private helper `emit_practitioner_mfn(&event,
mfe_event_code)` factors out the shared `decode payload →
source-gate → load practitioner → encode → send_frame` flow.

### Quality gates

- `cargo build --workspace --all-targets` → clean.
- `cargo test --workspace --lib` → 471 PAS + 9 patient-administration-system-frontend = 480
  passing (no new lib tests in this release — the publisher path
  is exercised in production but the existing MFN inbound test
  also covers the inverse direction through its `source` tag).
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.

### Version

- `patient-administration-system` 0.24.0 → 0.25.0.

## [0.24.0] — 2026-05-28

The HL7 v2 MFN^M02 (Master File Notification — Staff) release.
Lets an EMR's provider directory drive the PAS practitioner roster:
inbound MFN^M02 messages can add, update, or soft-delete
practitioner rows in one atomic transaction per message.

### Added — wire encoders + parser

- `MfnM02Item { event_code, primary_key, name, gender, birth_date,
  active }` + `MfnM02Message { master_file_id, items }` in
  `src/hl7v2/mapping.rs`.
- `parse_mfn_m02(&Message) -> Result<MfnM02Message>` walks every
  `MFE` segment, pairs it with the immediately-following `STF`
  segment, and returns one `MfnM02Item` per pair. Errors name the
  1-based MFE index. Honored fields:
  - MFI-1 = `"PRA"` (other master file ids AE-ACK as unsupported).
  - MFE-1 = event code: `MAD` / `MUP` / `MDL`.
  - MFE-4 (with STF-1 fallback) = primary key (EMR's staff id).
  - STF-3 = staff name (XPN: `family^given^middle`).
  - STF-5 = sex (`M`/`F`/`O`/`U`).
  - STF-6 = birth date (`YYYYMMDD`).
  - STF-7 = active flag (`A`/`I`; defaults to `A`).
- `encode_mfn_m02(items, sending_app, receiving_app,
  message_control_id)` builds MSH + MFI + (MFE+STF){1..N}.

### Added — repository

- New `src/db/repositories/practitioner.rs` with
  `PractitionerRepository::{create, find_by_id,
  find_by_identifier_value, update, set_active}`. The
  `find_by_identifier_value` query mirrors
  `PatientRepository::find_by_identifier_value` (Postgres JSONB
  `@>` containment). The previous direct `practitioner::Entity`
  usage in `src/api/rest/handlers.rs` continues to work — the
  repo is additive, not a replacement.

### Added — REST inbound handler

- `POST /api/hl7/v2/mfn-staff` ingests MFN^M02. Pre-checks each
  MFE so duplicate-MAD returns 409 + AE cleanly (no partial DB
  work) and unknown-staff-id on MUP/MDL returns 404 + AE.
- All items in one message are applied in a single DB transaction
  (atomic per message — same contract as v0.20's multi-FT1 DFT).
  Atomicity is exercised by an integration test that intentionally
  builds a 2-item MFN whose second item duplicates an existing
  staff id; the first item must not persist when the second fails.
- Per-item the handler writes:
  - Audit (`action = "{mad|mup|mdl}_via_hl7v2_mfn_m02"`).
  - Outbox event (`PractitionerCreated` / `PractitionerUpdated` /
    `PractitionerDeactivated`, payload `{practitioner_id,
    primary_key, source: "hl7v2_mfn_m02"}`).
- The EMR's staff id is stored as `Identifier { type: Other,
  system: "urn:hl7v2:staff:id" }` on the practitioner row, so
  subsequent MUP / MDL messages can locate the same PAS row.
- MDL is a **soft delete** via `active = false` — practitioner
  rows are referenced by encounter / appointment / schedule and
  hard delete would orphan them.
- MSA-3 diag: `practitioner=<uuid> event=<MAD/MUP/MDL>` for
  single-item messages, `staff_records_applied=<N>` for multi-
  item messages.

### Added — MLLP routing

- `route_for_payload` learns `("MFN", "M02") →
  /api/hl7/v2/mfn-staff`.

### Added — tests

- 8 new lib unit tests in `hl7v2::mapping`:
  - Happy-path MAD parse with all STF fields populated.
  - Walk multiple MFE+STF pairs in one message (MAD + MUP + MDL).
  - Reject non-PRA master file id.
  - Reject unknown event code.
  - Reject MFE without a following STF.
  - Reject missing family name.
  - MFE-4 falls back to STF-1 when MFE-4 is empty.
  - encode + parse round-trip preserves all fields.
- 1 new lib unit test for the listener routing
  (`test_route_for_payload_mfn_m02`).
- 1 new DB-bound integration test
  (`hl7v2_mfn_m02_walks_add_update_delete`): MAD then MUP then
  MDL on the same staff id; duplicate-MAD → 409; unknown-staff
  MUP → 404; 2-item MFN with a duplicate second item rolls back
  the first.

### Quality gates

- `cargo build --workspace --all-targets` → clean.
- `cargo test --workspace --lib` → 471 PAS + 9 patient-administration-system-frontend = 480
  passing.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.

### Version

- `patient-administration-system` 0.23.0 → 0.24.0.

## [0.23.0] — 2026-05-28

The Bundle-write entry extension release. `POST /fhir` Bundle
entries now accept `Practitioner`, `Schedule`, and `Slot` resources
in addition to the existing `Patient` / `Encounter` / `Appointment`
/ `Coverage` types. Builds on v0.21 (which added the direct
`POST /fhir/<Type>` endpoints for those three) so a single
transaction Bundle can provision a new practitioner, their schedule,
and a slot in one atomic write — useful for new-clinic onboarding
flows.

### Added — Bundle dispatch

- `ResourceKind` in `src/api/fhir/handlers.rs` gains three new
  variants: `Practitioner`, `Schedule`, `Slot`.
- `validate_and_route` accepts the matching `resourceType` strings.
- `process_batch_bundle` and `process_transaction_bundle` dispatch
  loops gain three new arms.
- New helpers `create_practitioner_in_db`, `create_schedule_in_db`,
  `create_slot_in_db` symmetric with the existing
  `create_patient_in_db` / `_encounter_in_db` / `_appointment_in_db`
  / `_coverage_in_db` shape. Each returns the standard
  `EntrySuccess { response, indexable_patient: None }` — none of
  the three resources need Tantivy re-indexing.
- OpenAPI `request_body` description on `POST /fhir` updated to
  list the new entry types.

Transaction semantics are unchanged: the whole bundle still runs
in one `sea_orm::DatabaseTransaction` and any failing entry rolls
the whole bundle back. A malformed Slot status in entry 1 will
unwind a successfully-built Schedule in entry 0.

### Added — tests

- 1 new DB-bound integration test
  (`fhir_bundle_creates_practitioner_schedule_slot_entries`):
  - Batch bundle creates a Practitioner.
  - Transaction bundle creates a Schedule referencing the
    Practitioner.
  - Transaction bundle creates a Slot referencing the Schedule;
    the result is round-trip-readable via `GET /fhir/Slot/{id}`.
  - Atomicity: a 2-entry transaction with a malformed Slot
    status (entry 1) rolls back the Schedule (entry 0); the
    `OperationOutcome` `diagnostics` names "entry 1".

### Quality gates

- `cargo build --workspace --all-targets` → clean.
- `cargo test --workspace --lib` → 460 PAS + 9 patient-administration-system-frontend = 469
  passing (no new lib tests — the Bundle dispatch is exercised end-
  to-end via the integration test).
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.

### Version

- `patient-administration-system` 0.22.0 → 0.23.0.

## [0.22.0] — 2026-05-27

Two related touches to the HL7 v2 path:
- Repair the pre-existing `tests/hl7v2_outbound_test.rs` regression
  (two failing tests had been red since at least v0.13).
- Add PID-29 (Patient Death Date and Time) and PID-30 (Patient Death
  Indicator) round-trip to the PID encoder + decoder so an inbound
  ADT message can mark a patient deceased and an outbound message
  can carry the deceased state forward.

### Fixed — pre-existing test regression

- `CreateFacilityRequest.address` is now optional (defaults to an
  all-empty `Address`). The strict-required shape was added at
  some point after `tests/hl7v2_outbound_test.rs` was written; the
  test (and several others) had been silently broken because the
  `/api/facilities` POST returned 422 with "missing field
  `address`" before exercising any HL7 v2 logic.
- `CreateWardRequest.capacity` is now optional with default `0`
  (same rationale).
- `Address` now derives `Default` so the optional field can fall
  back to an all-`None` value when omitted.
- `tests/hl7v2_outbound_test.rs` fixture cleanups: the test posted
  `"gender": "Female"` and `"Other"` (uppercase), but the
  serde-derived `Gender` enum is `rename_all = "lowercase"`.
  Switched both to lowercase.
- Both outbound tests now pass against a real Postgres:
  `outbound_publisher_emits_adt_a01_to_fake_emr` and
  `outbound_publisher_reports_err_on_ae_ack`.

### Added — PID-29 / PID-30 deceased round-trip

- `patient_from_pid` reads PID-29 (`YYYYMMDDHHMMSS` → `Patient.
  deceased_datetime`) and PID-30 (`Y`/`N` → `Patient.deceased`).
  When PID-30 is absent or unrecognized but PID-29 is present, the
  decoder **infers** `deceased = true`.
- `pid_from_patient` emits PID-29 + PID-30 when the patient is
  marked deceased (`Patient.deceased == true` OR
  `deceased_datetime.is_some()`). For living patients the encoder
  stays compact at 14 fields — no change to the existing wire shape.
- 6 new lib unit tests cover the wire surface:
  - Parse PID with PID-29 + PID-30 = Y → both populated.
  - Parse PID with PID-29 only → `deceased = true` inferred.
  - Parse PID with PID-30 = N and no PID-29 → both null.
  - Encode a deceased patient → PID-29 + PID-30 land at the right
    indices.
  - Encode a living patient → PID-29 + PID-30 stay empty (no
    growth of the segment width).
  - Full round-trip via a parseable wire message preserves both
    the indicator and the datetime.

### Quality gates

- `cargo build --workspace --all-targets` → clean.
- `cargo test --workspace --lib` → 460 PAS + 9 patient-administration-system-frontend = 469
  passing.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.

### Version

- `patient-administration-system` 0.21.0 → 0.22.0.

## [0.21.0] — 2026-05-26

The FHIR write-surface completion release. `Practitioner`, `Schedule`,
and `Slot` were read-only before this release; v0.21 adds the matching
`POST` / `PUT` / `DELETE` endpoints so FHIR clients can manage those
resources without dropping to the `/api/*` REST surface.

Also fixes a pre-existing bug in `PUT /fhir/Patient/{id}` where a
client-supplied non-UUID body id (a FHIR-conformant placeholder
shape) was rejected before the URL id could override it.

### Added — repositories

- `ScheduleRepository::update(conn, &Schedule)` and
  `ScheduleRepository::delete(conn, id)`.
- `SlotRepository::update(conn, &Slot)` and
  `SlotRepository::delete(conn, id)`.
- Schedule and Slot deletes are **hard delete** (invariant §5.3
  restricts soft-delete to patients / encounters / appointments).

### Added — FHIR handlers

- `POST /fhir/Practitioner`, `PUT /fhir/Practitioner/{id}`,
  `DELETE /fhir/Practitioner/{id}`. Server assigns a fresh UUID on
  POST. PUT preserves id + created_at. **DELETE flips `active =
  false`** rather than hard-deleting — practitioner rows are
  referenced by encounter / appointment / schedule, and an orphan
  reference would silently break those join paths.
- `POST /fhir/Schedule`, `PUT /fhir/Schedule/{id}`,
  `DELETE /fhir/Schedule/{id}`. Hard delete.
- `POST /fhir/Slot`, `PUT /fhir/Slot/{id}`, `DELETE /fhir/Slot/{id}`.
  Hard delete. Slot `PUT` is an operator-driven override: status
  transitions are NOT validated against the `SlotStatus` state
  machine here — use the booking REST surface
  (`POST /api/slots/{id}/book`, …) for state-machine-protected
  status flips.

### Fixed — PUT id handling

`PUT /fhir/Patient/{id}` previously failed `400 + AE` when the
request body carried a non-UUID `id` field (e.g. a placeholder like
`"ignored-client-id"`) because `FhirPatient::into_domain()` strict-
parses the id before the handler can override it with the URL id.
Per FHIR semantics, the URL id is canonical and a client-supplied
body id should be ignored. v0.21 strips `body.id` before parsing on
all four PUT paths: Patient, Practitioner, Schedule, Slot.

### Added — routes + OpenAPI

- `fhir_router` registers nine new endpoints. `Practitioner`,
  `Schedule`, and `Slot` each gain `POST /fhir/<Type>` and
  `PUT/DELETE /fhir/<Type>/{id}` (the existing `GET /fhir/<Type>/{id}`
  is preserved, now sharing the parameterized route with PUT +
  DELETE).
- Compile-time-known handler list is unchanged at the `ApiDoc`
  level — utoipa macros on the handlers themselves were not added
  in this release (the FHIR handlers don't use `#[utoipa::path]`
  on `create_patient` either; the FHIR surface is documented in
  README + spec.md rather than in the OpenAPI doc).

### Added — tests

- 2 new DB-bound integration tests in `tests/fhir_write_test.rs`:
  - `fhir_practitioner_crud` — POST + GET + PUT + DELETE
    (`active = false` after delete; row still readable). 404 on
    PUT / DELETE of an unknown id.
  - `fhir_schedule_and_slot_crud` — chained: POST a Practitioner,
    POST a Schedule referencing it, PUT to change serviceType,
    POST a Slot referencing the Schedule, PUT to change status,
    DELETE Slot (then GET → 404, DELETE again → 404), DELETE
    Schedule.
- Pre-existing `fhir_patient_crud` now passes too (the PUT-id fix
  above unblocks it).

### Quality gates

- `cargo build --workspace --all-targets` → clean.
- `cargo test --workspace --lib` → 454 PAS + 9 patient-administration-system-frontend = 463
  passing.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.

### Version

- `patient-administration-system` 0.20.0 → 0.21.0.

## [0.20.0] — 2026-05-26

The multi-FT1 DFT^P03 release. A single DFT message can now carry many
charges (one FT1 segment each) and PAS posts them all in one DB
transaction — matching how real-world EMR billing batches actually
behave (a visit typically produces 5–20 line items in a single
financial message).

### Changed — DFT^P03 wire shape

- `DftP03Message` now has `{ patient: Patient, items: Vec<DftP03Item> }`
  instead of one set of per-charge fields. The new `DftP03Item`
  carries `transaction_type`, `code`, `description`, `amount`,
  `currency`, `posted_at`.
- `parse_dft_p03` walks **every** `FT1` segment in the message via
  `Message::all_segments("FT1")`. The parser rejects when no FT1 is
  present.
- Per-FT1 error messages now name the 1-based segment index, e.g.
  `FT1[2]-11.1 not a decimal amount: "not-a-number"` — so a sender
  with a bad row in the middle of a 12-line batch can pinpoint it.
- `encode_dft_p03` is unchanged on the wire — it still emits one
  FT1 per call. Outbound delivery is one DFT per `ChargePosted`
  outbox event, which keeps the publisher loop simple. Senders
  that want to bundle outbound charges into a single multi-FT1
  message can do so via the FHS/BHS batch envelope from v0.6.

### Changed — `hl7_v2_dft` handler

- All charges from one message are now posted in a single
  `state.db.transaction(...)` block. Any per-charge DB failure
  rolls the whole transaction back; nothing partially lands.
  Audit + outbox writes also run inside the same transaction so a
  half-applied DFT can't leave audit / outbox / charge rows out of
  sync.
- All FT1 segments must share the same `FT1-11.2` currency.
  Mixing currencies in a single DFT — which would force PAS to
  split across multiple accounts — is rejected `400 + AE` rather
  than guessed.
- MSA-3 diagnostic now adapts to the count:
  - Single FT1 (the v0.19 default): `charge=<uuid> account=<uuid>`
    (unchanged for backwards compatibility — existing senders that
    parse the AA's MSA-3 to learn the assigned charge id keep
    working).
  - Multi FT1: `charges_posted=<N> account=<uuid>` (the assigned
    individual charge UUIDs aren't enumerated inline — pull them
    from a follow-up `GET /api/accounts/{id}/charges` if needed).

### Added — tests

- 2 new lib unit tests in `hl7v2::mapping`:
  - `test_parse_dft_p03_walks_multiple_ft1_segments` — three FT1
    segments in one message all parse, codes preserved in order.
  - `test_parse_dft_p03_reports_per_item_field_index_on_error` —
    error names the 1-based FT1 index that failed.
- 1 new DB-bound integration test
  (`hl7v2_dft_p03_posts_multiple_ft1_in_one_transaction`): three
  FT1 happy path; mixed currencies → 400; **atomicity** — a bad
  second FT1 must not leave the first one persisted.
- All v0.19 single-FT1 unit tests + integration test still pass
  unchanged (the wire is backwards-compatible).

### Quality gates

- `cargo build --workspace --all-targets` → clean.
- `cargo test --workspace --lib` → 454 PAS + 9 patient-administration-system-frontend = 463
  passing.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.

### Version

- `patient-administration-system` 0.19.0 → 0.20.0.

## [0.19.0] — 2026-05-26

The HL7 v2 DFT^P03 (post detail financial transaction) release.
Couples the wire protocol to the existing PAS billing surface
(accounts, charges) so EMRs can push charge data into PAS as it
happens, and PAS-originated charges can flow back out the same way.

Also fixes a long-standing schema-vs-entity type mismatch in the
billing tables (see "Schema fix" below) that was silently failing
for anyone exercising `POST /api/charges` or
`BillingService::post_charge` against a real Postgres.

### Added — wire encoders + parser

- New `DftP03Message` struct + `parse_dft_p03(&Message) ->
  Result<DftP03Message>` in `src/hl7v2/mapping.rs`. Honored FT1
  fields:
  - FT1-4 (Transaction Date, `YYYYMMDDHHMMSS`) → `Charge.posted_at`.
    Defaults to "now" when missing.
  - FT1-6 (Transaction Type): `CG` only. `PY` / `AJ` AE-ACK as
    unsupported (need different domain logic). Empty defaults to
    `CG`.
  - FT1-7.1 → `Charge.code`. Required.
  - FT1-8 → `Charge.description`. Required.
  - FT1-11.1 → decimal amount, FT1-11.2 → ISO 4217 currency. Both
    required; currency uppercased and validated for shape (3
    uppercase ASCII letters).
- `encode_dft_p03(patient, charge, sending_app, receiving_app,
  message_control_id)` builds MSH + EVN + PID + FT1. FT1-2 carries
  the PAS charge UUID so the receiver can dedupe on it.

### Added — REST inbound handler

- `POST /api/hl7/v2/dft` ingests DFT^P03. Parses patient from PID
  (dedup-on-MRN like A01/A28; creates the row when unknown),
  resolves an open billing account for the patient via the existing
  `find_open_account_for_patient` repo, and **auto-creates one** in
  the FT1-11.2 currency when none exists. Posts the charge via
  `BillingRepository::create_charge`. Writes audit (`action =
  "post_via_hl7v2_p03"`) and outbox (`event_type = "ChargePosted"`,
  payload includes `source: "hl7v2_p03"` so the outbound publisher
  can skip the boomerang). AA ACK's MSA-3 reports `charge=<uuid>
  account=<uuid>`.

### Added — MLLP routing

- `route_for_payload` learns `("DFT", "P03") → /api/hl7/v2/dft`.

### Added — outbound publisher mapping

- `Hl7v2MllpPublisher` learns `ChargePosted → DFT^P03`. Skipped
  when payload `source == "hl7v2_p03"`. Resolves
  `charge → account → patient` via new `find_charge_by_id` and
  `find_account_by_id` repository methods (the outbox payload's
  `patient_id` is honored when present — the inbound handler
  includes it; REST `post_charge` doesn't yet, so the publisher
  falls back to the account lookup).

### Added — repository helpers

- `BillingRepository::find_charge_by_id(conn, id)` and
  `BillingRepository::find_account_by_id(conn, id)`. Both return
  `Result<Option<...>>`. Used by the new outbound mapping but
  generally useful for any consumer that needs to hop from a charge
  back through to the patient identity.

### Schema fix (bug)

The `charges.amount_value`, `payments.amount_value`, and
`invoices.total_value` columns were defined in the v0.1 migration
as `decimal(20, 4)` but the corresponding SeaORM entities declared
them as `String`. Postgres rejects an implicit `text → numeric`
conversion, so any actual write to those columns failed at runtime
with `column "amount_value" is of type numeric but expression is
of type text`. This silently broke `POST /api/charges` and
`POST /api/payments` end-to-end against a real Postgres (both the
v0.1 billing surface and the `billing_flow_test.rs` integration
test were affected).

v0.19 changes the migration to `text()` on all three columns so the
column type matches the entity. `rust_decimal::Decimal` is still
the in-memory representation; the repo serializes via
`Decimal::to_string` on write and `Decimal::from_str` on read,
preserving precision. No data migration is needed because the bug
prevented any rows from being written in the first place.

### Added — tests

- 10 new lib unit tests in `hl7v2::mapping` (parse happy path,
  defaults, unsupported transaction type, missing code, missing
  description, bad amount, bad currency, missing FT1, encode
  shape, encode + parse round-trip).
- 1 new lib unit test in `hl7v2::listener` for `DFT^P03` routing.
- 1 new DB-bound integration test
  (`hl7v2_dft_p03_posts_charge_and_creates_account`): happy path
  (account auto-creation + AA + MSA-3 carries the assigned ids),
  account reuse for the second DFT, PY → 400, missing FT1 → 400,
  bad currency → 400, wrong message type → 400.
- The pre-existing `tests/billing_flow_test.rs` integration test
  now passes (the schema fix above unblocks it).

### Quality gates

- `cargo build --workspace --all-targets` → clean.
- `cargo test --workspace --lib` → 452 PAS + 9 patient-administration-system-frontend = 461
  passing.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.

### Version

- `patient-administration-system` 0.18.0 → 0.19.0.

## [0.18.0] — 2026-05-26

The HL7 v2 ADT^A40 (merge patient) release. PAS now ingests and emits
patient-merge notifications over the same HL7 v2 surface as the rest
of the ADT lifecycle, pairing the wire protocol with the existing
v0.11 merge-tombstone domain logic.

### Added — wire encoders + parser

- `encode_adt_a40(survivor, source, sending_app, receiving_app,
  message_control_id)` in `src/hl7v2/mapping.rs`. PID describes the
  survivor (kept identity); the new MRG segment carries the source
  patient's MRN in MRG-1 (`<value>^^^<facility>^MR`, mirroring the
  PID-3 shape). MRG-7 (Prior Patient Name) is intentionally left
  empty — the PAS row preserves the original name in the DB.
- `parse_merge_source_mrn(&Message) -> Result<String>` extracts the
  MRG-1.1 source MRN. Returns `Error::Validation` when the MRG
  segment is missing or MRG-1.1 is empty.
- Private `mrg_from_patient` helper used by `encode_adt_a40`.

### Added — REST inbound handler

- `POST /api/hl7/v2/merge` — ingest ADT^A40. Parses the PID for the
  survivor (dedup-on-MRN like A01/A28; creates the row when
  unknown). Looks up the source by MRG-1.1 MRN against the existing
  `PatientRepository::find_by_identifier_value` index. Applies the
  same merge logic as `POST /api/patients/{id}/merge-into/
  {target_id}`: one DB transaction wraps `set_replaced_by` + audit
  (`action = "merge_via_hl7v2_a40"`) + outbox (`event_type =
  "PatientMerged"`, payload includes `source: "hl7v2_a40"`).
  Best-effort drops the source from Tantivy post-commit so it stops
  appearing in search results.
- Returns `400 + AE` when MRG / MRG-1.1 is missing, `404 + AE` when
  no PAS patient matches the source MRN, `409 + AE` when the source
  is already merged (already a tombstone) or when the source MRN
  resolves to the same row as PID-3 (self-merge).

### Added — MLLP routing

- `route_for_payload` learns `("ADT", "A40") → /api/hl7/v2/merge`.

### Added — outbound publisher mapping

- `Hl7v2MllpPublisher` learns `PatientMerged → ADT^A40`. Skipped
  when payload `source == "hl7v2_a40"` (boomerang protection).
  REST-driven merges (no source tag) DO relay so a downstream EMR
  learns about PAS-initiated merges. Loads both the survivor
  (`payload.target_id`) and source (`payload.source_id`) patients
  by id to populate PID + MRG.

### Added — tests

- 5 new lib unit tests in `hl7v2::mapping`:
  - `test_encode_adt_a40_includes_pid_and_mrg` — assert MSH + EVN +
    PID (survivor's MRN) + MRG (source's MRN, `<value>^^^<fac>^MR`).
  - `test_parse_merge_source_mrn_extracts_mrg_1_1`.
  - `test_parse_merge_source_mrn_rejects_missing_mrg`.
  - `test_parse_merge_source_mrn_rejects_empty_mrg1`.
  - `test_encode_adt_a40_round_trip_through_parse_helpers`.
- 1 new lib unit test in `hl7v2::listener`
  (`test_route_for_payload_a40` → /merge).
- 1 new DB-bound integration test
  (`hl7v2_adt_a40_merges_source_into_survivor`): bootstrap two
  patients via A28 → merge → assert AA + diagnostic shape; re-merge
  → 409 (already a tombstone); unknown source MRN → 404; missing
  MRG segment → 400; self-merge → 409; wrong message type at
  `/merge` → 400.

### Quality gates

- `cargo build --workspace --all-targets` → clean.
- `cargo test --workspace --lib` → 441 PAS + 9 patient-administration-system-frontend = 450
  passing.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.

### Version

- `patient-administration-system` 0.17.0 → 0.18.0.

## [0.17.0] — 2026-05-25

The SIU lifecycle completion release. Adds inbound + outbound
SIU^S13 (notification of rescheduling) and SIU^S14 (notification of
modification) alongside the v0.16 S12/S15 surface. The four-trigger
set (S12/S13/S14/S15) covers the full book → reschedule → modify →
cancel lifecycle that real-world EMRs need.

### Added — wire encoders + parser

- `encode_siu_s13(patient, appointment_id, placer_id, start, end,
  reason, sending_app, receiving_app, message_control_id)` — same
  shape as S12; the SCH-11 + SCH-9 carry the *new* time window.
- `encode_siu_s14(patient, appointment_id, placer_id, reason,
  sending_app, receiving_app, message_control_id)` — same shape as
  S15 but carries SCH-7 reason; SCH-9/10/11 left empty.
- `parse_siu` extended to accept `S13` and `S14` triggers:
  - `S13` requires SCH-11 (new start) and SCH-2 (filler id).
  - `S14` requires SCH-2 (filler id); SCH-11 ignored.
  - Unknown triggers (S17/S26/…) continue to AE-ACK as unsupported.

### Added — REST inbound handlers

- `POST /api/hl7/v2/schedule-reschedule` — ingest SIU^S13. Looks up
  the appointment by SCH-2 filler UUID, validates non-terminal
  status, runs the **overlap-excluding** check (the row's own time
  window doesn't flag itself), and updates `start_datetime` +
  `end_datetime` via `AppointmentRepository::update`. Writes audit
  (`reschedule_via_hl7v2_s13`) + outbox (`AppointmentRescheduled`
  with `source: "hl7v2_s13"`). Returns:
  - `200 + AA` on success.
  - `400 + AE` when SCH-2 / SCH-11 are missing or malformed.
  - `404 + AE` when no PAS appointment matches.
  - `409 + AE` for terminal-status appointments or new-window
    collisions with other live appointments.
- `POST /api/hl7/v2/schedule-modify` — ingest SIU^S14. Looks up by
  SCH-2, validates non-terminal status, updates `reason` from SCH-7
  (no time changes). Writes audit (`modify_via_hl7v2_s14`) + outbox
  (`AppointmentModified` with `source: "hl7v2_s14"`).

### Added — overlap helper

- `AppointmentRepository::find_overlapping_for_patient_excluding(
  conn, patient_id, start, end, exclude_id)` — mirrors the existing
  `find_overlapping_for_patient` but skips the given row. Used by
  the S13 reschedule path so a row's own current window doesn't
  flag itself.

### Added — MLLP routing

- `route_for_payload` learns `("SIU", "S13") → /schedule-reschedule`
  and `("SIU", "S14") → /schedule-modify`. Unknown SIU triggers
  (S17/S26/…) continue to fall through to `/patient`.

### Added — outbound publisher mappings

- `Hl7v2MllpPublisher` learns two more event types:
  - `AppointmentRescheduled → SIU^S13`. Skipped when payload
    `source == "hl7v2_s13"` (boomerang protection).
  - `AppointmentModified → SIU^S14`. Same gate on `hl7v2_s14`.

### Added — tests

- 7 new lib unit tests in `hl7v2::mapping` (S13 missing-filler,
  S13 happy-path with new time window, S14 missing-filler, S14
  reason-only path, unknown-trigger rejection, encode S13, encode
  S14 + round-trip).
- 2 new lib unit tests in `hl7v2::listener` (S13 and S14 routing).
- 1 new DB-bound integration test
  (`hl7v2_siu_s13_reschedules_and_s14_modifies`): book → reschedule
  → modify → cancel walks the full lifecycle through all four SIU
  endpoints, with overlap conflict (409), terminal-status conflict
  (409), unknown filler (404), missing SCH-11 (400), missing SCH-2
  (400), and wrong-trigger-at-endpoint (400) paths.

### Quality gates

- `cargo build --workspace --all-targets` → clean.
- `cargo test --workspace --lib` → 435 PAS + 9 patient-administration-system-frontend = 444
  passing.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.

### Version

- `patient-administration-system` 0.16.0 → 0.17.0.

## [0.16.0] — 2026-05-25

The SIU scheduling interop release. Adds HL7 v2 SIU^S12 (notification
of new appointment) and SIU^S15 (notification of cancellation) on
both inbound (HTTP + MLLP) and outbound (`Hl7v2MllpPublisher`) sides
of the wire. Completes the scheduling story alongside the existing
ADT lifecycle (A01/A02/A03/A08/A11/A13/A28).

### Added — wire encoders + parser

- New `SiuMessage` struct + `parse_siu(&Message) -> Result<SiuMessage>`
  in `src/hl7v2/mapping.rs`. Reads MSH + SCH + PID:
  - SCH-1 → placer appointment id (preserved verbatim).
  - SCH-2 → filler appointment id (PAS UUID for inbound S15).
  - SCH-7 → appointment reason text → `Appointment.reason`.
  - SCH-9 / SCH-10 → duration value + units (`min`). When missing or
    non-numeric, defaults to 30 minutes for S12.
  - SCH-11 → appointment start datetime (`YYYYMMDDHHMMSS`,
    `YYYYMMDDHHMM`, or `YYYYMMDD`; trailing `±HHMM` offsets stripped
    and treated as UTC).
- `encode_siu_s12(patient, appointment_id, placer_id, start, end,
  reason, sending_app, receiving_app, message_control_id)`.
- `encode_siu_s15(patient, appointment_id, placer_id, reason,
  sending_app, receiving_app, message_control_id)`.
- New helper `parse_v2_datetime` next to the existing `parse_v2_date`
  so any module reading HL7 v2 datetimes can share one parser.

### Added — REST inbound handlers

- `POST /api/hl7/v2/schedule-book` — ingest SIU^S12. Parses patient
  from PID (dedup-on-MRN like A01/A28), checks for overlapping
  appointments via `AppointmentRepository::find_overlapping_for_
  patient`, persists via `AppointmentRepository::create`, writes an
  audit row (`action = "book_via_hl7v2_s12"`) and an outbox event
  (`event_type = "AppointmentBooked"`, payload includes
  `source: "hl7v2_s12"`). AA ACK's MSA-3 diagnostic carries the
  assigned filler id so the sender can record it.
- `POST /api/hl7/v2/schedule-cancel` — ingest SIU^S15. Looks up the
  target appointment by `SCH-2` (PAS UUID), flips its status to
  `Cancelled` via `AppointmentRepository::set_status_and_reason`,
  writes audit + outbox (`source: "hl7v2_s15"`). Returns:
  - `200 + AA` on success.
  - `400 + AE` when SCH-2 is missing or not a UUID.
  - `404 + AE` when no PAS appointment matches.
  - `409 + AE` when the appointment is already in a terminal status
    (`Cancelled`, `Fulfilled`, `NoShow`).

### Added — MLLP routing

- `route_for_payload` in `src/hl7v2/listener.rs` learns two new
  branches: `("SIU", "S12") → /api/hl7/v2/schedule-book` and
  `("SIU", "S15") → /api/hl7/v2/schedule-cancel`. Unknown SIU
  triggers (S13, S14, S17, S26) fall through to `/patient` exactly
  like unknown ADT events, which AE-ACKs them as unsupported.

### Added — outbound publisher mappings

- `Hl7v2MllpPublisher` learns two new event types in addition to
  the existing six:
  - `AppointmentBooked → SIU^S12`. Skipped when payload carries
    `source: "hl7v2_s12"` (boomerang protection — don't echo back
    appointments we just received from the EMR).
  - `AppointmentCancelled → SIU^S15`. Same boomerang rule with
    `source: "hl7v2_s15"`.
- REST / FHIR / `SchedulingService::book_slot` paths produce
  outbox events with no `source` tag and therefore DO get relayed,
  which is the desired behavior — the EMR learns about PAS-side
  bookings via SIU^S12.

### Added — tests

- 15 new lib unit tests: 12 in `hl7v2::mapping` (encode + parse
  round-trip for S12/S15, MSH/SCH/PID inspection, missing SCH-11,
  unknown triggers, datetime parsing with and without timezone
  offsets); 3 in `hl7v2::listener` (SIU routing rows + the
  unknown-trigger fallback).
- 1 new DB-bound integration test
  (`hl7v2_siu_s12_books_appointment_and_s15_cancels_it`) walks the
  full lifecycle through `POST /api/hl7/v2/schedule-book` and
  `POST /api/hl7/v2/schedule-cancel`: happy path AA, overlap → 409,
  re-cancel → 409, unknown filler → 404, non-UUID filler → 400,
  missing filler → 400, wrong message type at /schedule-book → 400.

### Quality gates

- `cargo build --workspace --all-targets` → clean.
- `cargo test --workspace --lib` → 426 PAS + 9 patient-administration-system-frontend = 435
  passing.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.

### Version

- `patient-administration-system` 0.15.0 → 0.16.0.

## [0.15.0] — 2026-05-25

The multinational national-identifier release. The PAS now carries
typed `Identifier` factories and per-country format validators for the
five national healthcare-identifier schemes the workspace cares about
beyond the existing UK NHS Number, plus the per-type dispatch helper
`validate_identifier` and a `validate_patient` hook that calls it for
every entry in `Patient.identifiers`.

### Added — `IdentifierType` variants

- `IdentifierType::NIR` — France Numéro d'Identification au Répertoire
  (a.k.a. INSEE).
- `IdentifierType::TSI` — España Tarjeta Sanitaria Individual / SNS CIP.
- `IdentifierType::IHI` — Ireland Individual Health Identifier.
- `IdentifierType::HCN` — Northern Ireland Health & Care Number.
- All variants serialize UPPERCASE (`"NIR"`, `"TSI"`, `"IHI"`, `"HCN"`),
  same as the existing `"NHS"` / `"SSN"` / `"MRN"` codes. Existing wire
  payloads (FHIR Bundle, bulk JSON / XML / TSV / CSV, REST) are
  unaffected; the new variants ride on the same `Identifier` shape.

### Added — typed factories

- `Identifier::nir(value)` → `urn:oid:1.2.250.1.213.1.4.8`
- `Identifier::tsi(value)` → `urn:oid:2.16.724.4.40`
- `Identifier::ihi(value)` → `https://fhir.hl7.ie/Id/individual-health-identifier`
- `Identifier::hcn(value)` → `https://fhir.hscni.net/Id/hcn`
- Exported system-URI constants alongside the existing
  `NHS_SYSTEM_URI` / `SSN_SYSTEM_URI`:
  `NIR_SYSTEM_URI`, `TSI_SYSTEM_URI`, `IHI_SYSTEM_URI`, `HCN_SYSTEM_URI`.

### Added — format validators (`src/validation/`)

- `validate_nhs_number` — 10 digits + Modulus 11 check digit on the
  last; whitespace and hyphens stripped (`"943 476 5919"` and
  `"943-476-5919"` validate identically to `"9434765919"`). Computed
  check `10` is rejected (such NHS numbers were never issued).
- `validate_nir` — 15 chars: sex(1) + year(2) + month(2) + dept(2,
  digits or Corsica `2A`/`2B`) + commune(3) + order(3) + control(2);
  control = `97 − (body mod 97)`, with Corsica `2A → 19` /
  `2B → 18` substitution before the modulus.
- `validate_tsi` — 1–20 ASCII alphanumeric (per-region issuance,
  envelope check only).
- `validate_ihi` — exactly 7 digits (HSE allocates randomly; no
  documented public checksum).
- `validate_hcn` — exactly 10 digits (modern HSC allocation; no
  documented public checksum).
- `validate_identifier(&Identifier)` — dispatch helper that routes by
  `IdentifierType`. `MRN` / `SSN` / `DL` / `Passport` / `Other` pass
  with no per-type check (their formats vary by issuing authority).

### Changed — `validate_patient`

- Now calls `validate_identifier` over every entry in
  `Patient.identifiers`. An invalid national-identifier value surfaces
  as `Error::Validation` → `400 VALIDATION` on REST, `400 invalid` on
  FHIR. Existing patients that carry only locally-issued `MRN`s or
  unrestricted types are unaffected.

### Added — tests

- 30 new lib unit tests:
  - 6 in `src/models/identifier.rs` — system-URI assertions for `NIR`,
    `TSI`, `IHI`, `HCN` factories; serde UPPERCASE roundtrip for the
    four new variants.
  - 24 in `src/validation/mod.rs` — NHS Mod 11 valid / wrong length /
    non-digit / bad check digit; NIR metropolitan + Corsica `2A` +
    Corsica `2B` valid; NIR wrong length / bad sex / bad control / bad
    department; TSI valid alphanumeric / empty / oversize / punctuated;
    IHI valid / wrong length / non-digit; HCN valid / wrong length /
    non-digit; `validate_identifier` dispatch; `validate_patient`
    accepts a five-country mix and rejects an invalid one.
- New PAS lib test count: 411 (was 381 at v0.14.0).

### Version

- `patient-administration-system` 0.14.0 → 0.15.0.

## [0.14.0] — 2026-05-25

The outbox webhook release. Adds an HTTP webhook implementation of
`EventPublisher` and a fan-out `CompositePublisher` so the existing
HL7 v2 MLLP peer and a webhook subscriber can both be active at the
same time. Webhook receivers get the durable, retryable delivery
guarantees that the v0.5 dead-letter machinery already provided to
the HL7 v2 path.

### Added — `WebhookEventPublisher`

- New module `src/streaming/webhook.rs`. POSTs each `DomainEvent` as
  JSON to `PAS_WEBHOOK_URL` with:
  - `Content-Type: application/json`
  - `X-PAS-Event-Id: <uuid>`
  - `X-PAS-Event-Type: <e.g. EncounterAdmitted>`
  - `X-PAS-Signature: sha256=<hex>` — HMAC-SHA256 of the raw body
    keyed by `PAS_WEBHOOK_SECRET` (only when the secret is set).
- 2xx ⇒ `Ok(())`. Anything else (4xx, 5xx, DNS / TLS / connect /
  read / timeout) ⇒ `Err(Error::Streaming)`, so the outbox row stays
  unpublished and the dispatcher retries on its next tick. Once the
  retry budget is exhausted the row moves to `outbox_dead_letters`
  exactly like the HL7 v2 path.
- Receivers MUST be idempotent on `X-PAS-Event-Id` — retries can
  duplicate when the server committed locally before a network
  failure surfaced.

### Added — `CompositePublisher`

- New struct in `src/streaming/mod.rs`. Wraps a non-empty list of
  `Arc<dyn EventPublisher>` and forwards every event to **all** of
  them sequentially.
- First-failure-wins: if any child returns `Err`, the composite
  returns that error and the outbox row stays pending. The remaining
  children are not invoked.
- Idempotency on the receiver side is the recommended way to handle
  duplicates that arise when a downstream-published event is retried
  because a later child failed. (HL7 v2 receivers should key on
  MSH-10; webhook receivers should key on `X-PAS-Event-Id`.)

### Added — config

- `PAS_WEBHOOK_URL` — destination URL. Empty / unset ⇒ webhook
  disabled.
- `PAS_WEBHOOK_SECRET` — optional HMAC-SHA256 secret. Enables the
  `X-PAS-Signature` header.
- `PAS_WEBHOOK_TIMEOUT_SECS` — request timeout. Default `10`.
  Ignored when `PAS_WEBHOOK_URL` is empty.

### Changed — `main.rs` publisher selection

Publisher selection is now a chain rather than a switch:

1. If `HL7V2_OUTBOUND_PEER` is set, append `Hl7v2MllpPublisher`.
2. If `PAS_WEBHOOK_URL` is set, append `WebhookEventPublisher`.
3. If the chain is empty, fall back to `InMemoryEventPublisher`. If
   it has exactly one entry, use it directly. Otherwise wrap in a
   `CompositePublisher`.

This is backwards-compatible: an existing deployment that has only
`HL7V2_OUTBOUND_PEER` set continues to get exactly the v0.13
behavior, with the same single publisher type, no composite
overhead.

### Added — deps

- `reqwest = { version = "0.12", default-features = false,
  features = ["json", "rustls-tls"] }` — promoted from transitive
  (already in the tree via `opentelemetry-otlp`'s `reqwest-client`
  feature). Zero compile cost.
- `sha2 = "0.10"`, `hmac = "0.12"` — promoted from transitive (already
  in the tree via sqlx). Zero compile cost.

### Added — tests

- 8 new lib unit tests in `src/streaming/webhook.rs`:
  - HMAC-SHA256 reference vector (RFC 4231 key=`"key"`, body=`""`)
  - signature changes with body / with secret
  - empty URL rejected
  - end-to-end POST with body + headers + signature
  - signature header omitted when no secret
  - 500 → `Err(Streaming(... contains "500"))`
  - transport failure (connect to closed port) → `Err`
- 3 new lib unit tests on `CompositePublisher`:
  - fan-out to two children
  - first-failure-wins short-circuits the chain
  - single-child composite is equivalent to the bare child
- 1 new config test: `test_from_env_parses_webhook_overrides`.
- 2 new DB-free integration tests in `tests/webhook_test.rs`:
  - `webhook_publisher_posts_event_with_hmac_signature` — end-to-end
    POST against an in-process `TcpListener` receiver; verifies body
    shape, all three custom headers, and the HMAC matches the body.
  - `composite_publisher_fans_out_to_webhook_and_in_memory` — webhook
    + in-memory composite, asserts both subscribers got the event.

### Version

- `patient-administration-system` 0.13.0 → 0.14.0.

## [0.13.0] — 2026-05-25

The FHIR Coverage write release. The `POST /fhir` Bundle endpoint
(both `type: batch` and `type: transaction`) now accepts `Coverage`
resource entries alongside Patient / Encounter / Appointment.
Coverage rows created this way are indistinguishable from those
created via `POST /api/coverages` — same repository, same audit
trail.

### Added — FHIR wire layer

- `FhirCoverage::into_domain(self) -> Result<Coverage>`:
  - `beneficiary.reference = "Patient/{uuid}"` → `patient_id`
    (required; missing or malformed → 400).
  - `subscriber.reference = "Patient/{uuid}"`, when present, →
    `subscriber_id` (optional; ignored if shape unknown).
  - `subscriberId` (the FHIR string field — opaque policy id) →
    domain `policy_number` (required).
  - `payor[0].display` → `payor_name` (required). `payor[0].identifier
    .value` → `payor_identifier` (optional).
  - `status` accepts both FHIR kebab-case (`entered-in-error`) and
    domain snake_case (`entered_in_error`).
  - `type.text` → `CoverageKind`. Defaults to `insurance` when
    missing; rejects unknown values.
  - `period.start` → `start_date` (required). `period.end` →
    `end_date`.
  - `relationship.text` → `relationship` string. Defaults to `"self"`.

### Added — Bundle handler

- New `create_coverage_in_db` helper, symmetric with the existing
  `create_{patient,encounter,appointment}_in_db` helpers.
- `ResourceKind` enum gains a `Coverage(serde_json::Value)` variant;
  both `process_batch_bundle` and `process_transaction_bundle` now
  dispatch Coverage entries to the new helper.
- `validate_and_route` accepts `"Coverage"` alongside the three prior
  resource types; any other `resourceType` still produces the
  existing `400 Bad Request: unsupported resourceType …` diagnostic.
- The OpenAPI `request_body` description for `POST /fhir` is bumped
  to list Coverage in the supported set.

### Added — tests

- `src/api/fhir/resources.rs` (lib): 4 new unit tests
  - `test_fhir_coverage_round_trip_through_into_domain`
  - `test_fhir_coverage_into_domain_requires_beneficiary_patient`
  - `test_fhir_coverage_into_domain_accepts_entered_in_error_kebab_case`
  - `test_fhir_coverage_into_domain_requires_payor_display`
- `tests/fhir_bundle_test.rs` (integration, DB-bound):
  - `fhir_bundle_creates_coverage_entries` — batch with two Coverage
    entries (primary insurance + self-pay); both succeed; one is
    read back through `GET /fhir/Coverage/{id}`.
  - `fhir_bundle_transaction_rolls_back_when_coverage_invalid` —
    transaction with a valid Coverage followed by one missing
    `payor[0].display` rolls back atomically; the probe policy
    number is not found via `CoverageRepository::list_by_patient`.

### Changed — docs

- Stale "Read-only as of v0.10" comments in `src/api/fhir/resources.rs`
  and `src/api/fhir/handlers.rs` updated to point at the v0.13 write
  path.

### Version

- `patient-administration-system` 0.12.0 → 0.13.0.

## [0.12.0] — 2026-05-25

The rate-limit release. Per-IP token-bucket middleware in front of the
HTTP server caps incoming request rate so a runaway client (or a
brute-force bearer-token guesser) can't saturate the dispatcher /
database pool.

### Added — middleware

- New `src/api/rest/rate_limit.rs`:
  - `RateLimitConfig { requests_per_minute, burst }` — capacity +
    refill rate.
  - `RateLimiter` — `Arc<Mutex<HashMap<IpAddr, Bucket>>>` token-bucket
    store. Cleanup sweep evicts buckets idle >5 min once the map
    crosses `MAX_BUCKETS = 50_000`.
  - `rate_limit_middleware` — tower middleware that extracts peer IP
    via `axum::extract::ConnectInfo<SocketAddr>`, debits one token,
    allows or returns `429`. `/api/health` is exempt so operational
    pings never trip the limiter.
- **No new direct dep**. Hand-rolled in `std` so the dependency tree
  stays in line with the v0.3.1 audit posture. (`governor` /
  `tower_governor` would be the off-the-shelf choices for richer
  behavior; deliberately not pulled in.)

### Added — middleware stack placement

Layer order in `main.rs` (innermost → outermost):

  handlers → bearer-auth → **rate-limit** → trace → cors → compress

Rate-limit sits **outside** bearer auth so brute-force token guessing
is throttled, but **inside** trace so 429s still get logged.

### Added — config

- `PAS_RATE_LIMIT_RPM` env var (default `600` = 10 req/sec sustained).
  Set `0` to disable the layer entirely.
- `PAS_RATE_LIMIT_BURST` env var (default `60` ≈ 6 seconds of burst
  at sustained rate). Ignored when RPM is `0`.
- `Config` gains `pas_rate_limit_rpm: u32` + `pas_rate_limit_burst: u32`.

### Added — response shape on cap

- Status `429 Too Many Requests`.
- Header `Retry-After: <seconds>` rounded up to the next whole second.
- Body uses the standard `ApiResponse` envelope:
  `{ success: false, data: null, error: { code: "RATE_LIMITED", message: "..." } }`.

### Added — `axum::serve` change

- `into_make_service_with_connect_info::<SocketAddr>()` is now used in
  place of the implicit `IntoMakeService`. Required so the middleware
  can extract peer IP via `ConnectInfo<SocketAddr>`. Without it, every
  request looked like it came from no IP and all clients shared one
  bucket — useless for per-IP limiting.

### Tests

- 7 new lib unit tests in `api::rest::rate_limit::tests` (bucket
  starts full, refills over time, caps at capacity, Retry-After ≥ 1
  second, per-IP isolation, cleanup runs without panic, refill math).
- 1 new config test (`PAS_RATE_LIMIT_RPM` / `PAS_RATE_LIMIT_BURST`
  parse correctly).
- 2 new integration tests in `tests/rate_limit_test.rs` (both run
  without `DATABASE_URL` — the middleware needs no DB):
  - `rate_limit_blocks_after_burst_and_serves_429_with_retry_after`:
    burst=2 → first two pass, third gets 429 with the right body
    + Retry-After header.
  - `rate_limit_exempts_health_endpoint`: drain the bucket on `/test`,
    then assert 5 consecutive hits to `/api/health` still 200.
- Lib total: 357 → **365 passing** (+8). Integration: 38 → **40**
  (+2). Workspace: 366 → **374 passing**.

### Documentation

- `spec.md` §4.9 (Observability & Ops) describes the limiter; §11.1
  (Config) adds the two new env vars; §12 + §14 (counts + acceptance)
  refreshed.
- `README.md` Config table adds `PAS_RATE_LIMIT_RPM` +
  `PAS_RATE_LIMIT_BURST`; Status + Testing counts updated.
- `AGENTS/share/availability.md` notes the per-IP limiter;
  `AGENTS/share/overview.md`, `AGENTS/index.md`, `AGENTS/testing.md`
  count bumps.

### Verified

`cargo build --workspace --all-targets`, `cargo test --workspace --lib`
(365 PAS + 9 patient-administration-system-frontend = **374 passing**), `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo fmt --check --all` all green at
the bumped version.

## [0.11.0] — 2026-05-25

The Patient merge / tombstone release. Closes the long-standing
handoff with the sister MPI crate — when the MPI determines two PAS
patient rows refer to the same person, one row becomes the survivor
and the other becomes a tombstone pointing at it. The PAS has
advertised an `mpi_id` column since v0.1; v0.11 finally gives the MPI
something concrete to call.

### Added — schema

- New migration `m20260528_000005_patient_replaced_by`. ALTER
  `patients` ADD `replaced_by UUID NULL`. Partial index
  `idx_patients_replaced_by ON patients (replaced_by) WHERE
  replaced_by IS NOT NULL` for cheap inverse lookups.
- No new tables (32 total, unchanged from v0.10).

### Added — domain model

- `Patient` gains `replaced_by: Option<Uuid>` (with `#[serde(default)]`
  so existing JSON payloads continue to deserialize). `Patient::new`
  defaults it to `None`.
- A merge tombstone is a row with `replaced_by = Some(target)` and
  `active = false`. Both rows survive — the audit trail can replay
  the merge.

### Added — repository

- `PatientRepository::set_replaced_by(id, target_id)` — atomic flip:
  sets `replaced_by`, flips `active = false`, stamps `updated_at`.
  Idempotent against re-merging into the same target.
- `PatientRepository::list_replaces_for(target_id)` — inverse lookup:
  every tombstone that points at `target_id`, newest-first.
- `PatientRepository::list_active` now also filters
  `replaced_by IS NULL` — tombstones drop out of the default list.

### Added — REST endpoints

- `POST /api/patients/{id}/merge-into/{target_id}`:
  - Validation: `id != target_id` (400), source must exist (404),
    source must not already be a tombstone (409 — no chained merges),
    target must exist (404).
  - One DB transaction: flip the source row + audit
    (`action = "merge_into"`) + outbox (`event_type = "PatientMerged"`).
  - After commit, best-effort drop the source from the Tantivy index.
- `GET /api/patients/{id}/replaces` — list of tombstones that point
  at this row. Returns `[]` when no merge has ever happened.

### Added — FHIR R5

- `FhirPatient.link` (new field on the existing resource) — emitted
  when the domain row is a tombstone. One entry with
  `type = "replaced-by"` (kebab-case, per FHIR spec) and
  `other.reference = "Patient/{target}"`. Live patients emit an empty
  array, suppressed by `skip_serializing_if`.
- The reverse direction (a survivor enumerating its `replaces` links)
  is **not** auto-emitted on the survivor's FHIR resource — that
  would cost an extra DB query per FHIR read. Callers who want the
  list use `GET /api/patients/{id}/replaces`.

### Added — outbox event

- New event type `PatientMerged` with payload
  `{ source_id, target_id }`. Sister MPI crates and downstream
  systems can subscribe to keep their identity tables in sync.

### Tests

- 1 new lib unit test
  (`test_patient_to_fhir_emits_replaced_by_link_on_tombstone`).
- 1 new integration test `patient_merge_full_lifecycle_with_fhir_link_and_search_drop`:
  create A + B → merge → assert tombstone fields → assert B
  unchanged → assert `GET /api/patients/{B}/replaces` lists A →
  assert FHIR Patient/A carries the `replaced-by` link → assert FHIR
  Patient/B has no link → assert search no longer surfaces A → reject
  self-merge (400) → reject double-merge of A (409) → reject merge
  into a non-existent target (404).
- Lib total: 356 → **357 passing** (+1). Integration: 37 → **38**.
  Workspace: 365 → **366 passing**.

### Added — OpenAPI

- 2 new annotated handlers. `ApiDoc` reaches **105 annotated paths**
  (was 103).

### Documentation

- `spec.md` §4.1 (Identity) gains a merge subsection; §6.5
  (Aggregates) notes the tombstone link on `Patient`; §9.1 (Schema)
  notes the new column; §12 + §14 (counts + acceptance) refreshed;
  §2.3 (MPI boundary) updated — the integration seam now has real
  endpoints behind it.
- `README.md` endpoint table gains the two new routes, Features
  mentions the merge surface, Status + Testing counts updated.
- `AGENTS/share/overview.md` notes the merge surface; `AGENTS/index.md`
  + `AGENTS/testing.md` count bumps.

### Verified

`cargo build --workspace --all-targets`, `cargo test --workspace --lib`
(357 PAS + 9 patient-administration-system-frontend = **366 passing**), `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo fmt --check --all` all green at
the bumped version.

## [0.10.0] — 2026-05-25

The Coverage (insurance / payer) release. Adds the natural complement
to the billing aggregate: a new `Coverage` aggregate that records who
pays for a patient's care, with optional linkage to a billing account
and a FHIR R5 read surface so downstream payer integrations can
consume PAS data through the standard interop path.

### Added — domain model

- New `src/models/coverage.rs`:
  - `CoverageStatus`: `Active` / `Cancelled` / `Draft` / `EnteredInError`
    (FHIR R5 aligned; snake_case serde).
  - `CoverageKind`: `Insurance` (default) / `SelfPay` / `Other`.
  - `Coverage` struct: `id`, `patient_id`, optional `account_id`,
    `status`, `kind`, optional `subscriber_id`, `payor_name`, optional
    `payor_identifier`, `policy_number`, optional `group_number`,
    `relationship` (default `"self"`), `start_date`, optional
    `end_date`. **No state machine** — status flips are intentional
    and operator-driven.

### Added — schema

- New migration `m20260527_000004_coverage` creates the `coverages`
  table. Indexes: `(patient_id)`, partial `(account_id) WHERE
  account_id IS NOT NULL`, and a partial unique
  `(patient_id, payor_name, policy_number) WHERE status = 'active'`
  to prevent active-row duplicates.
- **No `deleted_at`** — invariant §5.3 keeps soft-delete to
  patients/encounters/appointments; coverage retirement is via
  `status = 'cancelled'` or `status = 'entered_in_error'`.
- 32 PAS tables total (was 31).

### Added — repository

- `CoverageRepository` with `create`, `find_by_id`, `update`,
  `set_status` (status-only flip; used by the DELETE handler),
  `link_to_account` (attach / detach via `Option<Uuid>`),
  `list_by_patient`, `list_by_account`. Inline tests for the string
  conversions + round-trip via the explicit `Model` shape.

### Added — REST endpoints

- `POST   /api/coverages` — create.
- `GET    /api/coverages/{id}` — read.
- `PUT    /api/coverages/{id}` — selective update; `Option<Option<T>>`
  fields (e.g. `account_id`, `subscriber_id`, `payor_identifier`,
  `group_number`, `end_date`) let callers explicitly clear by sending
  `null`, distinct from omitting the field.
- `DELETE /api/coverages/{id}` — flips status to `Cancelled`. Coverage
  rows are **never** hard-deleted.
- `GET    /api/patients/{id}/coverages` — list per patient, newest first.
- `GET    /api/accounts/{id}/coverages` — list per account, newest first.
- All six audit-logged. `CoverageCreated` / `CoverageUpdated` /
  `CoverageCancelled` outbox events emitted.

### Added — FHIR R5

- `GET /fhir/Coverage/{id}` — read-only. Maps the domain row to a FHIR
  R5 `Coverage` resource:
  - `patient_id` → `beneficiary` (`Patient/{id}`).
  - `subscriber_id` (defaulting to `patient_id`) → `subscriber`.
  - `policy_number` → `subscriberId`.
  - `payor_name` → `payor[0].display`; `payor_identifier` →
    `payor[0].identifier.value`.
  - `start_date` / `end_date` → `period.start` / `period.end`.
  - `EnteredInError` is translated to FHIR's `entered-in-error`
    (kebab-case); everything else passes through as-is.
- FHIR Bundle write support for Coverage is deferred to v0.11.

### Added — tests

- 5 new lib unit tests in `models::coverage::tests` (default
  constructor, status serde snake_case, kind serde snake_case,
  status round-trip, full-field serde round-trip).
- 4 new repo unit tests (zero-sized marker, status string round-trip,
  kind string round-trip, active-model round-trip with explicit
  `Model`).
- 1 new integration test `coverage_full_lifecycle_with_fhir_read` in
  `tests/coverage_flow_test.rs`: create → list per patient → open
  account + link → list per account → update (kind + clear group via
  null) → FHIR GET → soft-cancel via DELETE → FHIR GET after cancel →
  404 on unknown id.
- Lib total: 347 → **356 passing** (+9). Integration: 36 → **37**.
  Workspace: 356 → **365 passing**.

### Added — OpenAPI

- 6 new `#[utoipa::path]` annotated handlers + 2 new `ToSchema`
  request structs. `ApiDoc` now reaches **103 annotated paths** (was 97).

### Documentation

- `spec.md` §4.5 (Billing) expanded with a Coverage subsection; §6.5
  (Aggregates) adds the Coverage aggregate; §9.1 (Schema) bumps to 32
  tables; §12 + §14 (counts + acceptance) refreshed.
- `README.md` endpoint table gains the seven new routes (6 REST + 1
  FHIR), Features mentions Coverage, Status + Testing counts
  updated, Deferred list updated.
- `AGENTS/share/overview.md` notes Coverage; `AGENTS/index.md` +
  `AGENTS/testing.md` count bumps.

### Verified

`cargo build --workspace --all-targets`, `cargo test --workspace --lib`
(356 PAS + 9 patient-administration-system-frontend = **365 passing**), `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo fmt --check --all` all green at
the bumped version.

## [0.9.0] — 2026-05-25

The recurring appointment series release. Adds first-class support for
patterns like "weekly cardiology follow-up for 12 weeks" or "every
Tuesday and Thursday until 2026-09-30" without making the operator
book each appointment one at a time.

### Added — domain model

- New `src/models/appointment_series.rs`:
  - `Frequency`: `Daily` / `Weekly` / `Monthly` (RFC 5545 `FREQ`,
    minus the variants v0.9 does not support — yearly, sub-daily).
  - `RecurrenceRule`: `frequency` + `interval >= 1` + optional
    `by_weekday: Vec<Weekday>` (only meaningful for `Weekly`) +
    `end: RecurrenceEnd { Count{count} | Until{until} }`.
  - `AppointmentSeries`: id, patient_id, optional practitioner_id,
    service_type, `start_datetime`, `duration_minutes`, the rule,
    `status: SeriesStatus { Active | Cancelled }`, reason.
  - Free function `compute_occurrences(rule, start) -> Result<Vec<DateTime<Utc>>>`.
    Handles `Daily` (cursor + N days), `Weekly` (day-by-day filter by
    `by_weekday`), `Monthly` (chrono's `Months` arithmetic, which
    clamps Jan 31 → Feb 28 for short months). Validates the rule and
    enforces a hard cap of `MAX_OCCURRENCES = 200` so a runaway count
    can't pile up arbitrary DB rows.
- `Appointment` gains `series_id: Option<Uuid>` — backlink to the
  generating series; `None` for singleton appointments.

### Added — schema

- New migration `m20260526_000003_appointment_series`:
  - `appointment_series` table (rule stored as JSONB; everything else
    typed columns). Index on `patient_id`.
  - `appointments` gains `series_id UUID NULL` + index.

### Added — service

- New `src/scheduling/series.rs::SeriesService` with three transactional
  methods + two reads:
  - `preview(input) -> PreviewResult` — pure dry-run, no DB writes.
    Returns the computed datetimes so the UI can confirm with the user
    before commit.
  - `create(input, ctx) -> CreateSeriesResult` — **one DB
    transaction**: insert series row, expand to N occurrences, for
    each: per-patient overlap check (atomic reject — any conflict
    rolls back the whole transaction with a `409` naming the offending
    datetime + appointment id), insert `Appointment(Booked)` with
    `series_id`. Audit (`appointment_series:create`) + outbox
    (`AppointmentSeriesCreated`) written in the same transaction.
  - `cancel(series_id, reason, ctx) -> SeriesWithOccurrences` — one DB
    transaction: flip series to `Cancelled`, walk every linked
    occurrence and cancel the `Proposed` / `Booked` ones via the
    appointment state machine (terminal `Arrived` / `Fulfilled` /
    `NoShow` rows are left alone). Audit + outbox.
  - `get_with_occurrences(series_id)` / `list_by_patient(patient_id)`
    — straightforward reads.
- New `AppointmentRepository::list_by_series` /
  `set_status_and_reason`. New `AppointmentSeriesRepository`.

### Added — endpoints

- `POST /api/appointment-series/preview` — dry-run.
- `POST /api/appointment-series` — create. Returns the series plus
  every concrete `Appointment` row.
- `GET /api/appointment-series/{id}` — series + occurrences.
- `POST /api/appointment-series/{id}/cancel` — cancel future occurrences.
- `GET /api/patients/{id}/appointment-series` — list per patient.

All five carry `#[utoipa::path]` annotations. `CreateSeriesRequest` and
`CancelSeriesRequest` registered as `ToSchema`. `ApiDoc` now reaches
97 annotated paths.

### Added — domain events

- `AppointmentSeriesCreated` / `AppointmentSeriesCancelled` —
  emitted to the outbox alongside per-occurrence
  `AppointmentBooked` / `AppointmentCancelled` events.

### Tests

- 12 new lib unit tests in `models::appointment_series::tests`
  (daily count, daily interval, weekly default same-weekday, weekly
  BYDAY MWF, weekly UNTIL, monthly clamp-to-short-month, plus six
  validation rejections). 2 new repo unit tests
  (`series_status_roundtrip`, `to_active_model_serialises_rule_to_json`).
  1 new service smoke test.
- 2 new integration tests in `tests/appointment_series_test.rs`:
  preview → create → fetch → cancel happy path; atomic-overlap-reject
  (a singleton blocker on week 2 of 4 aborts the whole create and
  leaves zero series rows behind).
- Lib total: 332 → **347 passing** (+15). Integration: 34 → **36** (+2).
  Workspace: 341 → **356 passing**.

### Documentation

- `spec.md` §4.2 (Scheduling) gains a recurring-series subsection;
  §6.5 (Aggregates) adds the new aggregate; §9.1 (Schema) lists the
  new table; §2.2 removes the deferred entry; §12 + §14 (counts +
  acceptance criteria) refreshed.
- `README.md` Features mentions recurring series, endpoint table adds
  the five new routes, Status + Testing counts updated, Deferred list
  updated.
- `AGENTS/share/overview.md` notes recurring series; `AGENTS/index.md`
  + `AGENTS/testing.md` count bumps.

### Verified

`cargo build --workspace --all-targets`, `cargo test --workspace --lib`
(347 PAS + 9 patient-administration-system-frontend = **356 passing**), `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo fmt --check --all` all green at
the bumped version.

## [0.8.0] — 2026-05-25

The SMS letter channel release. `DeliveryChannel::Sms` has been an enum
variant since v0.1 but generating an SMS letter just persisted a
`Pending` row — no message ever left the building. v0.8 wires a
trait-based SMS provider so a real gateway (Twilio, MessageBird, …)
can be swapped in by the consumer, and ships two first-party
implementations: `NoopSmsProvider` (the default) and `LogSmsProvider`
(dev / smoke-test, logs every outbound message to `tracing`).

### Added — module

- New `src/communication/sms.rs`:
  - `SmsProvider` trait: `async fn send(&self, to, body) -> Result<()>`,
    default `fn is_enabled(&self) -> bool { true }`.
  - `NoopSmsProvider` — default. `is_enabled() == false`, so the auto-
    send code path is skipped entirely; behavior matches v0.7 exactly
    (letter rendered + persisted as `Pending`).
  - `LogSmsProvider` — `is_enabled() == true`, logs `to` + `body` +
    char-count at `tracing::info!(target: "pas::sms", …)`, returns
    `Ok(())`. Useful for dev runs and replays without real spend.

### Added — service

- `CommunicationService::with_sms_provider(...)` builder swaps in a
  concrete provider. The service holds it by `Arc<dyn SmsProvider>`.
- `CommunicationService::generate_letter` now performs **auto-send**
  when `channel == Sms` and the configured provider reports
  `is_enabled()`:
  - Look up the patient's first `ContactPoint { system: Phone }`.
    Missing? Log a `warn!`, write `audit_log("send_sms_skipped_no_phone")`,
    leave the letter `Pending`.
  - Present? Call `provider.send(phone, rendered_body)`. On `Ok`: flip
    the row to `Sent` + stamp `sent_at`, write
    `audit_log("send_sms_ok")` with the recipient phone.
  - On `Err`: flip the row to `Failed`, write
    `audit_log("send_sms_failed")` with the diagnostic.
- The existing `POST /api/letters/{id}/sent` and `…/failed` endpoints
  still work for Print, Email, and operator overrides.

### Added — config

- New `PAS_SMS_PROVIDER` env var. Recognized values:
  - `"none"` (default — install `NoopSmsProvider`).
  - `"log"` (install `LogSmsProvider`).
  Unknown values fall back to `none` with a startup `warn!` so a typo
  can't accidentally enable auto-send against a live patient list.
- `Config` gains `pas_sms_provider: String` (lowercased on read).

### Added — wiring

- `AppState::with_sms_provider(Arc<dyn SmsProvider>)` builder method
  rebuilds `CommunicationService` around the new provider.
- `main.rs` matches on `cfg.pas_sms_provider` and installs the
  corresponding provider before serving requests.

### Added — tests

- 3 new lib unit tests in `sms::tests` (Noop is_enabled=false + send
  Ok; Log is_enabled=true + send Ok; Log handles empty body without
  panicking).
- 1 updated config test asserts `PAS_SMS_PROVIDER` is normalised
  (uppercase + whitespace) and that the default is `"none"`.
- 1 new integration test `letter_sms_auto_sends_when_log_provider_is_wired`
  in `tests/letter_flow_test.rs`:
  - Patient with a phone telecom + `LogSmsProvider` → letter flips to
    `sent` and `sent_at` is stamped.
  - Patient with no phone telecom → letter stays `pending`, no
    `sent_at`.
- Lib total: 328 → **332 passing** (+4). Integration: 33 → **34**.
  Workspace lib: 337 → **341 passing**.

### Documentation

- `DeliveryChannel::Sms` doc comment rewritten to describe the new
  auto-send behavior (was "declared for completeness; not actually
  delivered").
- `spec.md` §4.6 (Communications) gains an SMS subsection; §11.1
  (Configuration) adds `PAS_SMS_PROVIDER`; §2.2 removes the deferred
  entry; §12 + §14 (counts + acceptance criteria) refreshed.
- `README.md` Config table adds `PAS_SMS_PROVIDER`; Status and Testing
  counts updated; Deferred list updated.
- `AGENTS/share/overview.md` notes SMS is now wired; `AGENTS/index.md`
  + `AGENTS/testing.md` count bumps.

### Verified

`cargo build --workspace --all-targets`, `cargo test --workspace --lib`
(332 PAS + 9 patient-administration-system-frontend = **341 passing**), `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo fmt --check --all` all green at
the bumped version.

## [0.7.0] — 2026-05-25

The OpenTelemetry release. Wires the OTLP exporter that has been
documented (but unwired) since the v0.3.1 dependency audit. Setting
`OTLP_ENDPOINT` now actually exports spans to a collector — no code
change required at call sites.

### Added — OpenTelemetry OTLP exporter

- New `observability::init(&Config)` signature replaces the old
  `init(&str)`. When `cfg.otlp_endpoint` is `Some(url)`, the function
  builds an OTLP `SpanExporter` (HTTP/protobuf transport via reqwest,
  no tonic — keeps the v0.3.1 dep-audit posture intact), wraps it in a
  batch processor that runs on the tokio runtime, and installs a
  `tracing_opentelemetry::layer()` alongside the existing fmt layer.
  Every `#[tracing::instrument]` and `tracing::info_span!` in the
  service now lands in the collector for free.
- When `cfg.otlp_endpoint` is `None`, behavior is identical to v0.6 —
  fmt layer only, no network egress.
- Setup failures (malformed endpoint, reqwest init failure, etc.) log
  a `warn!` and fall back to fmt-only; the server still boots. The
  shipped service must always come up — observability is best-effort.

### Added — config

- New `OTEL_SERVICE_NAME` env var, default `pas-axum`. Attached as the
  `service.name` resource attribute on every exported span. Override
  to disambiguate replicas / environments in the collector.
- `Config` gains `otel_service_name: String`.

### Added — dependencies

Four crates reintroduced (all transitively removed in the v0.3.1
audit; now actually needed):

- `opentelemetry = "0.27"`
- `opentelemetry_sdk = { version = "0.27", features = ["rt-tokio"] }`
- `opentelemetry-otlp = { version = "0.27", default-features = false, features = ["http-proto", "reqwest-client"] }`
- `tracing-opentelemetry = "0.28"`

HTTP/protobuf transport (via `reqwest-client`) was picked over the
default gRPC/tonic transport so the PAS doesn't have to take a
direct `tonic` dep back. (Some `tonic` does come in transitively
through `opentelemetry-otlp`'s common type crate, but no direct dep.)

### Tests

- 2 new lib unit tests in `observability::tests` (init without OTLP
  returns `Ok` and is idempotent; init with a syntactically-valid
  endpoint URL builds the exporter without panicking under a
  `#[tokio::test]` runtime).
- 1 updated config test asserts `OTEL_SERVICE_NAME` plumbs through and
  defaults to `pas-axum`.
- Lib total: 326 → **328 passing**. Workspace total: 335 → **337
  passing**. Integration count unchanged (33 functions).

### Documentation

- `spec.md` §4.9 (Observability & Ops) and §11.1 (Configuration)
  updated; deferred-list entry "Real OpenTelemetry OTLP exporter"
  removed from §2.2.
- `README.md` Config table gains `OTEL_SERVICE_NAME`; Status section
  updated.
- `AGENTS/share/observability.md` rewritten to document the now-wired
  path; `AGENTS/share/technology.md` moves OpenTelemetry from the
  "planned" row to the "wired" row.

### Verified

`cargo build --workspace --all-targets`, `cargo test --workspace --lib`
(328 PAS + 9 patient-administration-system-frontend = **337 passing**), `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo fmt --check --all` all green at
the bumped version.

## [0.6.0] — 2026-05-25

The HL7 v2 batch release. Adds support for the v2 batch envelope so a
sender can ship many ADT messages in one transmission (bulk backfill,
end-of-day batches, system migrations).

### Added — HL7 v2 batch envelope

- New `POST /api/hl7/v2/batch` accepts an `FHS`/`BHS`/`BTS`/`FTS`
  envelope around any number of `MSH` messages (up to
  `MAX_BATCH_MESSAGES = 1000`). Each contained message is dispatched
  independently to the matching single-message handler
  (`hl7_v2_admit` / `_transfer` / `_discharge` / `_update` /
  `_cancel_admit` / `_cancel_discharge` / `_patient`), and the
  responses are stitched back together into one batch ACK envelope:
  `BHS … <per-message MSH + MSA blocks> … BTS|<n>`. Per-message
  AA/AE/AR appears inside the envelope; one failure does **not** roll
  back the others (matches real HL7 v2 batch processors and mirrors
  the PAS FHIR `batch` Bundle path).
- Always returns `200 OK` when the envelope itself parses (per-message
  ACKs carry the success/failure detail). Returns `400 BAD REQUEST`
  with a single `AR` envelope when the batch is malformed, oversize,
  or carries more than one `BHS` (multi-batch files are out of scope).
- The MLLP listener routes any payload that begins with `FHS` or `BHS`
  to `/api/hl7/v2/batch`, so the batch surface is reachable over both
  HTTP and MLLP without a separate listener.
- Bare `MSH` lists without a `BHS` envelope are also accepted (some
  senders are lazy) — surfaced as `Batch { bhs: None, … }`.

### Added — module

- New `src/hl7v2/batch.rs`: `Batch { fhs, bhs, messages, bts, fts }`,
  `parse_batch(s) -> Result<Batch>`, `encode_batch_ack(...) -> String`,
  `looks_like_batch(s) -> bool`, `MAX_BATCH_MESSAGES`. Inline tests for
  every parse path + the envelope shape. Re-exported from
  `src/hl7v2/mod.rs`.

### Added — example

- `examples/hl7-batch-adt.txt` — three-message ADT^A28 batch demo.

### Added — tests

- 9 new lib unit tests in `hl7v2::batch::tests` (parse with envelope,
  parse bare MSH list, accept FHS+BHS, reject empty, reject body
  before MSH, reject second BHS, reject oversize, encode wraps ACKs,
  `looks_like_batch` detection).
- 2 new MLLP listener routing tests (BHS, FHS → batch endpoint).
- 1 new integration test
  `hl7v2_batch_dispatches_each_message_independently`: 3-message
  batch (2 OK + 1 bad), asserts AA/AA/AE per-message inside one BTS|3
  envelope, asserts the two OK patients are searchable, asserts empty
  input returns `400 + AR`.
- Lib total: 315 → **326 passing**. Integration total: 32 → **33**.

### Added — OpenAPI

- `hl7_v2_batch` registered in `ApiDoc` (90 → **92 annotated handlers**;
  the previous v0.5 release was 91, this brings it to 92 with the one
  new handler).

### Documentation

- `spec.md` §4.8.3 expanded with the batch endpoint, the dispatch rule,
  and the `MAX_BATCH_MESSAGES` cap.
- `README.md` endpoint table gains the new route.
- `AGENTS/interchange.md` HL7 v2 section refreshed.

### Verified

`cargo build --workspace --all-targets`, `cargo test --workspace --lib`
(326 PAS + 9 patient-administration-system-frontend = **335 passing**), `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo fmt --check --all` all green at
the bumped version.

## [0.5.0] — 2026-05-24

The outbox dead-letter release. Bounds the outbox dispatcher's retry
behavior so a chronically-failing peer can no longer pile up retries
forever, and gives operators the endpoints they need to review +
replay events that got dead-lettered.

### Added — schema

- New migration `m20260525_000002_outbox_dlq`. Two changes:
  - `outbox_events` gains three columns: `retry_count INTEGER NOT NULL
    DEFAULT 0`, `last_attempted_at TIMESTAMPTZ NULL`, `last_error TEXT
    NULL`. Existing rows back-fill with `retry_count = 0`.
  - New `outbox_dead_letters` table: `id UUID PK`, `original_id UUID`,
    `event_type TEXT`, `payload JSONB`, `created_at TIMESTAMPTZ`,
    `dead_lettered_at TIMESTAMPTZ DEFAULT now()`, `retry_count INTEGER`,
    `last_error TEXT`. Index on `dead_lettered_at` for newest-first
    listing.

### Added — dispatcher

- New `tick` argument: `max_retries: u32`. After the N-th consecutive
  failed publish, the row is moved into `outbox_dead_letters` in one DB
  transaction (insert dead-letter + delete original) and stops being
  retried. `0` disables dead-lettering (retry forever).
- New `OutboxRepository::record_failure(id, error) -> Result<i32>`
  increments `retry_count`, records the error, returns the new count.
- New `OutboxRepository::move_to_dead_letter(id, error)` performs the
  atomic move.
- New `OutboxRepository::publish` now seeds `retry_count = 0` /
  `last_attempted_at = NULL` / `last_error = NULL`. Existing call sites
  unchanged.
- `mark_published` now stamps `last_attempted_at` and clears
  `last_error` so a successful tick leaves no stale failure metadata.

### Added — admin surface

- `GET /api/admin/outbox/dead-letters?limit=N` — newest-first list of
  dead-letter rows with original payload, retry count, last error.
  Capped at 500.
- `POST /api/admin/outbox/dead-letters/{id}/replay` — inserts a fresh
  `outbox_events` row carrying the same payload + `event_type` (with
  `retry_count = 0`, `last_error = NULL`), deletes the DLQ row; one DB
  transaction. Returns `{ dead_letter_id, new_outbox_id }`. Idempotent
  — returns `404` if the DLQ id is already gone. Replay is itself
  audit-logged (`entity_type = "outbox_dead_letter"`, `action =
  "replay"`).
- Both handlers registered in `ApiDoc` (now 91 utoipa-annotated paths).

### Added — DLQ repository

- New `DeadLetterRepository` with `list(limit)`, `find_by_id(id)`,
  `count()`, `replay(id) -> Result<Uuid>`. Re-exported from
  `db::repositories`.

### Added — config

- `PAS_OUTBOX_MAX_RETRIES` env var (default `10`). Plumbed through
  `Config::pas_outbox_max_retries` into `dispatcher::run`.

### Added — tests

- 1 new lib unit test (`DEFAULT_MAX_RETRIES`). Lib total: **315
  passing** (314 → 315).
- 1 new integration test
  `dispatcher_dead_letters_after_retry_budget_and_replay_restores`:
  seeds an outbox row with a known marker, drives the dispatcher with a
  small `max_retries = 3` and an `AlwaysFailPublisher`, asserts the row
  rises through `retry_count = 1, 2, 3` then disappears from
  `outbox_events` and appears in `outbox_dead_letters` with the right
  metadata. Then calls `DeadLetterRepository::replay`, asserts the DLQ
  row is gone and a fresh outbox row exists with `retry_count = 0` /
  `last_error = NULL`. Finally, a tick with an in-memory (happy)
  publisher drains the replayed row to `published = true`. Integration
  total: 31 → **32 test functions**.

### Documentation

- `spec.md`: §6.3 (transactional outbox) gains a "dead-letter queue"
  paragraph; §4.9 (Observability & Ops) lists the new admin endpoints;
  §11.1 (config) adds `PAS_OUTBOX_MAX_RETRIES`; §12.1 + §14 (acceptance
  criteria) updated to the new counts.
- `README.md` endpoint table gains the two new admin routes; Status
  line bumped; Testing section + counts updated.
- `AGENTS/share/auditability.md` documents the DLQ + replay surface.

### Verified

`cargo build --workspace --all-targets`, `cargo test --workspace --lib`
(315 PAS + 9 patient-administration-system-frontend = **324 passing**), `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo fmt --check --all` all green at
the bumped version.

## [0.4.0] — 2026-05-24

The ADT lifecycle release. Closes the obvious correction-path gap in the
HL7 v2 surface by adding the three "update / undo" ADT message types and
their MLLP routing + outbound publishing.

### Added — HL7 v2 ADT

- **`POST /api/hl7/v2/update` (ADT^A08)** — update patient information.
  Looks up the existing patient by PID-3.1 (MRN) and merges the inbound
  PID over the demographic fields it actually carries (`identifiers`,
  `name`, `telecom`, `gender`, `birth_date`, `addresses`, `updated_at`).
  Preserves the rest (`id`, `mpi_id`, `additional_names`, `deceased*`,
  `emergency_contacts`, `marital_status`, `created_at`, `active`). One
  DB transaction: patient row updated, `audit_log` (`action =
  "update_via_hl7v2_a08"`), `outbox_events` (`event_type =
  "PatientUpdated"`, with `source: "hl7v2_a08"`). Tantivy re-index after
  commit. AA ACK carries `updated patient <uuid>` in MSA-3.
- **`POST /api/hl7/v2/cancel-admit` (ADT^A11)** — reverse a wrong admit.
  Locates the patient's currently-open admission by MRN, releases the
  active `BedAssignment`, flips the bed to `Cleaning` (the regular
  post-`Occupied` transition), and moves the encounter to `Cancelled`.
  The `admissions` row is preserved for audit. Outbox event
  `EncounterCancelled` with `reason: "hl7v2_a11"`.
- **`POST /api/hl7/v2/cancel-discharge` (ADT^A13)** — reverse a wrong
  discharge. Locates the most-recently-discharged admission for the
  patient, deletes the `discharges` row, force-flips the bed back to
  `Occupied`, force-sets the encounter back to `InProgress`, and inserts
  a fresh active `BedAssignment` for the original bed. Two
  state-machine bypasses are required (`Finished → InProgress` and
  `Cleaning|Available → Occupied`) and are documented as the explicit
  exception in `spec.md` §6.4 — every other write path keeps using the
  state-machine-guarded helpers. Returns `AE` if the original bed has
  since been taken (`Occupied` by someone else) or is `OutOfService`.
  New domain event type `EncounterDischargeCancelled`.

### Added — MLLP listener

The `MllpServer` dispatch table now routes `ADT^A08`, `ADT^A11`,
`ADT^A13` to the new HTTP endpoints. No handler refactoring — the
listener keeps using `axum::Router::oneshot` against the same router.

### Added — outbound publisher

`Hl7v2MllpPublisher` now relays the new event types when configured:

- `PatientUpdated` with `source: "hl7v2_a08"` → `ADT^A08` to the
  configured peer. REST-driven patient edits **do not** carry the source
  tag and are not echoed (avoids boomeranging EMR-originated updates).
- `EncounterCancelled` with `reason: "hl7v2_a11"` → `ADT^A11`. Same
  source-gate: REST-driven encounter cancels are not echoed.
- `EncounterDischargeCancelled` → `ADT^A13` (unconditional; the event
  only originates from the A13 handler today).

### Added — service layer

- `AdtService::cancel_admission(admission_id, ctx) -> Result<Admission>`.
- `AdtService::cancel_discharge(admission_id, ctx) -> Result<Admission>`.

Both run in one DB transaction with audit + outbox writes.

### Added — repository layer

- `BedRepository::set_status_unchecked` — bypasses the bed-status state
  machine; documented as cancel-flow only.
- `EncounterRepository::set_status_unchecked` — same shape, for
  cancel-discharge's `Finished → InProgress` exception.
- `AdmissionRepository::find_most_recently_discharged_for_patient`,
  `find_latest_bed_assignment_for_encounter`, `delete_discharge`.

### Added — mapping + encoders

- `encode_adt_a08`, `encode_adt_a11`, `encode_adt_a13` for the outbound
  publisher.

### Added — tests

- 6 new lib unit tests (3 mapping encoders + 3 MLLP listener routes) —
  314 PAS lib tests passing.
- 3 new integration tests in `tests/hl7v2_test.rs` covering A08 (update
  + dedup-search reflow + unknown-MRN AE), A11 (admit → cancel → ward
  occupancy drops to 0 → re-cancel AE), and A13 (admit → discharge →
  cancel → ward occupancy back to ≥1 → re-cancel AE + unknown-MRN AE).

### Documentation

- `spec.md` §4.8.3 expanded with the three new message types, the
  state-machine-bypass rule, and the new `EncounterDischargeCancelled`
  event.
- `README.md` endpoint table gains the three new routes.
- `AGENTS/interchange.md` HL7 v2 section refreshed.

### Verified

`cargo build --workspace --all-targets`, `cargo test --workspace --lib`
(317 → **323 passing**: 314 PAS + 9 patient-administration-system-frontend csrf), `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo fmt --check --all` all
green at the bumped version.

## [0.3.4] — 2026-05-24

The audit-cleanup release. No behavior change to the API; pure
hygiene pass on the dev-dependency graph.

### Removed

- **`mockall` dev-dep** — no `use mockall` / `#[automock]` references
  anywhere in the test tree, so the crate was just bloating
  `cargo test` link time.
- **`tokio-test` dev-dep** — same story: no `use tokio_test` /
  `tokio_test::` references. `#[tokio::test]` (which we do use) comes
  from the regular `tokio` crate, not `tokio-test`.

### Verified

- `cargo build --workspace --all-targets`, `cargo test --workspace
  --lib` (308 PAS lib tests + 9 patient-administration-system-frontend csrf tests = 317
  passing), `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --check --all` all green at the bumped version.

## [0.3.3] — 2026-05-23

The Lily + workspace release. Adopts [Lily Design System](https://github.com/LilyDesignSystem/lily) headless HTML class names across every dashboard template, and promotes the project root into a proper Cargo workspace so the PAS Axum binary builds alongside the new `patient-administration-system-frontend` Loco-rs UI under one `Cargo.lock` + one `target/`.

### Changed

- **Lily Design System markup** on every dashboard template. Updated the
  five Tera templates to use Lily's semantic class names: `header` /
  `footer` on page chrome, `panel` with `role="region"` on each card,
  full `data-table` / `data-table-head` / `data-table-body` /
  `data-table-row` / `data-table-th` / `data-table-td` family on every
  table, `badge` with `data-status="ok|warn|error"` for status pills,
  `alert` with `role="status"` for empty states and inline diagnostics,
  `code` for inline code spans. ARIA tightened (`scope="col"` on every
  header cell, `aria-label` on every panel + table). Embedded
  `<style>` block updated to target the Lily class names — Lily is
  *headless* (zero CSS by design) so the consumer-provided styling is
  exactly what it expects.

### Added

- **Top-level Cargo workspace** at the project root with three members:
  `patient-administration-system-rust-crate` (this crate),
  `patient-administration-system-rust-crate/migrations`,
  and `patient-administration-system-frontend` (a sibling Loco-rs UI). Single shared
  `Cargo.lock`, single shared `target/`. `cargo build --workspace`,
  `cargo test --workspace --lib`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo fmt --check --all` all green
  from the workspace root.
- **`patient-administration-system-frontend`** — sibling Loco-rs 0.14.1 app at `../patient-administration-system-frontend/`
  (initial v0.1.0). Tera templates + HTMX live-refresh + Lily Design
  System markup. Read-mostly: shares the PAS PostgreSQL database via
  sea-orm and reuses the migration crate by path. Boot with
  `cargo run -- start` from the `patient-administration-system-frontend/` directory.
- One new lib unit test pinning the Lily class names in the dashboard
  template (`test_page_uses_lily_class_names`); lib total now 308.

## [0.3.2] — 2026-05-23

The dashboard release. Adds a server-side-rendered ops view with HTMX
live-refresh so operators can see ward occupancy, RTT breaches, outbox
status, and recent audit at a glance — no JavaScript required for
first-load, HTMX layered on as progressive enhancement.

### Added

- **Operational dashboard at `/dashboard`** — server-side-rendered Tera
  page with four panels (ward occupancy, RTT breaches, outbox
  unpublished count, recent audit). Reuses the existing services + repos;
  no template engine in `AppState`, just `Tera::one_off`. Works without
  JavaScript on first load (every fragment is embedded inline) — HTMX is
  a progressive enhancement.
- **HTMX live-refresh** on the dashboard. Four fragment endpoints
  (`GET /dashboard/wards` · `/dashboard/breaches` · `/dashboard/outbox` ·
  `/dashboard/audit`) return just their panel body; the main page wires
  `hx-get` + `hx-trigger="load, every 10s"` per panel and shows a live
  indicator dot during in-flight requests. Each fragment template lives
  in `templates/dashboard_*.html`, baked into the binary via
  `include_str!`. 11 unit tests (up from 3) plus an integration test that
  asserts the page embeds HTMX + the four `hx-get` URLs and that each
  fragment endpoint returns a body, not a full HTML document.

## [0.3.1] — 2026-05-23

The polish release. Closes the one v0.3.0 deferred flag (exact-MRN dedup on
HL7 v2 ADT ingest), tightens the dependency tree, and gets `cargo clippy
--all-targets -- -D warnings` to green for the first time.

### Added

- **Exact-MRN deduplication on HL7 v2 ADT ingest.** Both
  `POST /api/hl7/v2/patient` and `POST /api/hl7/v2/admit` now look up PID-3.1
  in the existing `patients` table via JSONB containment before creating a
  new row. If a non-deleted patient with that MRN already exists *and* the
  matched row's identifier set actually carries that MRN type, the existing
  patient is reused — no duplicate insert, no audit row, no Tantivy
  re-index. The AA ACK reports `matched existing patient <uuid>` in MSA-3
  so the sender can confirm. Probabilistic dedup (name + DOB + SSN) is
  still the sister MPI crate's job.
- Shared `dedup_or_create_patient_from_pid` helper consolidates the v2
  patient-create path so future ADT message types (A04, A08, …) get the
  same dedup treatment for free.
- Full v0.1 surface annotated with `#[utoipa::path]` — every one of the 86
  `pub async fn` handlers now appears in the OpenAPI spec, tagged across
  19 categories.

### Quality

- Clean `cargo clippy --all-targets -- -D warnings`, clean `cargo fmt --check`.
- Dependency tree slimmed: 15 unused direct deps removed (`loco-rs`,
  `fluvio`, `tonic`, `prost`, `tonic-build`, `openapiv3`, `jsonwebtoken`,
  `argon2`, `strsim`, `anyhow`, `bigdecimal`, `validator`, `hyper`, all 5
  `opentelemetry*` crates). `tower` features narrowed `"full"` → `"util"`,
  `axum` features narrowed `["macros","multipart","ws"]` → `["macros"]`.

## [0.3.0] — 2026-05-23

The interop release. v0.3 makes the PAS a full peer in HL7 v2 and FHIR
networks: it now both receives and emits ADT messages, accepts batch +
genuine transactional FHIR Bundles, and ships an OpenAPI spec for the
interchange surface.

### Added — HL7 v2 ADT

- New `src/hl7v2/` module: parser, encoder, mapping (PID ↔ Patient + ADT
  builders for A01/A02/A03/A28), and ACK builder.
- HTTP ingest endpoints:
  - `POST /api/hl7/v2/parse` — echoes structured JSON for inspection.
  - `POST /api/hl7/v2/patient` — ADT^A28 add-person; creates Patient from PID.
  - `POST /api/hl7/v2/admit` — ADT^A01; creates Patient + admits to bed in PV1-3.3.
  - `POST /api/hl7/v2/transfer` — ADT^A02; identifies admission by MRN, transfers to PV1-3.3 bed.
  - `POST /api/hl7/v2/discharge` — ADT^A03; identifies admission by MRN and discharges.
- All five endpoints respond with a v2 ACK envelope. Codes: `AA` accept,
  `AE` application error, `AR` reject before processing.
- MLLP TCP listener (`HL7V2_MLLP_BIND`, conventional port 2575). Reuses the
  same handler dispatch via `axum::Router::oneshot` — zero refactoring.
- Outbound MLLP publisher (`HL7V2_OUTBOUND_PEER`). When set, the outbox
  dispatcher emits ADT^A01/A02/A03 messages to a downstream EMR for the
  corresponding `EncounterAdmitted` / `Transferred` / `Discharged` events.
  Failed sends stay unpublished and retry on the next dispatcher tick.
- HL7 v2 escape sequences (`\F\` `\S\` `\T\` `\R\` `\E\`) honored at the
  domain↔wire boundary. A patient named `O^Brien-Jones` round-trips
  losslessly instead of silently splitting into a phantom component.
- New repository methods: `PatientRepository::find_by_identifier_value`
  (Postgres JSONB `@>` containment) and `AdmissionRepository::find_open_for_patient`.

### Added — FHIR R5

- `POST /fhir` accepts batch + transaction Bundles for Patient / Encounter /
  Appointment creates. Returns the matching `batch-response` /
  `transaction-response` envelope with per-entry `status` + `location`.
- **`type: transaction` is genuinely all-or-nothing**: every entry runs
  inside one `sea_orm::DatabaseTransaction`; any failure rolls back the
  whole bundle and returns `400 + OperationOutcome`. Search-index updates
  are deferred until after commit so rolled-back rows are never Tantivy-visible.
- New read endpoints: `GET /fhir/Practitioner/{id}`, `/fhir/Schedule/{id}`,
  `/fhir/Slot/{id}`, `/fhir/Location/{id}`.
- Collection Bundle: `GET /fhir/Patient?_count=N`.

### Added — Bulk interchange

- New `src/interchange/` module with a flat `PatientRow` projection shared
  by every format.
- `GET /api/patients/export.json` / `.xml` / `.tsv` / `.csv` — bulk export
  capped at 10k active patients.
- `POST /api/patients/import` — idempotent bulk import. Format picked from
  `Content-Type`. Rows whose `id` already exists are skipped. Response is
  `{ inserted, skipped, failed }`.
- Examples under `examples/`: `patients.json`, `patients.xml`, `patients.tsv`,
  `patients.csv`, `patient-single.json`, `fhir-patient.json`,
  `fhir-bundle.json`, `fhir-transaction-bundle.json`, `hl7-adt-a01.txt`,
  `hl7-adt-a02.txt`, `hl7-adt-a03.txt`, `hl7-adt-a28.txt`.
- Cargo example `cargo run --example interchange` round-trips three
  patients through all four flat formats.

### Added — OpenAPI

- `src/api/openapi.rs` aggregates a `utoipa::OpenApi` doc covering the 11
  v0.2/v0.3 endpoints across three tags (Interchange / FHIR / HL7v2).
- Swagger UI served at `/swagger-ui`, raw spec at `/api-docs/openapi.json`.
- v0.1 endpoints are not yet annotated; that is a follow-on task.

### Added — Documentation

- New `AGENTS/interchange.md` covering the `PatientRow` schema, lossy
  field notes, the four flat formats, the HL7 v2 surface (HTTP + MLLP +
  outbound), and the FHIR Bundle write semantics.
- README has an "Interchange formats" section with curl one-liners for
  every new endpoint, plus an MLLP example in Python.
- New configuration env vars: `HL7V2_MLLP_BIND`, `HL7V2_OUTBOUND_PEER`.

### Test coverage

- **296 library unit tests** (up from 228 in v0.1).
- **18 integration test binaries**, all skipping cleanly without
  `DATABASE_URL` and exercising the full surface with one.
- New integration tests: `interchange_test`, `fhir_bundle_test`,
  `hl7v2_test`, `hl7v2_mllp_test`, `hl7v2_outbound_test`.

### Deferred to a future release

- HL7 v2 batch envelope (`FHS` / `BHS`).
- HL7 v2 ORM (order entry) — clinical, out of PAS scope.
- DICOM Modality Worklists.
- Probabilistic MPI dedup (name + DOB + SSN). Exact-MRN dedup landed in
  v0.3.1; full MPI integration is still a sister-crate concern.

## [0.1.0] — 2026-05-20

Initial release. The original v0.1 implementation plan and per-task
delivery log have since been consolidated into the comprehensive
specification at [`spec.md`](spec.md).
