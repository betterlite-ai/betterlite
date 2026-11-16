use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::ParamMap;
use betterlite_models::decisions::Json;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_params::params::ResponsesParams;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
pub(crate) enum Input {
    #[serde(rename = "bedrock_responses_model_route")]
    ModelRoute(Vec<RouteCase>),
    #[serde(rename = "responses_bearer_auth")]
    BearerAuth(Vec<AuthCase>),
    #[serde(rename = "responses_sigv4")]
    Sigv4(Vec<SigningCase>),
    #[serde(rename = "response_option_gates")]
    OptionGates(Vec<OptionCase>),
    #[serde(rename = "native_file_search_flag")]
    NativeFileSearch(Vec<OptionCase>),
    #[serde(rename = "inline_image_traversal")]
    Traversal(Vec<TraversalCase>),
    #[serde(rename = "remote_url_inlined")]
    RemoteUrlInlined(Vec<FetchCase>),
    #[serde(rename = "responses_runtime_url")]
    RuntimeUrl(Vec<UrlCase>),
    #[serde(rename = "inline_image_replacement")]
    Replacement(Vec<ReplacementCase>),
    #[serde(rename = "codex_input_rewrite")]
    CodexRewrite(Vec<CodexCase>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct RouteCase {
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) model_cost: ParamMap,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthCase {
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) headers: IndexMap<String, String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) sign: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorProbe {
    pub(super) status: u16,
    pub(super) message: String,
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SigningCase {
    pub(super) access_key_id: String,
    pub(super) secret_access_key: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) optional_params: BTreeMap<String, Box<RawValue>>,
    pub(super) request_data: IndexMap<String, Json>,
    pub(super) now: u64,
    pub(super) error: Option<ErrorProbe>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OptionCase {
    pub(super) params: ResponsesParams,
    #[serde(default)]
    pub(super) drop_params: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TraversalCase {
    pub(super) input: ResponsesInput,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReplacementCase {
    pub(super) input: ResponsesInput,
    pub(super) inlined: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FetchCase {
    pub(super) input: ResponsesInput,
    pub(super) data_urls: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct UrlParams {
    pub(super) aws_region_name: Option<String>,
    pub(super) aws_bedrock_runtime_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UrlCase {
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) litellm_params: UrlParams,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexCase {
    pub(super) model: String,
    pub(super) input: ResponsesInput,
    #[serde(default)]
    pub(super) params: Object,
}
