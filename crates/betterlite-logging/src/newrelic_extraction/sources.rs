use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::fields::payload::StandardLoggingPayload;
use crate::fields::timestamps::Moment;

#[derive(Clone, Copy, Debug, Default)]
pub struct CallKwargs<'event> {
    pub messages: Option<&'event Json>,
    pub model: Option<&'event str>,
    pub litellm_call_id: Option<&'event str>,
    pub custom_llm_provider: Option<&'event str>,
    // kwargs["litellm_params"]["metadata"]["headers"], where LiteLLM keeps the inbound request headers
    pub headers: Option<&'event IndexMap<String, Json>>,
    pub optional_params: Option<&'event IndexMap<String, Json>>,
    pub llm_api_duration_ms: Option<f64>,
    pub start_time: Option<Moment>,
    pub end_time: Option<Moment>,
}

#[derive(Clone, Copy, Debug)]
pub struct EventSources<'event> {
    pub kwargs: &'event CallKwargs<'event>,
    pub response: &'event IndexMap<String, Json>,
    pub standard_logging_object: Option<&'event StandardLoggingPayload>,
}
