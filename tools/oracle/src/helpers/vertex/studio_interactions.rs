use crate::helpers::OrHelperError as _;
use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_models::call_helpers::headers::{HeaderSource, process_response_headers};
use betterlite_models::decisions::Json;
use betterlite_vertex::studio_interactions::credentials::StudioParams;
use betterlite_vertex::studio_interactions::decode::{
    RawResponse, transform_delete_response, transform_get_response, transform_response,
};
use betterlite_vertex::studio_interactions::request::{InteractionOptions, transform_request};
use betterlite_vertex::studio_interactions::urls::{Lifecycle, lifecycle_request};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    TransformRequest(Box<RequestCall>),
    Lifecycle(LifecycleCall),
    Response(ResponseCall),
    Delete(DeleteCall),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    model: Option<String>,
    agent: Option<String>,
    input: Option<String>,
    optional_params: InteractionOptions,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleCall {
    action: Lifecycle,
    interaction_id: String,
    api_base: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reply {
    Create,
    Get,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseCall {
    action: Reply,
    status: u16,
    #[serde(default)]
    headers: IndexMap<String, String>,
    body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteCall {
    status: u16,
    #[serde(default)]
    headers: IndexMap<String, String>,
    body: String,
    interaction_id: String,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let calls = input
            .calls
            .iter()
            .map(resolve)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": calls }))
    }
}

fn resolve(call: &Call) -> crate::helpers::HelperResult {
    match call {
        Call::TransformRequest(request) => {
            let text = request.input.as_ref().map(|text| Json::Text(text.clone()));
            let body = transform_request(
                request.model.as_deref(),
                request.agent.as_deref(),
                text.as_ref(),
                &request.optional_params,
            )
            .or_value_error()?;
            Ok(json!({ "body": body }))
        }
        Call::Lifecycle(given) => {
            let settings = Settings {
                environment_key: None,
                generic_api_key: None,
                api_base: DEFAULT_API_BASE.to_owned(),
            };
            let params = StudioParams {
                api_key: Some(SecretString::from("key")),
            };
            let url = lifecycle_request(
                given.action,
                &given.interaction_id,
                Some(&given.api_base),
                &params,
                &settings,
            )
            .or_value_error()?;
            Ok(json!({ "url": url }))
        }
        Call::Response(given) => response_call(given),
        Call::Delete(given) => delete_call(given),
    }
}

fn delete_call(call: &DeleteCall) -> crate::helpers::HelperResult {
    let response = RawResponse {
        status: call.status,
        headers: &call.headers,
        body: call.body.as_bytes(),
    };
    let result = transform_delete_response(&response, &call.interaction_id).or_value_error()?;
    Ok(json!(result))
}

fn response_call(call: &ResponseCall) -> crate::helpers::HelperResult {
    let response = RawResponse {
        status: call.status,
        headers: &call.headers,
        body: call.body.as_bytes(),
    };
    let interaction = match call.action {
        Reply::Create => transform_response(&response, |raw| {
            process_response_headers(HeaderSource::Upstream(raw))
        }),
        Reply::Get => transform_get_response(&response),
    }
    .or_value_error()?;
    Ok(json!({
        "record": interaction.record,
        "headers": interaction.hidden_params.headers,
        "additional_headers": interaction.hidden_params.additional_headers,
    }))
}

mod tests;
