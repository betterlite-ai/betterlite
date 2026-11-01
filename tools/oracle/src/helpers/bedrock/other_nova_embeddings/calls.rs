use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct DefaultPurposeCall {
    pub(crate) non_default: Map<String, Value>,
    pub(crate) optional_params: Map<String, Value>,
    pub(crate) input: String,
    pub(crate) inference_params: Map<String, Value>,
    pub(crate) async_invoke_route: bool,
    pub(crate) model_id: Option<String>,
    pub(crate) output_s3_uri: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct TransformRequestCall {
    pub(crate) input: String,
    pub(crate) inference_params: Map<String, Value>,
    pub(crate) async_invoke_route: bool,
    pub(crate) model_id: Option<String>,
    pub(crate) output_s3_uri: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct DataUrlCall {
    pub(crate) data_url: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct TransformResponseCall {
    pub(crate) response_list: Vec<Map<String, Value>>,
    pub(crate) batch_data: Option<Vec<Map<String, Value>>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct InvocationArnCall {
    pub(crate) response: Map<String, Value>,
}
