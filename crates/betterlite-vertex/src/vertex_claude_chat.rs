pub mod beta;
pub mod error;
pub mod json;
pub mod output;
pub mod params;
pub mod provider;
pub mod request;
pub mod response;

mod tests;

// LiteLLM's VertexAIAnthropicConfig reports this identity whatever the caller passed.
pub const PROVIDER: &str = "vertex_ai";
