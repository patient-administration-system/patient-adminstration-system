//! Patient Administration System (PAS)
//!
//! A hospital Patient Administration System built with Rust.
//!
//! This library provides the administrative system of record for healthcare
//! workflow — identity, ADT (admission/discharge/transfer), scheduling,
//! waitlists, resources (wards/beds), communications, and billing. It is
//! deliberately non-clinical: diagnoses, prescriptions, vitals and lab
//! results are out of scope.
//!
//! Capabilities:
//! - ADT state machine with transactional bed allocation
//! - Slot-based scheduling with overlap detection
//! - Waitlist + RTT (Referral-To-Treatment) clock arithmetic
//! - Ward/Room/Bed resource model with status transitions
//! - Tera-rendered patient correspondence
//! - Episode-of-care billing (accounts, charges, invoices, payments)
//! - RESTful API via Axum + OpenAPI via Utoipa
//! - HL7 FHIR R5 surface for interoperability
//! - PostgreSQL persistence via SeaORM
//! - Transactional outbox for domain events
//! - Distributed tracing and observability via OpenTelemetry

// Module declarations
pub mod adt;
pub mod api;
pub mod billing;
pub mod communication;
pub mod config;
pub mod db;
pub mod error;
pub mod hl7v2;
pub mod interchange;
pub mod models;
pub mod observability;
pub mod privacy;
pub mod resources;
pub mod scheduling;
pub mod search;
pub mod streaming;
pub mod validation;
pub mod waitlist;

// Re-exports
pub use error::{Error, Result};

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_imports() {
        // Verify key types are accessible
        let _: fn() -> crate::Result<()> = || Ok(());
    }
}
