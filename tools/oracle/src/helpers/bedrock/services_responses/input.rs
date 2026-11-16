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
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    BedrockResponsesModelRoute(Vec<RouteCase>),
    ResponsesBearerAuth(Vec<AuthCase>),
    ResponsesSigv4(Vec<SigningCase>),
    ResponseOptionGates(Vec<OptionCase>),
    NativeFileSearchFlag(Vec<OptionCase>),
    InlineImageTraversal(Vec<TraversalCase>),
    RemoteUrlInlined(Vec<FetchCase>),
    ResponsesRuntimeUrl(Vec<UrlCase>),
    InlineImageReplacement(Vec<ReplacementCase>),
    CodexInputRewrite(Vec<CodexCase>),
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
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorProbe {
    pub(super) status: u16,
    pub(super) message: String,
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SigningCase {
    pub(super) headers: BTreeMap<String, String>,
    pub(super) optional_params: BTreeMap<String, Box<RawValue>>,
    pub(super) request_data: IndexMap<String, Json>,
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
