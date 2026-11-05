#![expect(
    clippy::disallowed_types,
    reason = "Event fields set by name and tool schemas are heterogeneous provider JSON forwarded as given"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "json! fixtures build the heterogeneous event inputs concisely"
    )
)]

pub mod call;
pub mod error;
pub mod event;
pub mod function_call;
pub mod list_tools;

mod tests;
