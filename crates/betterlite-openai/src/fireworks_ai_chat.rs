pub mod auth;
pub mod chunk;
pub mod error;
pub mod hidden;
pub mod messages;
pub mod models;
pub mod reply;
pub mod request;
pub mod tools;

mod tests;

// LiteLLM's llm_provider for the Fireworks AI chat config
pub const PROVIDER: &str = "fireworks_ai";
pub const MODEL_PREFIX: &str = "fireworks_ai/";
// RESPONSE_FORMAT_TOOL_NAME, the name a JSON-mode tool call carries and the repair never converts
pub const JSON_TOOL_NAME: &str = "json_tool_call";
