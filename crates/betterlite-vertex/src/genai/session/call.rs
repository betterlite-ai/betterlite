use betterlite_core::api::genai::generate_content::GenerateContentRequest;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Deserialize;
use std::time::{Duration, TryFromFloatSecsError};

// One generate content call, field for field with litellm/google_genai/main.py's arguments and its **kwargs
#[derive(Debug, Deserialize)]
pub struct GenaiCall {
    pub model: String,
    pub contents: Json,
    pub config: Option<Object>,
    pub tools: Option<Json>,
    pub extra_headers: Option<IndexMap<String, String>>,
    pub extra_query: Option<Object>,
    pub extra_body: Option<Object>,
    #[serde(default)]
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub kwargs: Object,
}

impl GenaiCall {
    // timeout or request_timeout hands the handler seconds; a zero is no timeout, and B3489 resolves it then
    pub fn timeout(&self) -> Result<Option<Duration>, TryFromFloatSecsError> {
        self.timeout.map_or(Ok(None), |seconds| {
            Duration::try_from_secs_f64(seconds).map(Some)
        })
    }

    // the request the four entry points read, field for field
    #[must_use]
    pub fn request(&self, timeout: Option<Duration>) -> GenerateContentRequest<'_> {
        GenerateContentRequest {
            model: &self.model,
            contents: &self.contents,
            config: self.config.as_ref(),
            tools: self.tools.as_ref(),
            extra_headers: self.extra_headers.as_ref(),
            extra_query: self.extra_query.as_ref(),
            extra_body: self.extra_body.as_ref(),
            timeout,
            custom_llm_provider: self.custom_llm_provider.as_deref(),
            kwargs: &self.kwargs,
        }
    }
}
