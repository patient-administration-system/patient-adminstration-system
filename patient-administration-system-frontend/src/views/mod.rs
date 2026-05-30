//! View helpers.
//!
//! Loco's `format::render().view(...)` handles template lookup against the
//! `assets/views/` directory, so most of the "view layer" actually lives
//! as `.html` files alongside this module's source. This file exists for
//! the rare cases where we need a Rust-side helper (e.g. a custom Tera
//! function or a registered filter); for now it's intentionally empty.
