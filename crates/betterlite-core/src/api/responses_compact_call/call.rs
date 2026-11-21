use std::collections::BTreeMap;

use betterlite_models::controls::call_params::CallParams;
use betterlite_models::controls::optional_params::OptionalParams;
use betterlite_models::decisions::Json;
use betterlite_models::headers::affinity::AffinityParams;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::responses_compact_http::service::SharedSession;

// The arguments of compact_responses: the named parameters, then kwargs; extra_query is unused
#[derive(Clone, Debug)]
pub struct CompactResponsesCall {
    pub model: String,
    pub input: ResponsesInput,
    pub instructions: Option<String>,
    pub previous_response_id: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub ssl_verify: Option<bool>,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub extra_query: Option<Object>,
    pub extra_body: Option<Object>,
    pub timeout: Option<f64>,
    pub shared_session: Option<SharedSession>,
    pub kwargs: IndexMap<String, Json>,
    pub acompact_responses: Option<Json>,
}

// The resolution seeds the caller's key and base; every other field keeps its default
pub(super) fn resolution_params(call: &CompactResponsesCall) -> CallParams {
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
