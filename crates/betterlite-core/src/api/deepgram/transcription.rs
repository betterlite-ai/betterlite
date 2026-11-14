#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "LiteLLM forwards free-form Deepgram query options, echoes the raw reply text and reads corpus file names"
)]

pub mod error;
pub mod request;
pub mod response;

mod tests;
