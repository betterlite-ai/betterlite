use betterlite_models::controls::call_params::CallParams;
use betterlite_models::controls::optional_params::OptionalParams;
use betterlite_models::decisions::Json;
use betterlite_models::headers::affinity::AffinityParams;
use betterlite_openai::response_input::{ResponsesInput, ResponsesTool};
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::api::responses_compact_http::service::SharedSession;
use crate::api::responses_create_http::config::Headers;
use crate::api::responses_provider_selection::ParameterMap;

// The arguments of responses(): the named parameters, then the caller's kwargs
#[derive(Clone, Debug)]
pub struct ResponsesCreateCall {
    pub model: String,
    pub input: ResponsesInput,
    pub include: Option<Json>,
    pub instructions: Option<String>,
    pub max_output_tokens: Option<i64>,
    pub prompt: Option<Json>,
    pub metadata: Option<Json>,
    pub parallel_tool_calls: Option<bool>,
    pub previous_response_id: Option<String>,
    pub reasoning: Option<Json>,
    pub store: Option<bool>,
    pub background: Option<bool>,
    pub stream: Option<bool>,
    pub temperature: Option<f64>,
    pub text: Option<Json>,
    pub text_format: Option<Json>,
    pub tool_choice: Option<Json>,
    pub tools: Option<Vec<ResponsesTool>>,
    pub top_p: Option<f64>,
    pub truncation: Option<String>,
    pub user: Option<String>,
    pub service_tier: Option<String>,
    pub safety_identifier: Option<String>,
    pub extra_headers: Option<Headers>,
    pub extra_query: Option<Json>,
    pub extra_body: Option<Object>,
    pub timeout: Option<f64>,
    pub allowed_openai_params: Option<Vec<String>>,
    pub custom_llm_provider: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub ssl_verify: Option<bool>,
    pub shared_session: Option<SharedSession>,
    pub kwargs: ParameterMap,
}

// get_llm_provider receives the caller's key and base through litellm_params
#[must_use]
pub fn resolution_params(call: &ResponsesCreateCall) -> CallParams {
    CallParams {
        model: None,
        api_key: call.api_key.clone(),
        api_base: call.api_base.clone(),
        custom_llm_provider: None,
        metadata: None,
        litellm_metadata: None,
        litellm_session_id: None,
        litellm_trace_id: None,
        no_log: None,
        drop_params: None,
        control: None,
        credentials: None,
        optional: OptionalParams::default(),
        affinity: AffinityParams::default(),
    }
}
