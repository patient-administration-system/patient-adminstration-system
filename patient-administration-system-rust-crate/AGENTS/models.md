# Domain Model Reference

This document describes the domain model surface of the Patient Administration System (PAS). Models live under `src/models/` as pure Rust types with `serde` derives and no DB dependencies. The PAS is administrative: it owns identity, ADT, scheduling, waitlists, resources, communications, and billing. Clinical content (diagnoses, orders, results) is intentionally out of scope.

## Shared Value Types (`models/mod.rs`)

### `Gender`

Enum with variants `Male`, `Female`, `Other`, `Unknown`. Serialized lowercase.

### `NameUse`

Enum: `Usual`, `Official`, `Temp`, `Nickname`, `Anonymous`, `Old`, `Maiden`. FHIR-aligned name use codes.

### `AddressUse`

Enum: `Home`, `Work`, `Temp`, `Old`, `Billing`.

### `Address`

| Field         | Type                  | Description                |
| ------------- | --------------------- | -------------------------- |
| use_type      | Option\<AddressUse\>  | Address use                |
| line1         | Option\<String\>      | Street line 1              |
| line2         | Option\<String\>      | Street line 2              |
| city          | Option\<String\>      | City                       |
| state         | Option\<String\>      | State / province           |
| postal_code   | Option\<String\>      | Postal / ZIP code          |
| country       | Option\<String\>      | Country code               |

### `ContactPointSystem`

Enum: `Phone`, `Fax`, `Email`, `Pager`, `Url`, `Sms`, `Other`.

### `ContactPointUse`

Enum: `Home`, `Work`, `Temp`, `Old`, `Mobile`.

### `ContactPoint`

| Field    | Type                          | Description     |
| -------- | ----------------------------- | --------------- |
| system   | ContactPointSystem            | Channel kind    |
| value    | String                        | The value       |
| use_type | Option\<ContactPointUse\>     | Use code        |

### `Iso4217`

A newtype wrapping a `String`. Validates that the code is exactly three uppercase ASCII letters at construction (`Iso4217::new("USD")?`).

### `Money`

| Field    | Type                       | Description |
| -------- | -------------------------- | ----------- |
| amount   | `rust_decimal::Decimal`    | Amount      |
| currency | `Iso4217`                  | ISO 4217    |

**Methods:**
- `Money::new(amount, currency)`, `Money::zero(currency)`
- `Money::try_add(self, other) -> Result<Money>` — returns `Error::Validation` for currency mismatch.
- `impl Add for Money` — panics on currency mismatch; prefer `try_add` outside known-safe call sites.

### `TimeRange`

| Field | Type                  | Description |
| ----- | --------------------- | ----------- |
| start | `DateTime<Utc>`       | Inclusive   |
| end   | `DateTime<Utc>`       | Exclusive   |

**Methods:**
- `is_valid()` — `start < end`.
- `overlaps(&other)` — half-open interval overlap (touching ranges do not overlap).

## Patient (`models/patient.rs`)

### `HumanName`

| Field    | Type                  | Description           |
| -------- | --------------------- | --------------------- |
| use_type | Option\<NameUse\>     | Name use code         |
| family   | String                | Family / last name    |
| given    | Vec\<String\>         | Given names           |
| prefix   | Vec\<String\>         | Prefixes (Dr., Mr.)   |
| suffix   | Vec\<String\>         | Suffixes (Jr., III)   |

### `EmergencyContact`

| Field         | Type                    | Description                  |
| ------------- | ----------------------- | ---------------------------- |
| name          | String                  | Contact name                 |
| relationship  | String                  | Relationship                 |
| telecom       | Vec\<ContactPoint\>     | Contact points               |
| address       | Option\<Address\>       | Contact address              |
| is_primary    | bool                    | Primary contact flag         |

### `Patient`

