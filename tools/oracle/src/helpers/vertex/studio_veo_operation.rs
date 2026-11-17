mod tests;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::studio_veo_operation::download::download_target;
use betterlite_vertex::studio_veo_operation::records::{
    VideoContext, create_response, status_request, status_response,
};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    default_duration_seconds: i64,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    CreateResponse(CreateCall),
    StatusResponse(StatusCall),
    StatusRequest(StatusRequestCall),
    ContentTarget(ContentTargetCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateCall {
    model: String,
    provider: Option<String>,
    request_data: Option<Object>,
    body: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusCall {
    provider: Option<String>,
    body: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusRequestCall {
    video_id: String,
    api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentTargetCall {
    body: Value,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = VertexSettings {
            project: None,
            location: None,
            credentials: None,
            adc: AdcEnvironment::default(),
            default_video_duration_seconds: input.default_duration_seconds,
        };
        let calls = input
            .calls
            .iter()
            .map(|call| resolve(call, &settings))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": calls }))
    }
}

fn resolve(call: &Call, settings: &VertexSettings) -> HelperResult {
    match call {
        Call::CreateResponse(create) => create_answer(create, settings),
        Call::StatusResponse(status) => status_answer(status),
        Call::StatusRequest(request) => {
            let polled = status_request(&request.video_id, &request.api_base);
            Ok(json!({ "url": polled.url, "params": polled.params }))
        }
        Call::ContentTarget(target) => content_target_answer(target),
    }
}

fn create_answer(call: &CreateCall, settings: &VertexSettings) -> HelperResult {
    let body = serde_json::to_vec(&call.body).or_value_error()?;
    let context = VideoContext {
        provider: call.provider.as_deref(),
        request_data: call.request_data.as_ref(),
        settings,
    };
    let video = create_response(&call.model, &body, &context).or_value_error()?;
    serde_json::to_value(video).or_value_error()
}

fn status_answer(call: &StatusCall) -> HelperResult {
    let body = serde_json::to_vec(&call.body).or_value_error()?;
    let video = status_response(&body, call.provider.as_deref()).or_value_error()?;
    serde_json::to_value(video).or_value_error()
}

fn content_target_answer(call: &ContentTargetCall) -> HelperResult {
    let body = serde_json::to_vec(&call.body).or_value_error()?;
    let target = download_target(&body).or_value_error()?;
    Ok(json!({ "target": target }))
}
