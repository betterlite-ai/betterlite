pub mod beta;
mod capabilities;
pub mod documents;
pub mod error;
pub mod params;
pub mod request;
pub mod response;
mod tests;
pub mod tools;
mod wire;

// LiteLLM's AmazonAnthropicClaudeConfig.custom_llm_provider
pub const CUSTOM_LLM_PROVIDER: &str = "bedrock";
// LiteLLM's AmazonAnthropicClaudeConfig.anthropic_version default
pub const ANTHROPIC_VERSION: &str = "bedrock-2023-05-31";
// The catalog model map_openai_params borrows to force tool-based structured output
pub const RESPONSE_FORMAT_STUB_MODEL: &str = "claude-3-sonnet-20240229";
// LiteLLM's AmazonAnthropicClaudeConfig.should_strip_billing_metadata
pub const STRIPS_BILLING_METADATA: bool = true;
// LiteLLM's AmazonAnthropicClaudeConfig.uses_async_transform_request
pub const USES_ASYNC_TRANSFORM_REQUEST: bool = true;
