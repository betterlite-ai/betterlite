pub mod call;
pub mod error;
pub mod handler;
pub mod options;
pub mod providers;

mod logging;
#[cfg(test)]
#[path = "video_generation_call/testing/tests.rs"]
pub(crate) mod testing;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_remix_call::config::DEFAULT_TIMEOUT_SECONDS;
use crate::api::videos_http_transport::video_config::{Headers, Params};

// litellm.constants.DEFAULT_VIDEO_ENDPOINT_MODEL, the model get_llm_provider is asked to resolve when the caller names none
pub const DEFAULT_MODEL: &str = "sora-2";
// CallTypes.create_video, the call type video_generation leaves on the logging object
pub const CALL_TYPE: &str = "create_video";
// the kwargs key avideo_generation sets to true and video_generation pops
pub const ASYNC_CALL: &str = "async_call";
// the logging object's name, which GenericLiteLLMParams never carries as a keyword
pub const LOGGING_OBJECT: &str = "litellm_logging_obj";
// the keyword the logging update reads as the call id
pub const LITELLM_CALL_ID: &str = "litellm_call_id";

// video_generation's declared arguments, plus the loose keywords its **kwargs takes
pub struct VideoGenerationRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub input_reference: Option<SuppliedMedia>,
    pub seconds: Option<String>,
    pub size: Option<String>,
    pub user: Option<String>,
    // the signature's 600 default; a caller that wants LiteLLM's explicit None sets this to None
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<String>,
    pub extra_headers: Option<Headers>,
    // accepted to match LiteLLM's signature: it becomes a requested option but never reaches the handler
    pub extra_query: Option<Params>,
    pub extra_body: Option<Params>,
    pub kwargs: IndexMap<String, Json>,
}

impl VideoGenerationRequest {
    // a call with every optional argument unset and the timeout at the signature's 600 seconds
    #[must_use]
    pub fn new(prompt: String) -> Self {
        Self {
            prompt,
            model: None,
            input_reference: None,
            seconds: None,
            size: None,
            user: None,
            timeout: Some(DEFAULT_TIMEOUT_SECONDS),
            custom_llm_provider: None,
            extra_headers: None,
            extra_query: None,
            extra_body: None,
            kwargs: IndexMap::new(),
        }
    }
}
