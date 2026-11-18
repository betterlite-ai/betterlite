#![expect(
    clippy::disallowed_types,
    reason = "Chat results keep the provider JSON LiteLLM's TypedDicts forward, including the fields no declared type names"
)]

pub mod error;
pub mod records;
pub mod transform;

mod choices;
mod raw;
mod reasoning;
mod tools;

mod tests;
