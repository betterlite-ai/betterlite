pub mod environment;
pub mod error;
pub mod messages;
pub mod params;
pub mod reply;
pub mod request;

mod tests;

// LiteLLM's Bytez model root; get_complete_url ignores the configured api_base and builds on this one
pub const API_BASE: &str = "https://api.bytez.com/models/v2";

// LiteLLM stamps its own release into the Bytez user-agent, so betterlite pins the release it ports
pub const LITELLM_VERSION: &str = "1.105.1";
