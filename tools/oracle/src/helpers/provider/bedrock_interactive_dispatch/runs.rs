use betterlite_bedrock::model_routes::ParamMap;
use serde::Deserialize;
use serde_json::Value;

pub(crate) mod passthrough_run;
pub(crate) mod realtime_run;
pub(crate) mod responses_run;
pub(crate) mod search_run;
pub(crate) mod shared;
pub(crate) mod vector_run;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponsesRun {
    model: String,
    #[serde(default)]
    model_cost: ParamMap,
    input: Value,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    api_key: Option<String>,
    status: u16,
    reply: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PassthroughRun {
    endpoint: String,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    region: Option<String>,
    status: u16,
    #[serde(default)]
    reply: String,
    #[serde(default)]
    chunks: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchRun {
    query: String,
    #[serde(default)]
    optional: Option<Value>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    status: u16,
    reply: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VectorRun {
    query: String,
    #[serde(default)]
    optional: Option<Value>,
    #[serde(default)]
    extra_body: Option<Value>,
    status: u16,
    reply: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RealtimeRun {
    pending: String,
}
