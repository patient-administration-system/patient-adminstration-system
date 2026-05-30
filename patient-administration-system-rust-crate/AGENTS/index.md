# AGENTS Directory Index

Detailed reference documentation for the Patient Administration System.

## Files

| File | Description |
|------|-------------|
| [architecture.md](architecture.md) | Layered architecture (api → service → repository → db), aggregates, transactional outbox, four state machines, time handling, soft delete, module structure, error mapping, schema summary |
| [models.md](models.md) | Domain model reference for every type under `src/models/` — fields, factory methods, state-machine methods |
| [restful.md](restful.md) | REST envelope, error-to-status mapping (REST + FHIR), auth/CORS, headline request/response shapes. Full route list lives in `../README.md`. |
| [matching.md](matching.md) | Scheduling and RTT clock reference: appointment overlap detection, slot/bed booking concurrency, RTT clock arithmetic and breach predicate. (Title kept for stable links; content is PAS-specific, not the legacy MPI matching doc.) |
| [testing.md](testing.md) | Testing strategy: 497 unit tests, 62 integration tests across 24 files (most gated on `DATABASE_URL`), three Criterion benches, quality gates |
| [interchange.md](interchange.md) | v0.2 interchange formats: bulk JSON / XML / TSV import-export, the `PatientRow` projection, and the FHIR R5 `Bundle` + Practitioner/Schedule/Slot/Location read endpoints |

## Shared files

| File | Description |
|------|-------------|
| [share/overview.md](share/overview.md) | PAS feature summary |
| [share/technology.md](share/technology.md) | Technology stack reference (single source of truth lives in the README; this is the snapshot) |
| [share/auditability.md](share/auditability.md) | Audit logging and event streaming surface |
| [share/availability.md](share/availability.md) | Availability, scaling, health checks |
| [share/observability.md](share/observability.md) | Tracing, metrics, OpenTelemetry |
| [share/privacy.md](share/privacy.md) | Data masking, GDPR export, consent |
| [share/restful.md](share/restful.md) | REST conventions |
| [share/match-search-merge.md](share/match-search-merge.md) | PAS-applicable subset of match/search/merge guidance (probabilistic matching lives in the MPI crate, not here) |

## See Also

- [../AGENTS.md](../AGENTS.md) — project-level orientation for agents
- [../README.md](../README.md) — public-facing README with the full endpoint table
- [../CHANGELOG.md](../CHANGELOG.md) — per-release changelog (currently v0.26.0)
- [../spec.md](../spec.md) — comprehensive specification-driven-development document (scope, invariants, architecture, dependency stack, build sequence, acceptance criteria)
- [../../README.md](../../README.md) — workspace-level README (PAS + patient-administration-system-frontend + migration crate)
- [../../CHANGELOG.md](../../CHANGELOG.md) — workspace-level changelog that coordinates member-crate releases
- [../../patient-administration-system-frontend/README.md](../../patient-administration-system-frontend/README.md) — Loco-rs front-end sibling crate
