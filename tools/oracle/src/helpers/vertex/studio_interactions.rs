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
    Delete,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseCall {
    action: Reply,
    status: u16,
    #[serde(default)]
    headers: IndexMap<String, String>,
    body: String,
    interaction_id: Option<String>,
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
        Call::TransformRequest(given) => request_call(given),
        Call::Lifecycle(given) => lifecycle_call(given),
        Call::Response(given) => response_call(given),
    }
}

fn request_call(call: &RequestCall) -> crate::helpers::HelperResult {
    let text = call.input.clone().map(Json::Text);
    let (model, agent) = (call.model.as_deref(), call.agent.as_deref());
    transform_request(model, agent, text.as_ref(), &call.optional_params)
        .map(|body| json!({ "body": body }))
        .or_value_error()
}

fn fixed_inputs() -> (Settings, StudioParams) {
    let settings = Settings {
        environment_key: None,
        generic_api_key: None,
        api_base: DEFAULT_API_BASE.to_owned(),
    };
    let params = StudioParams {
        api_key: Some(SecretString::from("key")),
    };
    (settings, params)
}

fn lifecycle_call(call: &LifecycleCall) -> crate::helpers::HelperResult {
    let (settings, params) = fixed_inputs();
    let base = Some(call.api_base.as_str());
    lifecycle_request(call.action, &call.interaction_id, base, &params, &settings)
        .map(|url| json!({ "url": url }))
        .or_value_error()
}

fn response_call(call: &ResponseCall) -> crate::helpers::HelperResult {
    let response = RawResponse {
        status: call.status,
        headers: &call.headers,
        body: call.body.as_bytes(),
    };
    let interaction = match call.action {
        Reply::Delete => {
            let id = call
                .interaction_id
                .as_ref()
                .ok_or("delete needs an interaction_id");
            let result = transform_delete_response(&response, id.or_value_error()?);
            return Ok(json!(result.or_value_error()?));
        }
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
