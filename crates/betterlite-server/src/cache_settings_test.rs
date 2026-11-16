#![expect(
    clippy::disallowed_types,
    reason = "stored cache_settings is a free-form provider kwargs map with no fixed schema"
)]

pub mod route;
pub mod saved;

#[cfg(test)]
mod support;
