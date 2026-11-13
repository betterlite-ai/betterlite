#![expect(
    clippy::disallowed_types,
    reason = "private json models JSON-native values: the loader bounds untrusted JSON, the named Passthrough field carries untyped data through TryFrom, and the writers take Serialize records"
)]

pub mod error;
pub mod files;
pub mod safe_json;

mod tests;
