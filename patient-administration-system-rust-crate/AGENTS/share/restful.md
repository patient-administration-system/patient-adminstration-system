# RESTful conventions

- JSON request and response bodies; UTF-8.
- All `/api/*` responses use the envelope `{ success, data, error }` (see [../restful.md](../restful.md#response-envelope)).
- All `/fhir/*` responses return canonical FHIR resources on success and FHIR `OperationOutcome` on failure.
- HTTP status codes follow REST conventions: 200, 204, 400, 401, 404, 409, 422, 500. The mapping table lives in [../restful.md](../restful.md#http-status-code-mapping).
- Bearer-token auth (`Authorization: Bearer <token>`) is enforced when `API_TOKEN` is set; otherwise the API runs in trusted-caller mode with a startup `warn!`. `/api/health` is always exempt.
- User context for the audit log: `X-User-Id`, `X-User-Ip`, `X-User-Agent` headers.
- CORS via `tower_http::cors::CorsLayer`. Configurable origins through `CORS_ORIGINS`; empty/unset means permissive (with a startup warn).
- OpenAPI is annotated via `utoipa` on handler signatures. Wiring of `utoipa-swagger-ui` is opt-in per deployment.

The full route table lives in [../../README.md](../../README.md#api-endpoint-reference).