| Field                 | Type                          | Description                              |
| --------------------- | ----------------------------- | ---------------------------------------- |
| id                    | Uuid                          | Internal PAS identifier                  |
| mpi_id                | Option\<Uuid\>                | MPI identity reference (out-of-band)     |
| identifiers           | Vec\<Identifier\>             | External identifiers                     |
| active                | bool                          | Whether the record is active             |
| name                  | HumanName                     | Primary name                             |
| additional_names      | Vec\<HumanName\>              | Aliases, former names                    |
| telecom               | Vec\<ContactPoint\>           | Phone / email / fax contacts             |
| gender                | Gender                        |                                          |
| birth_date            | Option\<NaiveDate\>           | Date of birth                            |
| addresses             | Vec\<Address\>                | Physical addresses                       |
| deceased              | bool                          | Deceased flag                            |
| deceased_datetime     | Option\<DateTime\<Utc\>\>     | Date/time of death                       |
| emergency_contacts    | Vec\<EmergencyContact\>       | Emergency contacts                       |
| marital_status        | Option\<String\>              | Free-text marital status                 |
| created_at            | DateTime\<Utc\>               |                                          |
| updated_at            | DateTime\<Utc\>               |                                          |

**Methods:**
- `Patient::new(name, gender) -> Self` — generates UUID and timestamps; active=true.
- `Patient::full_name() -> String` — `"<given joined> <family>"`.

## Identifier (`models/identifier.rs`)

### `IdentifierUse`

Enum: `Usual`, `Official`, `Temp`, `Secondary`, `Old`.

### `IdentifierType`

Enum, serialized UPPERCASE: `MRN`, `NHS`, `NIR`, `TSI`, `IHI`, `HCN`, `SSN`, `DL`, `Passport`, `Other`.

The five national-government healthcare identifiers (`NHS`, `NIR`, `TSI`, `IHI`, `HCN`) carry per-country format rules — see [`../spec.md` §4.0.1](../spec.md) and the matching free functions in [`crate::validation`].

### `Identifier`

| Field            | Type                      | Description                       |
| ---------------- | ------------------------- | --------------------------------- |
| use_type         | Option\<IdentifierUse\>   |                                   |
| identifier_type  | IdentifierType            | Type code                         |
| system           | String                    | Issuing system URI                |
| value            | String                    | Identifier value                  |
| assigner         | Option\<String\>          | Assigning authority               |

**Factory methods** (one per supported national scheme; each fixes the system URI to the canonical value below):

| Factory                               | Country / scheme                                | System URI                                            |
| ------------------------------------- | ----------------------------------------------- | ----------------------------------------------------- |
| `Identifier::mrn(system, value)`      | (facility-local Medical Record Number)          | per-facility (caller-supplied)                        |
| `Identifier::nhs(value)`              | United Kingdom NHS Number                       | `https://fhir.nhs.uk/Id/nhs-number`                   |
| `Identifier::nir(value)`              | France Numéro d'Identification au Répertoire    | `urn:oid:1.2.250.1.213.1.4.8`                         |
| `Identifier::tsi(value)`              | España Tarjeta Sanitaria Individual / SNS CIP   | `urn:oid:2.16.724.4.40`                               |
| `Identifier::ihi(value)`              | Ireland Individual Health Identifier            | `https://fhir.hl7.ie/Id/individual-health-identifier` |
| `Identifier::hcn(value)`              | Northern Ireland Health & Care Number           | `https://fhir.hscni.net/Id/hcn`                       |
| `Identifier::ssn(value)`              | United States Social Security Number            | `http://hl7.org/fhir/sid/us-ssn`                      |

The system URIs are also exported as public constants
(`NHS_SYSTEM_URI`, `NIR_SYSTEM_URI`, `TSI_SYSTEM_URI`, `IHI_SYSTEM_URI`,
`HCN_SYSTEM_URI`, `SSN_SYSTEM_URI`) so callers building `Identifier`
literals against the same scheme stay byte-identical.

