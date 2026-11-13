#![expect(
    clippy::disallowed_types,
    reason = "the pre-call step rewrites LiteLLM's free-form request dict, whose keys no typed model covers"
)]

pub mod failure;
pub mod key;
pub mod ports;
pub mod rewrites;
pub mod route_type;
mod tests;
