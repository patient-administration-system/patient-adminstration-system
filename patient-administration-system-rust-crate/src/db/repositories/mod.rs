//! Repositories — one per aggregate.
//!
//! Each repository is a plain struct with associated `async` functions that
//! accept `&C: ConnectionTrait` so they work with either a
//! `DatabaseConnection` or a `DatabaseTransaction`. No traits — keep it
//! simple and idiomatic.
//!
//! Conversion between DB rows (which use `serde_json::Value` for JSONB,
//! `String` for money amounts, and string enums) and the rich domain models
//! is encapsulated in each module's private helpers.

pub mod admission;
pub mod appointment;
pub mod appointment_series;
pub mod audit;
pub mod bed;
pub mod billing;
pub mod consent;
pub mod coverage;
pub mod encounter;
pub mod letter;
pub mod outbox;
pub mod patient;
pub mod practitioner;
pub mod rtt;
pub mod schedule;
pub mod slot;
pub mod waitlist;

pub use audit::{AuditLogRepository, UserContext};
pub use outbox::{DeadLetterRepository, OutboxRepository};