## Workforce (`models/practitioner.rs`)

### `Practitioner`

| Field        | Type                       | Description     |
| ------------ | -------------------------- | --------------- |
| id           | Uuid                       |                 |
| identifiers  | Vec\<Identifier\>          |                 |
| active       | bool                       |                 |
| name         | HumanName                  |                 |
| telecom      | Vec\<ContactPoint\>        |                 |
| addresses    | Vec\<Address\>             |                 |
| gender       | Gender                     |                 |
| birth_date   | Option\<NaiveDate\>        |                 |
| created_at   | DateTime\<Utc\>            |                 |
| updated_at   | DateTime\<Utc\>            |                 |

`Practitioner::new(name, gender)` creates an active record.

### `PractitionerRole`

| Field            | Type             | Description                       |
| ---------------- | ---------------- | --------------------------------- |
| id               | Uuid             |                                   |
| practitioner_id  | Uuid             | Owning practitioner               |
| department_id    | Uuid             | Department                        |
| role             | String           | e.g. "Attending Physician"        |
| specialty        | Option\<String\> | e.g. "Cardiology"                 |
| active           | bool             |                                   |
| created_at       | DateTime\<Utc\>  |                                   |
| updated_at       | DateTime\<Utc\>  |                                   |

### `Department`

| Field        | Type             | Description    |
| ------------ | ---------------- | -------------- |
| id           | Uuid             |                |
| facility_id  | Uuid             | Owning facility|
| name         | String           |                |
| code         | String           | Local code     |
| active       | bool             |                |
| created_at   | DateTime\<Utc\>  |                |
| updated_at   | DateTime\<Utc\>  |                |

## Resources (`models/facility.rs`)

Hierarchy: `Facility` → `Ward` → `Room` → `Bed`.

### `Facility`

| Field      | Type             |
| ---------- | ---------------- |
| id         | Uuid             |
| name       | String           |
| code       | String           |
| address    | Address          |
| active     | bool             |
| created_at | DateTime\<Utc\>  |
| updated_at | DateTime\<Utc\>  |

### `Ward`

| Field        | Type             |
| ------------ | ---------------- |
| id           | Uuid             |
| facility_id  | Uuid             |
| name         | String           |
| code         | String           |
| capacity     | u32              |
| active       | bool             |
| created_at   | DateTime\<Utc\>  |
| updated_at   | DateTime\<Utc\>  |

### `Room`

| Field      | Type             |
| ---------- | ---------------- |
| id         | Uuid             |
| ward_id    | Uuid             |
| name       | String           |
| code       | String           |
| active     | bool             |
| created_at | DateTime\<Utc\>  |
| updated_at | DateTime\<Utc\>  |

### `Bed`

| Field      | Type             | Description           |
| ---------- | ---------------- | --------------------- |
| id         | Uuid             |                       |
| room_id    | Uuid             |                       |
| name       | String           |                       |
| code       | String           |                       |
| status     | BedStatus        | (state machine)       |
| created_at | DateTime\<Utc\>  |                       |
| updated_at | DateTime\<Utc\>  |                       |

### `BedStatus`

Enum: `Available`, `Occupied`, `Reserved`, `OutOfService`, `Cleaning`.

**Method:** `BedStatus::try_transition_to(self, next) -> Result<BedStatus>`. Valid transitions:

- `Available` → `Occupied | Reserved | OutOfService`
- `Occupied` → `Cleaning | OutOfService`
- `Cleaning` → `Available | OutOfService`
- `Reserved` → `Occupied | Available | OutOfService`
- `OutOfService` → `Available`

Same-state transitions are rejected.

## Encounter (`models/encounter.rs`)

### `EncounterClass`

Enum: `Outpatient`, `Inpatient`, `Emergency`, `DayCase`, `HomeCare`, `Virtual`.

### `EncounterStatus`

Enum: `Planned`, `Arrived`, `InProgress`, `OnLeave`, `Finished`, `Cancelled`.

