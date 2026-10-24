#![expect(
    clippy::disallowed_types,
    reason = "LiteLLM rewrites $ref inside any dict of a caller's free-form JSON schema and reads free-form option, metadata and error-body dicts"
)]

pub mod base_helpers;
pub mod error;
