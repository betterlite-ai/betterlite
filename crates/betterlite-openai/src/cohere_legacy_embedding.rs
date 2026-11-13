use std::time::Duration;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

pub mod decode;
pub mod error;
pub mod execute;
pub mod plan;

mod tests;

use crate::openai_base_config::Headers;
pub const DEFAULT_COMPLETE_URL: &str = "https://api.cohere.ai/v1/embed";
// LiteLLM builds the blocking HTTPHandler without the caller timeout, so its own default applies
pub const SYNC_CLIENT_TIMEOUT: Duration = Duration::from_secs(600);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdkMode {
    Sync,
    Async,
}
// The blocking handler logs only the body; the async one adds the headers and the api_base
pub struct PreCall<'call> {
    pub input: &'call [String],
    pub api_key: Option<&'call SecretString>,
    pub body: &'call Object,
    pub headers: Option<&'call Headers>,
    pub api_base: Option<&'call str>,
}
// original_response is the parsed reply on success and the failure text when the call fails
pub struct PostCall<'call> {
    pub input: &'call [String],
    pub api_key: Option<&'call SecretString>,
    pub body: &'call Object,
    pub original_response: &'call Json,
}

// LiteLLM's logging_obj: pre_call before the request and post_call for the reply or the failure
pub trait EmbeddingHooks: Send {
    fn pre_call(&mut self, call: PreCall<'_>);
    fn post_call(&mut self, call: PostCall<'_>);
}
