pub mod environment;
pub mod environment_error;
pub mod transform;
pub mod url;
pub mod url_error;

mod tests;

pub const CUSTOM_LLM_PROVIDER: &str = "azure_ai";
pub const SHOULD_STRIP_BILLING_METADATA: bool = true;
