use serde::Deserialize;
use serde_json::Value;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use media::Environment;

mod decisions;
mod media;
mod rerank;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    RerankParams {
        calls: Vec<rerank::ParamsCall>,
    },
    RerankRequest {
        calls: Vec<rerank::RequestCall>,
    },
    RerankReply {
        calls: Vec<rerank::ReplyCall>,
    },
    ResponsesUrl {
        calls: Vec<media::EnvCall>,
    },
    ImageEditUrl {
        calls: Vec<media::EnvCall>,
    },
    TranscriptionRequest {
        calls: Vec<media::TranscriptionCall>,
    },
    VideoForm {
        calls: Vec<media::VideoCall>,
    },
    DecisionsKey {
        calls: Vec<decisions::KeyCall>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::RerankParams { calls } => collect(calls, rerank::params_value),
            Input::RerankRequest { calls } => collect(calls, rerank::request_value),
            Input::RerankReply { calls } => collect(calls, rerank::reply_value),
            Input::ResponsesUrl { calls } => collect(calls, |call| {
                media::environment_value(Environment::Responses, call)
            }),
            Input::ImageEditUrl { calls } => collect(calls, |call| {
                media::environment_value(Environment::ImageEdit, call)
            }),
            Input::TranscriptionRequest { calls } => collect(calls, media::transcription_value),
            Input::VideoForm { calls } => collect(calls, media::video_value),
            Input::DecisionsKey { calls } => collect(calls, decisions::key_value),
        }
    }
}

fn collect<Call, Make>(calls: &[Call], make: Make) -> HelperResult
where
    Make: Fn(&Call) -> Result<Value, Box<HelperError>>,
{
    calls
        .iter()
        .map(make)
        .collect::<Result<Vec<Value>, _>>()
        .map(Value::Array)
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
