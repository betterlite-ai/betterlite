pub mod config;
pub mod decode;
pub mod error;
pub mod handler;
pub mod result;
mod tests;
pub mod timestamps;

pub const BATCHES_BETA_HEADER: &str = "message-batches-2024-09-24";
pub const BATCHES_ROUTE: &str = "/v1/messages/batches";
pub const COMPLETION_WINDOW: &str = "24h";
pub const CREATE_NOT_IMPLEMENTED: &str = "Batch creation not yet implemented for Anthropic";
pub const CUSTOM_LLM_PROVIDER: &str = "anthropic";
pub const DEFAULT_INPUT_FILE_ID: &str = "None";
pub const ENDPOINT: &str = "/v1/messages";
pub const MISSING_API_KEY: &str = "Missing Anthropic API Key";
pub const MISSING_API_KEY_VALIDATOR: &str = "Missing Anthropic API Key - A call is being made to anthropic but no key is set either in the environment variables or via params";
pub const PARSE_PREFIX: &str = "Failed to parse Anthropic batch response: ";
