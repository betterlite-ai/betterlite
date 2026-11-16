#![expect(
    clippy::disallowed_types,
    reason = "stored cache settings are a free-form provider kwargs dict with no fixed schema"
)]

pub mod fields;
pub mod overlay;
pub mod route;
pub mod stored;
pub mod view;

#[path = "cache_settings_read/tests/support.rs"]
mod test_support;