**Method:** `EncounterStatus::try_transition_to(self, next) -> Result<EncounterStatus>`. Valid transitions:

- `Planned` → `Arrived | Cancelled`
- `Arrived` → `InProgress | Cancelled`
- `InProgress` → `OnLeave | Finished | Cancelled`
- `OnLeave` → `InProgress | Finished | Cancelled`
- `Finished`, `Cancelled` are terminal.

### `Encounter`

| Field              | Type                          | Description                                        |
| ------------------ | ----------------------------- | -------------------------------------------------- |
| id                 | Uuid                          |                                                    |
| patient_id         | Uuid                          |                                                    |
| class              | EncounterClass                |                                                    |
| status             | EncounterStatus               | (state machine)                                    |
| period_start       | DateTime\<Utc\>               | Effective start                                    |
| period_end         | Option\<DateTime\<Utc\>\>     | Effective end (set on Finished/Cancelled)          |
| practitioner_id    | Option\<Uuid\>                |                                                    |
| department_id      | Option\<Uuid\>                |                                                    |
| reason             | Option\<String\>              | Free-text reason                                   |
| created_at         | DateTime\<Utc\>               |                                                    |
| updated_at         | DateTime\<Utc\>               |                                                    |

`Encounter::new(patient_id, class)` opens a new `Planned` encounter.

## Admission (`models/admission.rs`)

### `Admission`

| Field                       | Type                          | Description                |
| --------------------------- | ----------------------------- | -------------------------- |
| id                          | Uuid                          |                            |
| encounter_id                | Uuid                          | Owning encounter           |
| bed_id                      | Uuid                          | Initial bed                |
| admitting_practitioner_id   | Option\<Uuid\>                |                            |
| admitted_at                 | DateTime\<Utc\>               | Effective time             |
| reason                      | Option\<String\>              |                            |
| created_at                  | DateTime\<Utc\>               |                            |
| updated_at                  | DateTime\<Utc\>               |                            |

### `Transfer`

| Field           | Type                 |
| --------------- | -------------------- |
| id              | Uuid                 |
| admission_id    | Uuid                 |
| from_bed_id     | Uuid                 |
| to_bed_id       | Uuid                 |
| reason          | Option\<String\>     |
| transferred_at  | DateTime\<Utc\>      |
| created_at      | DateTime\<Utc\>      |

### `Discharge`

| Field                          | Type                  |
| ------------------------------ | --------------------- |
| id                             | Uuid                  |
| admission_id                   | Uuid                  |
| discharging_practitioner_id    | Option\<Uuid\>        |
| discharged_at                  | DateTime\<Utc\>       |
| disposition                    | Option\<String\>      |
| notes                          | Option\<String\>      |
| created_at                     | DateTime\<Utc\>       |

### `BedAssignment`

Tracks the bed occupation interval for one encounter. `released_at = None` is the currently active row (at most one per bed; enforced by the partial unique index).

| Field          | Type                          |
| -------------- | ----------------------------- |
| id             | Uuid                          |
| encounter_id   | Uuid                          |
| bed_id         | Uuid                          |
| assigned_at    | DateTime\<Utc\>               |
| released_at    | Option\<DateTime\<Utc\>\>     |
| created_at     | DateTime\<Utc\>               |
| updated_at     | DateTime\<Utc\>               |

**Methods:**
- `release(&mut self)` — sets `released_at` and `updated_at` to now.
- `is_active(&self) -> bool` — `released_at.is_none()`.

## Schedule and Slot (`models/schedule.rs`)

### `ScheduleOwner`

Tagged enum: `Practitioner(Uuid)`, `Bed(Uuid)`, `Room(Uuid)`. Serialized with `kind` + `id` fields.

### `Schedule`

