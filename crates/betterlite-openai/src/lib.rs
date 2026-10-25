#![expect(
    clippy::disallowed_types,
    reason = "LiteLLM rewrites $ref inside any dict of a caller's free-form JSON schema and reads free-form option, metadata and error-body dicts"
)]

pub mod base_helpers;
pub mod error;
pub mod search;

#[cfg(test)]
#[path = "search/search_credential.rs"]
mod search_credential;
#[cfg(test)]
#[path = "search/search_hooks.rs"]
mod search_hooks;
#[cfg(test)]
#[path = "search/search_types.rs"]
mod search_types;
