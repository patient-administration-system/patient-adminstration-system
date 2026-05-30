//! Error types for the Patient Administration System

use thiserror::Error;

/// Result type alias for PAS operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for the Patient Administration System.
#[derive(Error, Debug)]
pub enum Error {
    #[error("Database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Invalid state transition: {0}")]
    InvalidStateTransition(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Search error: {0}")]
    Search(String),

    #[error("Streaming error: {0}")]
    Streaming(String),

    #[error("FHIR error: {0}")]
    Fhir(String),

    #[error("Render error: {0}")]
    Render(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl Error {
    /// Build a `NotFound` error.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Error::NotFound(msg.into())
    }

    /// Build a `Validation` error.
    pub fn validation(msg: impl Into<String>) -> Self {
        Error::Validation(msg.into())
    }

    /// Build an `InvalidStateTransition` error.
    pub fn invalid_transition(msg: impl Into<String>) -> Self {
        Error::InvalidStateTransition(msg.into())
    }

    /// Build a `Conflict` error.
    pub fn conflict(msg: impl Into<String>) -> Self {
        Error::Conflict(msg.into())
    }

    /// Build an `Internal` error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Error::Internal(msg.into())
    }

    /// Build a `Config` error.
    pub fn config(msg: impl Into<String>) -> Self {
        Error::Config(msg.into())
    }

    /// Build a `Render` error.
    pub fn render(msg: impl Into<String>) -> Self {
        Error::Render(msg.into())
    }
}
