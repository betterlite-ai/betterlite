#![expect(
    clippy::disallowed_types,
    reason = "stored cache_settings is a free-form provider kwargs map with no fixed schema"
)]

pub mod route;
pub mod saved;

#[path = "cache_settings_test/tests/support.rs"]
mod support;
