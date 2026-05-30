//! FHIR R5 API surface.
//!
//! Minimal hand-rolled FHIR R5 types and conversion to/from the PAS domain
//! models. The aim for v0.1 is usable wire interop for `Patient`, `Encounter`,
//! `Appointment`, `Schedule`, `Slot`, `Practitioner`, and `Location` — not full
//! FHIR R5 compliance.
//!
//! The router built by [`routes::router`] is exposed as a standalone
//! [`axum::Router`] so callers can `merge` it into the main app. `main.rs` is
//! intentionally untouched by this module.

pub mod handlers;
pub mod operation_outcome;
pub mod resources;
pub mod routes;

pub use operation_outcome::OperationOutcome;
pub use routes::router as fhir_router;
