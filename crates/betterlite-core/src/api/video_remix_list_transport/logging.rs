use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::LogContext;
use super::records::{Headers, Params};

pub(super) struct RemixPreCall<'call> {
    pub(super) prompt: &'call str,
    pub(super) url: &'call str,
    pub(super) headers: &'call Headers,
    pub(super) data: &'call Params,
    pub(super) video_id: &'call str,
}

pub(super) struct ListPreCall<'call> {
    pub(super) url: &'call str,
    pub(super) headers: &'call Headers,
    pub(super) params: &'call Params,
}

// Which handler is logging; only its input and its own extra arguments differ
pub(super) enum PreCallSource<'call> {
    Remix(&'call RemixPreCall<'call>),
    List(&'call ListPreCall<'call>),
}

fn logged_headers(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

fn base_args(url: &str, headers: &Headers) -> AdditionalArgs {
    AdditionalArgs {
        api_base: Some(url.to_owned()),
        headers: Some(logged_headers(headers)),
        complete_input_dict: None,
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}

// A remix logs the prompt as its input, with the body and the video id as extra arguments; a list logs the empty string and the query params
pub(super) fn record_pre_call(
    log: &mut LogContext<'_, '_>,
    source: PreCallSource<'_>,
) -> Result<(), PrePostError> {
    let record = match source {
        PreCallSource::Remix(call) => PreCall {
            input: Json::Text(call.prompt.to_owned()),
            model: None,
            additional_args: AdditionalArgs {
                complete_input_dict: Some(RawBody::Object(call.data.clone())),
                passthrough: IndexMap::from([(
                    String::from("video_id"),
                    Json::Text(call.video_id.to_owned()),
                )]),
                ..base_args(call.url, call.headers)
            },
        },
        PreCallSource::List(call) => PreCall {
            input: Json::Text(String::new()),
            model: None,
            additional_args: AdditionalArgs {
                complete_input_dict: None,
                passthrough: IndexMap::from([(
                    String::from("params"),
                    Json::Object(call.params.clone()),
                )]),
                ..base_args(call.url, call.headers)
            },
        },
    };
    pre_call(log.state, &record, log.env).map(drop)
}
