# Availability

- Database connection pool (`src/db/mod.rs`): max 10, min 2, 8 s connect timeout, 60 s idle timeout, `sqlx_logging` off.
- Health check: `GET /api/health` pings the database and returns `{ "status": "ok" | "degraded", "database": "ok" | "unreachable" }`. Always exempt from bearer auth **and from the rate-limit middleware** so probes stay green under load.
- Per-IP token-bucket rate limit (v0.12): default 600 req/min with burst 60. Buckets keyed by `ConnectInfo<SocketAddr>` peer IP (X-Forwarded-For is *not* trusted — terminate proxy headers upstream). Exhaustion returns 429 with `Retry-After` and a `RATE_LIMITED` error code in the standard envelope. Set `PAS_RATE_LIMIT_RPM=0` to disable entirely. Cleanup sweep evicts idle buckets after 5 min and caps the table at 50 000 entries.
- Stateless server: every request reads from the DB. Horizontal scaling is straightforward — run more replicas behind a load balancer.
- The outbox dispatcher runs in-process. If multiple replicas run, each polls; duplicate delivery is possible. Production deployments that need exactly-once delivery should either pin the dispatcher to one replica or wrap publish in an idempotent consumer.
- The optional MLLP TCP listener (`HL7V2_MLLP_BIND`) is also in-process. It dispatches via `axum::Router::oneshot` against the same router as the HTTP path, so handler logic and middleware are shared (auth excepted — see [restful.md](restful.md)).
- Docker image (`Dockerfile`) is multi-stage and runs as a non-root user.
- `docker-compose.yml` brings up Postgres + the PAS server; `docker-compose.test.yml` brings up Postgres only for the integration suite.
- Sibling `patient-administration-system-frontend` (Loco-rs, port 5150) is stateless against the same DB; deploy independently with its own replica count.