| Field         | Type             |
| ------------- | ---------------- |
| id            | Uuid             |
| owner         | ScheduleOwner    |
| service_type  | String           |
| active        | bool             |
| created_at    | DateTime\<Utc\>  |
| updated_at    | DateTime\<Utc\>  |

### `SlotStatus`

Enum: `Free`, `Busy`, `BlockedOut`.

**Method:** `SlotStatus::try_transition_to`. Valid transitions: `Free` ↔ `Busy`, `Free` ↔ `BlockedOut`. Same-state and `Busy ↔ BlockedOut` are rejected.

### `Slot`

| Field           | Type                |
| --------------- | ------------------- |
| id              | Uuid                |
| schedule_id     | Uuid                |
| start_datetime  | DateTime\<Utc\>     |
| end_datetime    | DateTime\<Utc\>     |
| status          | SlotStatus          |
| created_at      | DateTime\<Utc\>     |
| updated_at      | DateTime\<Utc\>     |

**Method:** `Slot::duration() -> chrono::Duration`.

## Appointment (`models/appointment.rs`)

### `AppointmentStatus`

Enum: `Proposed`, `Booked`, `Arrived`, `Fulfilled`, `Cancelled`, `NoShow`.

**Method:** `AppointmentStatus::try_transition_to`. Valid transitions:

- `Proposed` → `Booked | Cancelled`
- `Booked` → `Arrived | Cancelled | NoShow`
- `Arrived` → `Fulfilled | Cancelled`
- `Fulfilled`, `Cancelled`, `NoShow` are terminal.

### `CancellationReason`

Enum: `PatientRequest`, `ProviderRequest`, `NoShow`, `Rescheduled`, `Other`.

### `Appointment`

| Field                      | Type                                | Description                                |
| -------------------------- | ----------------------------------- | ------------------------------------------ |
| id                         | Uuid                                |                                            |
| patient_id                 | Uuid                                |                                            |
| slot_id                    | Option\<Uuid\>                      | Bound Slot, if any                         |
| practitioner_id            | Option\<Uuid\>                      |                                            |
| start_datetime             | DateTime\<Utc\>                     | Effective start                            |
| end_datetime               | DateTime\<Utc\>                     | Effective end                              |
| status                     | AppointmentStatus                   |                                            |
| reason                     | Option\<String\>                    |                                            |
| from_waitlist_entry_id     | Option\<Uuid\>                      | Provenance, if booked off the waitlist     |
| cancellation_reason        | Option\<CancellationReason\>        |                                            |
| created_at                 | DateTime\<Utc\>                     |                                            |
| updated_at                 | DateTime\<Utc\>                     |                                            |

## Waitlist (`models/waitlist.rs`)

### `Priority`

Enum with derived `Ord`: `Routine < Urgent < TwoWeekWait < Emergency`. Sort respects declaration order. Serialized snake_case (`two_week_wait`).

### `WaitlistStatus`

Enum: `Waiting`, `Booked`, `Removed`, `Treated`.

### `Referral`

| Field                       | Type                          |
| --------------------------- | ----------------------------- |
| id                          | Uuid                          |
| patient_id                  | Uuid                          |
| referring_practitioner_id   | Option\<Uuid\>                |
| target_service              | String                        |
| reason                      | Option\<String\>              |
| received_at                 | DateTime\<Utc\>               |
| created_at                  | DateTime\<Utc\>               |

### `WaitlistEntry`

| Field           | Type                |
| --------------- | ------------------- |
| id              | Uuid                |
| referral_id     | Option\<Uuid\>      |
| patient_id      | Uuid                |
| target_service  | String              |
| priority        | Priority            |
| status          | WaitlistStatus      |
| created_at      | DateTime\<Utc\>     |
| updated_at      | DateTime\<Utc\>     |

**Method:** `days_waiting(now) -> i64` — floor of days since `created_at`.

## RTT (`models/rtt.rs`)

### `RTTStatus`

Enum: `Active`, `Paused`, `Stopped`.

### `RTTEventKind`

