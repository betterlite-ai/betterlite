#![expect(
    clippy::disallowed_types,
    reason = "speech's optional params, kwargs and litellm_params are LiteLLM's open **kwargs, and each branch reads them into a typed wire model"
)]

mod branches;
mod bridge;
pub mod call;
pub mod error;
pub mod logging;
mod openai;
pub mod operations;
pub mod service;
mod tests;
mod values;
