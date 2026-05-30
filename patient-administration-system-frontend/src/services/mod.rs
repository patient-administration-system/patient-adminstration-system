//! Outbound service clients.
//!
//! When patient-administration-system-frontend needs to *mutate* PAS data, it calls the PAS Axum
//! REST API (so audit + outbox stay in the system of record) rather than
//! writing to the shared database directly.

pub mod pas_api;
