//! REST API
pub mod auth;
pub mod handlers;
pub mod rate_limit;
pub mod routes;
pub mod state;

pub use auth::{RequireBearerToken, require_bearer};
pub use rate_limit::{RateLimitConfig, RateLimiter, rate_limit_middleware};
pub use routes::router;
pub use state::AppState;
