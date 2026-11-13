#![expect(
    clippy::disallowed_types,
    reason = "the form dictionary is the JSON object LiteLLM's endpoint returns, so its field values are serde_json values"
)]

pub mod core_tables;
pub mod error;
pub mod project;
pub mod spec;
mod tests;
