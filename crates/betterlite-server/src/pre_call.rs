#![expect(
    clippy::disallowed_types,
    reason = "the pre-call step rewrites LiteLLM's free-form request dict, whose keys no typed model covers"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

pub mod failure;
pub mod key;
pub mod ports;
pub mod rewrites;
pub mod route_type;
mod tests;