Enum: `Started`, `Paused`, `Resumed`, `Stopped`.

### `RTTPathway`

| Field             | Type                          | Description                          |
| ----------------- | ----------------------------- | ------------------------------------ |
| id                | Uuid                          |                                      |
| patient_id        | Uuid                          |                                      |
| target_service    | String                        |                                      |
| breach_weeks      | u32                           | Per-pathway threshold (default 18)   |
| status            | RTTStatus                     |                                      |
| started_at        | DateTime\<Utc\>               |                                      |
| stopped_at        | Option\<DateTime\<Utc\>\>     |                                      |
| created_at        | DateTime\<Utc\>               |                                      |
| updated_at        | DateTime\<Utc\>               |                                      |

Constants: `RTTPathway::DEFAULT_BREACH_WEEKS = 18`.

**Method:** `is_breaching(events, now) -> bool` — `compute_active_weeks(...) > breach_weeks`.

### `RTTClockEvent`

| Field        | Type                |
| ------------ | ------------------- |
| id           | Uuid                |
| pathway_id   | Uuid                |
| kind         | RTTEventKind        |
| reason       | Option\<String\>    |
| event_at     | DateTime\<Utc\>     |
| created_at   | DateTime\<Utc\>     |

**Factory methods:** `started(pathway_id)`, `paused(pathway_id, reason)`, `resumed(pathway_id)`, `stopped(pathway_id, reason)`.

### `compute_active_weeks(events, now) -> u32`

Free function. Sums the active (unpaused) intervals from `Started`/`Resumed` to `Paused`/`Stopped` (or to `now`, if the clock is still open at the end of the list). Returns the total floored to whole weeks. See `AGENTS/matching.md` for the full algorithm.

## Communication (`models/communication.rs`)

### `DeliveryChannel`

Enum: `Print`, `Email`, `Sms`. `Sms` is declared for completeness but is not actually delivered in v0.1 (only `Print` and `Email` are wired).

### `LetterStatus`

Enum: `Pending`, `Sent`, `Failed`.

### `LetterTemplate`

| Field                | Type                          | Description                       |
| -------------------- | ----------------------------- | --------------------------------- |
| id                   | Uuid                          |                                   |
| name                 | String                        |                                   |
| subject              | String                        | Tera-rendered                     |
| body_tera            | String                        | Tera source                       |
| required_variables   | Vec\<String\>                 | Must be present in render context |
| channels             | Vec\<DeliveryChannel\>        | Allowed channels                  |
| active               | bool                          |                                   |
| created_at           | DateTime\<Utc\>               |                                   |
| updated_at           | DateTime\<Utc\>               |                                   |

### `GeneratedLetter`

| Field             | Type                          |
| ----------------- | ----------------------------- |
| id                | Uuid                          |
| template_id       | Uuid                          |
| patient_id        | Uuid                          |
| appointment_id    | Option\<Uuid\>                |
| rendered_subject  | String                        |
| rendered_body     | String                        |
| channel           | DeliveryChannel               |
| status            | LetterStatus                  |
| sent_at           | Option\<DateTime\<Utc\>\>     |
| created_at        | DateTime\<Utc\>               |
| updated_at        | DateTime\<Utc\>               |

**Methods:** `mark_sent(&mut self)`, `mark_failed(&mut self)`.

## Billing (`models/billing.rs`)

### `AccountStatus`

Enum: `Open`, `Closed`.

### `InvoiceStatus`

Enum: `Draft`, `Finalized`, `Paid`, `PartiallyPaid`, `Void`.

### `PaymentMethod`

Enum: `Cash`, `Card`, `BankTransfer`, `Insurance`, `Other`.

### `Account`

