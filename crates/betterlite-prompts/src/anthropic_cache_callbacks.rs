pub mod error;
pub mod gate;
pub mod hook;
pub mod result;

mod tests;

// LiteLLM registers the hook as a custom logger compatible callback under this name
pub const INTEGRATION_NAME: &str = "anthropic_cache_control_hook";
