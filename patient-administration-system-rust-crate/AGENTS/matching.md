# Scheduling and RTT Clock Reference

> Filename kept as `matching.md` for stable links; content is PAS-specific (scheduling overlap, booking concurrency, RTT clock), not the legacy MPI fuzzy-matching reference.

The PAS has no fuzzy patient matcher — that lives in the sister MPI crate. The PAS does, however, have several pieces of non-trivial time arithmetic and concurrency control worth their own reference: scheduling overlap detection, slot-booking concurrency, bed allocation concurrency, and the RTT (Referral-To-Treatment) clock.

## Scheduling Overlap Detection

A `Slot` is a pre-generated `(schedule_id, start_datetime, end_datetime, status)` window. An `Appointment` is bound to at most one Slot and inherits its time range.

### Per-patient overlap

The `appointment` repository exposes `find_overlapping_for_patient(conn, patient_id, start, end) -> Vec<Appointment>`. The check is per-patient: a clinician can have two patients at 10:00, but one patient cannot have two appointments at 10:00.

Semantics: an existing appointment overlaps a candidate range iff their `[start_datetime, end_datetime)` intervals intersect, ignoring appointments in terminal `Cancelled` or `NoShow` status. Intervals are half-open: an appointment ending at 10:00 does not overlap one starting at 10:00.

The implemented SeaORM predicate is the standard interval-overlap form:

```sql
SELECT … FROM appointments
WHERE patient_id = $1
  AND deleted_at IS NULL
  AND status NOT IN ('cancelled', 'no_show')
  AND start_datetime < $end
  AND end_datetime   > $start;
```

(`existing.start < new.end AND existing.end > new.start` is the canonical half-open overlap predicate; equivalent to a `tsrange && tsrange` test.) The in-memory equivalent in `models::TimeRange::overlaps` follows the same half-open semantics.

### Overlap behaviour

v0.1 always **blocks**: an overlap returns `Error::Conflict` (HTTP 409) and the booking aborts. There is no "warn-only" mode — the plan's configurable warn-vs-block has not been implemented because no deployment has asked for it. The check happens before any state mutation, so a rejected booking leaves the slot status untouched.

## Slot Booking Concurrency

Booking a slot must be safe against concurrent bookers (two front-desk clerks both trying to claim 9:00 with Dr. Smith). The booking flow in `src/scheduling/` runs inside a single DB transaction:

1. `SELECT … FROM slots WHERE id = $1 FOR UPDATE`. The row lock blocks other transactions until commit.
2. Assert `slot.status == Free`. If the status is `Busy` or `BlockedOut`, return `Error::Conflict`.
3. Run the per-patient overlap check above.
4. Transition `slot.status` to `Busy` via `SlotStatus::try_transition_to(Busy)`.
5. Insert the `Appointment` row (status `Booked`).
6. Write the audit log row and the outbox event row in the same transaction.
7. Commit.

Cancellation reverses the slot status: `Appointment.status = Cancelled` and `Slot.status = Free` happen in one transaction.

## Bed Allocation Concurrency

Bed allocation follows the same locking pattern as slot booking. The ADT service in `src/adt/`:

1. Opens a transaction.
2. Inserts the `Encounter` (status `InProgress`, class `Inpatient`).
3. `SELECT … FROM beds WHERE id = $1 FOR UPDATE` on the target bed.
4. Asserts `bed.status == Available`. If not, returns `Error::Conflict`.
5. Transitions the bed via `BedStatus::try_transition_to(Occupied)`.
6. Inserts an active `BedAssignment` row (`released_at = NULL`).
7. Inserts the `Admission` row.
8. Writes the audit log row and the outbox event row.
9. Commits.

The partial unique index `bed_assignments (bed_id) WHERE released_at IS NULL` makes the "at most one active assignment per bed" invariant enforced by the database, not just by the application.

Transfers follow the same pattern but additionally close the old assignment (`released_at = now`) and set the old bed to `Cleaning`. Discharges close the active assignment and set the bed to `Cleaning`.

There are no silent fallbacks. If the target bed is not `Available`, the operation fails. The PAS never auto-picks a substitute bed.

