mod tests;

use betterlite_config::vertex::video_settings;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::vertex_veo_operation::content::content_response;
use betterlite_vertex::vertex_veo_operation::request::fetch_operation_request;
use betterlite_vertex::vertex_veo_operation::response::{
    create_response, edit_response, status_response,
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
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    CreateResponse {
        model: String,
        provider: Option<String>,
        request_data: Option<Object>,
        response: Value,
    },
    StatusResponse {
        provider: Option<String>,
        response: Value,
        now: i64,
        time_zone: String,
    },
    ContentResponse {
        response: Value,
    },
    EditResponse {
        provider: Option<String>,
        request_data: Option<Object>,
        response: Value,
    },
    FetchRequest {
        video_id: String,
        api_base: String,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = video_settings::Settings {
            default_video_duration_seconds: input.default_duration_seconds,
        };
        let calls = input
            .calls
            .iter()
            .map(|call| resolve(call, settings))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": calls }))
    }
}

fn resolve(call: &Call, settings: video_settings::Settings) -> HelperResult {
    match call {
        Call::CreateResponse {
            model,
            provider,
            request_data,
            response,
        } => create_answer(
            model,
            provider.as_deref(),
            request_data.as_ref(),
            response,
            settings,
        ),
        Call::StatusResponse {
            provider,
            response,
            now,
            time_zone,
        } => status_answer(provider.as_deref(), response, *now, time_zone),
        Call::ContentResponse { response } => content_answer(response),
        Call::EditResponse {
            provider,
            request_data,
            response,
        } => edit_answer(
            provider.as_deref(),
            request_data.as_ref(),
            response,
            settings,
        ),
        Call::FetchRequest { video_id, api_base } => {
            let request = fetch_operation_request(video_id, api_base).or_value_error()?;
            Ok(json!({ "url": request.url, "params": request.params }))
        }
    }
}

fn create_answer(
    model: &str,
    provider: Option<&str>,
    request_data: Option<&Object>,
    response: &Value,
    settings: video_settings::Settings,
) -> HelperResult {
    let body = serde_json::to_vec(response).or_value_error()?;
    let video = create_response(model, &body, provider, request_data, settings).or_value_error()?;
    serde_json::to_value(video).or_value_error()
}

fn status_answer(
    provider: Option<&str>,
    response: &Value,
    now: i64,
    time_zone: &str,
) -> HelperResult {
    let body = serde_json::to_vec(response).or_value_error()?;
    let zone = TimeZone::get(time_zone).or_value_error()?;
    let local = LocalTimeContext { zone };
    let video = status_response(&body, provider, now, &local).or_value_error()?;
    serde_json::to_value(video).or_value_error()
}

fn content_answer(response: &Value) -> HelperResult {
    let body = serde_json::to_vec(response).or_value_error()?;
    let content = content_response(&body).or_value_error()?;
    Ok(json!({ "bytes": content }))
}

fn edit_answer(
    provider: Option<&str>,
    request_data: Option<&Object>,
    response: &Value,
    settings: video_settings::Settings,
) -> HelperResult {
    let body = serde_json::to_vec(response).or_value_error()?;
    let video = edit_response(&body, provider, request_data, settings).or_value_error()?;
    serde_json::to_value(video).or_value_error()
}
