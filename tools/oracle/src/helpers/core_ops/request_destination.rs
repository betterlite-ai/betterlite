use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_destination::destination::{
    DestinationField, reject_url_valued_destination, reject_url_valued_destinations,
};
use betterlite_core::api::request_destination::error::RequestDestinationError;
use betterlite_core::api::request_destination::model::{
    InferenceModelKind, ModelDefaults, resolve_inference_model,
};
use betterlite_core::api::request_destination::parameters::{
    populate_request_parameters, tags_from_request_body,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    ResolveInferenceModel(ResolveCall),
    RejectUrlValuedDestination(SingleRejectCall),
    RejectUrlValuedDestinations(DataRejectCall),
    TagsFromRequestBody(TagsCall),
    PopulateRequestParameters(PopulateCall),
}

#[derive(Debug, Deserialize)]
struct ResolveCall {
    body_model: Option<String>,
    #[serde(default)]
    settings: ModelDefaults,
    cli_model: Option<String>,
    endpoint_model: Option<String>,
    kind: InferenceModelKind,
}

#[derive(Debug, Deserialize)]
struct SingleRejectCall {
    field: DestinationField,
    value: String,
    #[serde(default)]
    allowed_hosts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DataRejectCall {
    data: RequestData,
    #[serde(default)]
    allowed_hosts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TagsCall {
    request_body: RequestData,
}

#[derive(Debug, Deserialize)]
struct PopulateCall {
    request_data: RequestData,
    #[serde(default)]
    path_params: RequestData,
    #[serde(default)]
    query_params: IndexMap<String, String>,
    route_path: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(run_call(call)?);
        }
        Ok(Value::Array(results))
    }
}

fn run_call(entry: &Call) -> HelperResult {
    match entry {
        Call::ResolveInferenceModel(call) => Ok(resolve_value(call)),
        Call::RejectUrlValuedDestination(call) => Ok(single_rejection(call)),
        Call::RejectUrlValuedDestinations(call) => Ok(data_rejection(call)),
        Call::TagsFromRequestBody(call) => Ok(serde_json::to_value(tags_from_request_body(
            &call.request_body,
        ))?),
        Call::PopulateRequestParameters(call) => populate_result(call),
    }
}

fn resolve_value(call: &ResolveCall) -> Value {
    resolve_inference_model(
        call.body_model.as_deref(),
        &call.settings,
        call.cli_model.as_deref(),
        call.endpoint_model.as_deref(),
        call.kind,
    )
    .map_or(Value::Null, Value::String)
}

fn single_rejection(call: &SingleRejectCall) -> Value {
    rejection(reject_url_valued_destination(
        call.field,
        &call.value,
        &parsed_hosts(&call.allowed_hosts),
    ))
}

fn data_rejection(call: &DataRejectCall) -> Value {
    rejection(reject_url_valued_destinations(
        &call.data,
        &parsed_hosts(&call.allowed_hosts),
    ))
}

fn populate_result(call: &PopulateCall) -> HelperResult {
    Ok(serde_json::to_value(populate_request_parameters(
        &call.request_data,
        &call.path_params,
        &call.query_params,
        &call.route_path,
    ))?)
}

fn parsed_hosts(entries: &[String]) -> Vec<AllowedDestination> {
    let mut allowed = Vec::new();
    for entry in entries {
        allowed.extend(AllowedDestination::parse(entry));
    }
    allowed
}

fn rejection(result: Result<(), RequestDestinationError>) -> Value {
    match result {
        Ok(()) => Value::Null,
        Err(error) => {
            let facts = error.facts();
            json!({
                "status": facts.status,
                "type": facts.error_type,
                "param": facts.param,
                "code": facts.code,
                "detail": facts.detail,
            })
        }
    }
}
