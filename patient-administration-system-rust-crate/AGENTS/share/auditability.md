# Auditability

## Audit log

Every state-changing API call writes an `audit_log` row in the same transaction as the entity change. The row carries:

- `entity_type`, `entity_id`, `action`
- `old_value` and `new_value` as JSONB
- `user_id`, `user_ip`, `user_agent` (from `X-User-Id` / `X-User-Ip` / `X-User-Agent` headers, recorded as-is)
- `at` timestamp

## Audit query endpoints

- `GET /api/patients/:id/audit` — per-patient history.
- `GET /api/audit/recent?limit=N` — recent activity, capped at 500.
- `GET /api/audit/entity?entity_type=…&entity_id=…&limit=N` — history for any entity (encounter, charge, consent, etc.).

## Event streaming

Every state change also writes an `outbox_events` row in the same transaction. A background dispatcher (`src/streaming/dispatcher.rs`, spawned in `main.rs` at startup, 2 s poll, batch 64) forwards unpublished rows to the configured `EventPublisher` and marks them `published = true` on success. Failed publishes are logged and retried; rows are never deleted. Built-in publishers as of v0.14: `Hl7v2MllpPublisher` (outbound ADT), `WebhookEventPublisher` (HTTP POST with optional HMAC-SHA256 signing via `PAS_WEBHOOK_SECRET`), `InMemoryEventPublisher` (tests), and `CompositePublisher` fan-out when more than one backend is configured.

Event types include `PatientCreated/Updated/Deleted`, `EncounterAdmitted/Transferred/Discharged/Cancelled`, `AppointmentBooked/Cancelled/CheckedIn/Completed/NoShow`, `WaitlistAdded/Removed`, `RTTClockStarted/Paused/Resumed/Stopped`, `BedStatusChanged`, `LetterGenerated`, `ChargePosted`, `InvoiceFinalized`, `PaymentPosted`.

## Operator visibility

- `GET /api/admin/outbox/unpublished?limit=N` — currently-undelivered outbox rows for diagnostics.
- `GET /api/admin/outbox/dead-letters?limit=N` (v0.5) — events that exceeded `PAS_OUTBOX_MAX_RETRIES` consecutive failed publishes and were moved out of `outbox_events`. Newest first; capped at 500. Each row carries the original payload + `event_type`, the final `retry_count`, and the last error string from the publisher.
- `POST /api/admin/outbox/dead-letters/{id}/replay` (v0.5) — atomically re-inserts the event into `outbox_events` with `retry_count = 0` and deletes the DLQ row. Returns `{ dead_letter_id, new_outbox_id }`. The replay itself writes an `audit_log` row (`entity_type = "outbox_dead_letter"`, `action = "replay"`) so the operator action is traceable.
- `GET /dashboard` — ops view (Tera + HTMX). The "outbox unpublished" panel is the same data, rendered for humans; the "recent audit" panel surfaces the last 10 `audit_log` rows.

See [../architecture.md](../architecture.md#transactional-outbox) for the transactional invariant; see [../../spec.md](../../spec.md#63-transactional-outbox) §6.3 for the v0.5 dead-letter rule.
