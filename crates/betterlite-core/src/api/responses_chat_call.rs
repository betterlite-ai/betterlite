#![expect(
    clippy::disallowed_types,
    reason = "the completion arguments are the heterogeneous JSON dict LiteLLM builds in Python and B014 forwards to the provider, and the bridged Responses request round-trips through the same values"
)]

pub mod completions;
pub mod error;
pub mod handler;
pub mod prepare;
pub mod records;
pub mod session;
pub mod source;

mod tests;
