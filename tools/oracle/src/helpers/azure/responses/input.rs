use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_params::params::ResponsesParams;
use betterlite_openai::response_resources::request::InputItemsOrder;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    BaseAndApiVersion(UrlCall),
    StripModelPrefix(Steps<TransformStep>),
    OSeriesOptions(CapabilityCall),
    ReasoningNullFields(Steps<TransformStep>),
    WebsocketModelQuery(Steps<WebsocketStep>),
    ResourceIdBeforeQuery(Steps<ResourceStep>),
    CancelResponse(Steps<ReplyStep>),
    AzureResponsesCapabilities(CapabilityCall),
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Steps<T> {
    #[serde(default)]
    pub(crate) steps: Vec<T>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCall {
    pub(crate) default_api_version: String,
    #[serde(default)]
    pub(crate) steps: Vec<UrlStep>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlStep {
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) api_key: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformStep {
    pub(crate) model: String,
    pub(crate) input: ResponsesInput,
    #[serde(default)]
    pub(crate) params: Object,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WebsocketStep {
    pub(crate) api_base: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResourceStep {
    pub(crate) endpoint: Endpoint,
    pub(crate) api_base: String,
    pub(crate) response_id: String,
    #[serde(default)]
    pub(crate) after: Option<String>,
    #[serde(default)]
    pub(crate) before: Option<String>,
    #[serde(default)]
    pub(crate) include: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) limit: Option<i64>,
    #[serde(default)]
    pub(crate) order: Option<InputItemsOrder>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Endpoint {
    Get,
    Delete,
    Cancel,
    List,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyStep {
    pub(crate) status: u16,
    pub(crate) body: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapabilityCall {
    pub(crate) install: Install,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) map_steps: Vec<MapStep>,
    #[serde(default)]
    pub(crate) o_series_steps: Vec<MapStep>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Install {
    pub(crate) model_cost: BTreeMap<String, Value>,
    #[serde(default)]
    pub(crate) chat_models: Vec<String>,
    #[serde(default)]
    pub(crate) text_models: Vec<String>,
    #[serde(default)]
    pub(crate) drop_params: bool,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapStep {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) params: ResponsesParams,
    #[serde(default)]
    pub(crate) drop_params: bool,
    #[serde(default)]
    pub(crate) global_drop_params: bool,
}