## RTT Clock Arithmetic

An `RTTPathway` is the regulated wait-time clock for a patient on a target service. The pathway has a status (`Active`, `Paused`, `Stopped`) and an append-only list of `RTTClockEvent` rows.

### Event log invariants

`RTTClockEvent` rows are append-only and ordered by `event_at`. Each event has a `kind`:

- `Started`: opens the pathway. Exactly one, the first event.
- `Paused(reason)`: pauses the clock. Subsequent unpaused time does not count until `Resumed`.
- `Resumed`: resumes the clock. Pairs with the preceding `Paused`.
- `Stopped(reason)`: terminates the pathway. No further events allowed.

Ordering invariants enforced by `src/validation/`:

- `event_at` is monotonically non-decreasing per pathway.
- `Started` is the first event.
- `Stopped`, if present, is the last event.
- `Paused` and `Resumed` alternate, starting with `Paused`.

### `compute_active_weeks`

```rust
pub fn compute_active_weeks(
    events: &[RTTClockEvent],
    now: DateTime<Utc>,
) -> u32;
```

Defined in `src/models/rtt.rs`. Given a chronologically-ordered slice of events and a reference instant `now`, it returns the total active-but-not-paused time in whole weeks (floor).

Algorithm:

```
total_active_seconds = 0
active_since         = None
for ev in events sorted by event_at:
    match ev.kind:
        Started | Resumed:
            active_since = Some(ev.event_at)
        Paused | Stopped:
            if let Some(start) = active_since.take():
                total_active_seconds += (ev.event_at - start).num_seconds()
if let Some(start) = active_since:
    total_active_seconds += (now - start).num_seconds()
return floor(total_active_seconds / SECONDS_PER_WEEK)
```

Edge cases handled:

- Empty event slice → 0.
- A `Started` with no `Stopped`/`Paused` accrues time to `now`.
- A `Stopped` event freezes the clock; later `now` values do not extend the total.
- Negative totals (clock skew between events) are clamped to 0.

### Breach predicate

```rust
pathway.is_breaching(&events, now)
    == compute_active_weeks(&events, now) > pathway.breach_weeks
```

The comparison is **strict** (`>`). At exactly `breach_weeks`, the pathway is not yet breaching; only one week beyond does it breach.

The breach threshold is **per pathway**. The default is `RTTPathway::DEFAULT_BREACH_WEEKS = 18` (the NHS Referral-To-Treatment standard) but it is intentionally a column, not a hardcoded constant — other regulators and other services may use different limits, and a deployment can override the default per pathway at creation time.

### Booking off a waitlist

When an `Appointment` is booked off a `WaitlistEntry`, the resulting `Appointment.from_waitlist_entry_id` is set for traceability, and a `Stopped` RTT event with an appropriate reason (e.g. `FirstDefinitiveTreatment`) may be appended by the booking service. The PAS does not auto-stop the clock — that decision is operational and is left to an explicit API call.

## Source Files

- `src/scheduling/` — slot booking, per-patient overlap detection, status transitions.
- `src/adt/` — bed allocation, transfer, discharge.
- `src/waitlist/` — waitlist queue, RTT clock arithmetic, breach predicate.
- `src/validation/` — RTT clock-event ordering validator, appointment time validator.
- `src/models/rtt.rs` — `compute_active_weeks` (free function), `RTTPathway::is_breaching`.
- `src/models/mod.rs` — `TimeRange::overlaps` (in-memory half-open overlap).
- `benches/scheduling_bench.rs` — `TimeRange::overlaps` perf at 16…1024 ranges.
- `benches/waitlist_bench.rs` — `compute_active_weeks` perf at 4…256 events.
- `tests/concurrency_test.rs` — concurrent admit + concurrent book races.

## See Also

- [architecture.md](architecture.md) — outbox, state machines, transaction boundaries
- [models.md](models.md) — `Slot`, `Appointment`, `BedStatus`, `RTTPathway`, `RTTClockEvent`
- [restful.md](restful.md) — booking and RTT request shapes
