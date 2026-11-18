pub mod environment;
pub mod error;
pub mod handler;
pub mod request;
pub mod transform;

mod buffered;
mod stream;

mod tests;

pub const CUSTOM_LLM_PROVIDER: &str = "azure_ai";
pub const SHOULD_STRIP_BILLING_METADATA: bool = true;
