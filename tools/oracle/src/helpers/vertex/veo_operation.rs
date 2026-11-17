mod tests;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::vertex_veo_operation::content::content_response;
use betterlite_vertex::vertex_veo_operation::request::fetch_operation_request;
use betterlite_vertex::vertex_veo_operation::response::{
    VideoContext, create_response, edit_response, status_response,
};
use jiff::tz::TimeZone;
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
    ContentResponse(ContentCall),
    EditResponse(EditCall),
    FetchRequest(FetchCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateCall {
    model: String,
    provider: Option<String>,
    request_data: Option<Object>,
    response: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusCall {
    provider: Option<String>,
    response: Value,
    now: i64,
    time_zone: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentCall {
    response: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditCall {
    provider: Option<String>,
    request_data: Option<Object>,
    response: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FetchCall {
    video_id: String,
    api_base: String,
}

impl CreateCall {
    fn context<'call>(&'call self, settings: &'call VertexSettings) -> VideoContext<'call> {
        VideoContext {
            provider: self.provider.as_deref(),
            request_data: self.request_data.as_ref(),
            settings,
        }
    }
}

impl EditCall {
    fn context<'call>(&'call self, settings: &'call VertexSettings) -> VideoContext<'call> {
        VideoContext {
            provider: self.provider.as_deref(),
            request_data: self.request_data.as_ref(),
            settings,
        }
    }
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
        Call::ContentResponse(content) => content_answer(content),
        Call::EditResponse(edit) => edit_answer(edit, settings),
        Call::FetchRequest(fetch) => {
            let request =
                fetch_operation_request(&fetch.video_id, &fetch.api_base).or_value_error()?;
            Ok(json!({ "url": request.url, "params": request.params }))
        }
    }
}

fn create_answer(call: &CreateCall, settings: &VertexSettings) -> HelperResult {
    let body = serde_json::to_vec(&call.response).or_value_error()?;
    let video = create_response(&call.model, &body, &call.context(settings)).or_value_error()?;
    serde_json::to_value(video).or_value_error()
}

fn status_answer(call: &StatusCall) -> HelperResult {
    let body = serde_json::to_vec(&call.response).or_value_error()?;
    let zone = TimeZone::get(&call.time_zone).or_value_error()?;
    let local = LocalTimeContext { zone };
    let video =
        status_response(&body, call.provider.as_deref(), call.now, &local).or_value_error()?;
    serde_json::to_value(video).or_value_error()
}

fn content_answer(call: &ContentCall) -> HelperResult {
    let body = serde_json::to_vec(&call.response).or_value_error()?;
    let content = content_response(&body).or_value_error()?;
    Ok(json!({ "bytes": content }))
}

fn edit_answer(call: &EditCall, settings: &VertexSettings) -> HelperResult {
    let body = serde_json::to_vec(&call.response).or_value_error()?;
    let video = edit_response(&body, &call.context(settings)).or_value_error()?;
    serde_json::to_value(video).or_value_error()
}
