use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::responses_compact_http::service::SharedSession;

// The arguments of compact_responses: the named parameters plus the caller's remaining kwargs;
// extra_query is accepted and never reaches the handler
#[derive(Debug)]
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
