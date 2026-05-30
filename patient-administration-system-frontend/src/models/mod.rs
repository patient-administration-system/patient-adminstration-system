//! Read-side projections of the PAS database tables.
//!
//! These are small, page-shaped views — we deliberately don't drag in the
//! full PAS domain types so the front-end stays decoupled from the API
//! crate. If you need richer behavior (validation, state transitions),
//! call the PAS REST API instead.

pub mod audit;
pub mod bed;
pub mod letter_template;
pub mod occupancy;
pub mod outbox;
pub mod patient;
pub mod rtt;
pub mod schedule;
pub mod ward;
