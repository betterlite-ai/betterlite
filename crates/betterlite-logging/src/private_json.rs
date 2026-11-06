#![expect(
    clippy::disallowed_types,
    reason = "private json is the serialization boundary: the named Passthrough field is the one place untyped integration JSON enters, and the module strips NULs and bounds it before any writer sees it"
)]

pub mod error;
pub mod files;
pub mod safe_json;

mod tests;
