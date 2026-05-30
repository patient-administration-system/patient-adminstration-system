// `loco_rs::Error` is intentionally a large enum (variant per backend);
// every Loco handler signature ends up tripping `clippy::result_large_err`.
// Box-everywhere isn't worth the readability cost — accept the noise.
#![allow(clippy::result_large_err)]

//! patient-administration-system-frontend — Loco-rs front-end for the Patient Administration System.
//!
//! Architecture:
//!
//! - The PAS Axum binary (`patient-administration-system`) is the system of
//!   record. It owns the REST + FHIR + HL7 v2 surface and writes to the
//!   `patients` / `encounters` / `admissions` / … tables.
//! - This crate (`patient-administration-system-frontend`) is a separate Loco-rs app that:
//!   - Renders Tera templates dressed in [Lily Design System][lily] markup.
//!   - Serves HTMX-powered live views.
//!   - Reads from the same PostgreSQL database via sea-orm.
//!   - Calls the PAS REST API for writes that go through the service layer
//!     (audit + outbox).
//!
//! The two apps share migrations via the [`migration`] crate so a single
//! `loco-cli db migrate` keeps both schemas in lock-step.
//!
//! [lily]: https://github.com/LilyDesignSystem/lily

pub mod app;
pub mod controllers;
pub mod csrf;
pub mod initializers;
pub mod models;
pub mod services;
pub mod views;
