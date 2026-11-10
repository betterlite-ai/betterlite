use betterlite_models::passthrough_policies::query_merge::QueryParameters;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    pub(crate) existing_url: Option<String>,
    pub(crate) request_query_params: Option<QueryParameters>,
    pub(crate) default_query_params: Option<QueryParameters>,
    pub(crate) request_headers: Option<IndexMap<String, String>>,
    pub(crate) custom_headers: Option<IndexMap<String, String>>,
    pub(crate) forward_headers: Option<bool>,
    pub(crate) endpoint_timeout: Option<f64>,
    pub(crate) settings_timeout: Option<f64>,
    pub(crate) request: Option<RequestFields>,
    pub(crate) deployment: Option<DeploymentFields>,
    pub(crate) router: Option<RouterFields>,
    pub(crate) configured_timeout: Option<f64>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    QueryMerge,
    ForwardHeaders,
    ProxyTimeout,
    SdkTimeout,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestFields {
    #[serde(default)]
    pub(crate) stream: bool,
    pub(crate) stream_timeout: Option<ValueInput>,
    pub(crate) timeout: Option<ValueInput>,
    pub(crate) request_timeout: Option<ValueInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentFields {
    pub(crate) stream_timeout: Option<ValueInput>,
    pub(crate) timeout: Option<ValueInput>,
    pub(crate) request_timeout: Option<ValueInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouterFields {
    pub(crate) stream_timeout: Option<ValueInput>,
    pub(crate) timeout: Option<ValueInput>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ValueInput {
    Number(f64),
    Text(String),
}
