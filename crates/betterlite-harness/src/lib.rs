#![expect(
    clippy::disallowed_types,
    clippy::result_large_err,
    clippy::struct_excessive_bools,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::absolute_paths,
    clippy::missing_const_for_fn,
    clippy::must_use_candidate,
    clippy::partial_pub_fields,
    clippy::needless_match,
    reason = "the public harness wire contract contains required passthrough JSON and LiteLLM-shaped records"
)]

pub mod context;
pub mod errors;
pub mod handlers;
pub mod options;
pub mod sandbox;
pub mod transformation;
pub mod types;
