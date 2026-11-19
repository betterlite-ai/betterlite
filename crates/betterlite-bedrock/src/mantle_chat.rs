pub mod environment;
pub mod error;
pub mod messages;
pub mod request;
pub mod response;
pub mod stream;
pub mod url;

mod tests;

// AmazonMantleConfig strips this routing prefix once, wherever it appears, to recover the model ID.
pub const MANTLE_PREFIX: &str = "mantle/";
// AmazonMantleMessagesConfig.beta_headers_provider
pub const BETA_HEADERS_PROVIDER: &str = "bedrock_mantle";
// AmazonMantleMessagesConfig.should_filter_anthropic_beta_headers: the caller's list travels as sent.
pub const SHOULD_FILTER_ANTHROPIC_BETA_HEADERS: bool = false;
// AmazonMantleConfig.has_custom_stream_wrapper: the native Anthropic SSE reader owns the body.
pub const HAS_CUSTOM_STREAM_WRAPPER: bool = false;
// DEFAULT_ANTHROPIC_API_VERSION in anthropic/pass_through/messages/transformation.py
pub const DEFAULT_ANTHROPIC_API_VERSION: &str = "2023-06-01";
// The header the Messages surface reads the API version from; the Mantle body carries none.
pub const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";
// The exact key a configured project ID overwrites.
pub const ANTHROPIC_WORKSPACE_HEADER: &str = "anthropic-workspace";