| Field        | Type                          | Description                       |
| ------------ | ----------------------------- | --------------------------------- |
| id           | Uuid                          |                                   |
| patient_id   | Uuid                          |                                   |
| status       | AccountStatus                 |                                   |
| currency     | Iso4217                       | Fixed at account creation         |
| opened_at    | DateTime\<Utc\>               |                                   |
| closed_at    | Option\<DateTime\<Utc\>\>     |                                   |
| created_at   | DateTime\<Utc\>               |                                   |
| updated_at   | DateTime\<Utc\>               |                                   |

Invariant: one `Open` account per patient at a time.

### `Charge`

| Field          | Type                          | Description                       |
| -------------- | ----------------------------- | --------------------------------- |
| id             | Uuid                          |                                   |
| account_id     | Uuid                          |                                   |
| encounter_id   | Option\<Uuid\>                | Originating encounter, if any     |
| appointment_id | Option\<Uuid\>                | Originating appointment, if any   |
| code           | String                        | Billing code (CPT/HCPCS/local)    |
| description    | String                        |                                   |
| amount         | Money                         | Must match account currency       |
| posted_at      | DateTime\<Utc\>               |                                   |
| created_at     | DateTime\<Utc\>               |                                   |

### `Invoice`

| Field          | Type                          | Description                            |
| -------------- | ----------------------------- | -------------------------------------- |
| id             | Uuid                          |                                        |
| account_id     | Uuid                          |                                        |
| status         | InvoiceStatus                 |                                        |
| total          | Money                         | Sum of referenced charges              |
| charge_ids     | Vec\<Uuid\>                   | Snapshot of charges at finalization    |
| finalized_at   | Option\<DateTime\<Utc\>\>     |                                        |
| created_at     | DateTime\<Utc\>               |                                        |
| updated_at     | DateTime\<Utc\>               |                                        |

**Method:** `finalize(&mut self) -> Result<()>` — only valid from `Draft`; sets `Finalized` and `finalized_at`.

### `Payment`

| Field      | Type                  | Description                  |
| ---------- | --------------------- | ---------------------------- |
| id         | Uuid                  |                              |
| invoice_id | Uuid                  |                              |
| amount     | Money                 |                              |
| method     | PaymentMethod         |                              |
| reference  | Option\<String\>      | External txn id, check #     |
| posted_at  | DateTime\<Utc\>       |                              |
| created_at | DateTime\<Utc\>       |                              |

## Consent (`models/consent.rs`)

### `ConsentType`

Enum: `DataProcessing`, `DataSharing`, `Marketing`, `Research`, `EmergencyAccess`.

### `ConsentStatus`

Enum: `Active`, `Revoked`, `Expired`.

### `Consent`

| Field          | Type                  |
| -------------- | --------------------- |
| id             | Uuid                  |
| patient_id     | Uuid                  |
| consent_type   | ConsentType           |
| status         | ConsentStatus         |
| granted_date   | NaiveDate             |
| expiry_date    | Option\<NaiveDate\>   |
| revoked_date   | Option\<NaiveDate\>   |
| purpose        | Option\<String\>      |
| method         | Option\<String\>      |
| created_at     | DateTime\<Utc\>       |
| updated_at     | DateTime\<Utc\>       |

**Method:** `is_active(today: NaiveDate) -> bool`. Returns `false` if status is not `Active`, `granted_date > today`, or an `expiry_date` is set and is strictly less than `today`. Note the boundary rule: on the expiry day itself, the consent is still active.

## Database Entities

SeaORM entity modules under `src/db/entities/`, one file per PostgreSQL table (29 entities). Domain models above are converted to and from SeaORM entities at the repository boundary. The authoritative schema is `migrations/src/m20260520_000001_init.rs`; see [architecture.md](architecture.md#database-schema) for the table grouping and key indexes.

## See Also

- [architecture.md](architecture.md) — layers, aggregates, state machines, time, schema
- [restful.md](restful.md) — how these models cross the wire
- [matching.md](matching.md) — `compute_active_weeks`, `TimeRange::overlaps`, booking concurrency
- [testing.md](testing.md) — model test coverage
