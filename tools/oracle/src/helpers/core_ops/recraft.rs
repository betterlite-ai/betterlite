use std::collections::BTreeMap;

use betterlite_models::tiered_pricing::Scalar;
use betterlite_openai::api::image_contracts::records::{ImageData, ImageHeaders, ImageParams};
use serde::Deserialize;
use serde_json::Value;

mod answers;

use crate::helpers::{Helper, HelperError, HelperResult};

use answers::{
    cost_answer, edit_map_answer, edit_request_answer, environment_answer, map_answer,
    request_answer, response_answer, supported_answer, url_answer,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Endpoint {
    Generation,
    Edit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageSeed {
    #[serde(default)]
    filename: Option<String>,
    data_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Supported { endpoint: Endpoint, model: String },
    Map(MapCall),
    EditMap(EditMapCall),
    Url(UrlCall),
    Environment(EnvironmentCall),
    Request(RequestCall),
    EditRequest(EditRequestCall),
    Response(ReplyCall),
    EditResponse(ReplyCall),
    Cost(CostCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapCall {
    model: String,
    #[serde(default)]
    non_default_params: ImageParams,
    #[serde(default)]
    optional_params: ImageParams,
    #[serde(default)]
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditMapCall {
    model: String,
    #[serde(default)]
    params: ImageParams,
    #[serde(default)]
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    endpoint: Endpoint,
    model: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentCall {
    endpoint: Endpoint,
    model: String,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    headers: ImageHeaders,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    model: String,
    prompt: String,
    #[serde(default)]
    optional_params: ImageParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditRequestCall {
    model: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    images: Vec<ImageSeed>,
    #[serde(default)]
    optional_params: ImageParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    status: u16,
    #[serde(default)]
    headers: ImageHeaders,
    body: String,
    #[serde(default)]
    created: i64,
    #[serde(default)]
    data: Vec<ImageData>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CostCall {
    model: String,
    #[serde(default)]
    output_cost_per_image: Option<Scalar>,
    #[serde(default)]
    image_count: i64,
    #[serde(default)]
    other: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let values = input
            .calls
            .iter()
            .map(run_call)
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
        Ok(Value::Array(values))
    }
}

fn run_call(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Supported { endpoint, model } => Ok(supported_answer(*endpoint, model)),
        Call::Map(map) => Ok(map_answer(map)),
        Call::EditMap(edit_map) => Ok(edit_map_answer(edit_map)),
        Call::Url(url) => Ok(url_answer(url)),
        Call::Environment(environment) => Ok(environment_answer(environment)),
        Call::Request(request) => request_answer(request),
        Call::EditRequest(edit_request) => edit_request_answer(edit_request),
        Call::Response(reply) => response_answer(Endpoint::Generation, reply),
        Call::EditResponse(reply) => response_answer(Endpoint::Edit, reply),
        Call::Cost(cost) => cost_answer(cost),
    }
}
