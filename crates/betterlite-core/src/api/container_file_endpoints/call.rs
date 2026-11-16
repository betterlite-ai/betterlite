use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::containers::container_file_http::ContainerFileUpload;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::SecretString;

// the generated functions default custom_llm_provider to "openai" and timeout to 600
pub const DEFAULT_PROVIDER: &str = "openai";

pub const DEFAULT_TIMEOUT_SECONDS: u64 = 600;

// every keyword one generated container file function takes, typed the way the caller passes it
#[derive(Clone, Debug)]
pub struct ContainerFileCall {
    pub container_id: Option<String>,
    pub file_id: Option<String>,
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub order: Option<String>,
    pub file: Option<ContainerFileUpload>,
    // an explicit zero is falsy in LiteLLM's `timeout or DEFAULT_REQUEST_TIMEOUT`, so it falls back too
    pub timeout: Option<Duration>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub custom_llm_provider: String,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub extra_query: Option<IndexMap<String, ParamValue>>,
    // the generated functions take extra_body as a named argument and never pass it on
    pub extra_body: Option<Object>,
    pub mock_response: Option<Json>,
    pub litellm_call_id: Option<String>,
    // every other keyword, which reads only as litellm_params extras and logging metadata
    pub extra: IndexMap<String, Json>,
}

// one call's arguments and the provider the error mapping names, updated once the id is decoded
pub(super) struct CallSite<'call> {
    pub(super) endpoint_name: &'call str,
    pub(super) call: &'call ContainerFileCall,
    pub(super) logging: &'call Mutex<LoggingState>,
    pub(super) default_timeout_seconds: f64,
    pub(super) provider: String,
}

impl Default for ContainerFileCall {
    fn default() -> Self {
        Self {
            container_id: None,
            file_id: None,
            after: None,
            limit: None,
            order: None,
            file: None,
            timeout: Some(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS)),
            api_key: None,
            api_base: None,
            api_version: None,
            custom_llm_provider: DEFAULT_PROVIDER.to_owned(),
            extra_headers: None,
            extra_query: None,
            extra_body: None,
            mock_response: None,
            litellm_call_id: None,
            extra: IndexMap::new(),
        }
    }
}
