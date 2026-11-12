#![expect(
    clippy::disallowed_types,
    reason = "Chat messages and Responses input items are heterogeneous provider JSON without a closed schema"
)]

pub mod error;
pub mod history;
pub mod metadata;
pub mod reasoning;
pub mod tool_fields;

mod tests;
