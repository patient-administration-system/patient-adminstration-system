# Privacy

## Masking

`src/privacy/` provides `mask_value`, `mask_patient`, and the masked-view endpoint. Masked fields include phone-number middle digits, postal-code suffix, email local-part, address line 1, and identifier tails.

- `GET /api/patients/:id/masked` returns the patient with PII masked.

## GDPR export

- `GET /api/patients/:id/export` returns a JSON dump of the patient and all related rows (encounters, appointments, consents, etc.) — suitable as a subject-access response.

## Consent

Domain model: `Consent { id, patient_id, consent_type, status, granted_date, expiry_date, revoked_date, purpose, method, … }`.

- `ConsentType`: `DataProcessing`, `DataSharing`, `Marketing`, `Research`, `EmergencyAccess`.
- `ConsentStatus`: `Active`, `Revoked`, `Expired`.
- `Consent::is_active(today)` returns `false` if status is not `Active`, `granted_date > today`, or `expiry_date < today`. On the expiry day itself the consent is still active.

Endpoints:

- `POST /api/patients/:id/consents` — create.
- `GET /api/patients/:id/consents` — list.
- `POST /api/consents/:id/revoke` — flip to `Revoked` and stamp `revoked_date`.

All consent writes record audit-log entries.

`require_consent(patient_id, ConsentType, …)` exists as a helper for handlers that should refuse without active consent. It is not yet wired into every handler — call sites are added when a flow needs the gate.
