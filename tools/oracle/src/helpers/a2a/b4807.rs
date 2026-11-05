use std::num::NonZeroUsize;
use std::time::Duration;

use betterlite_a2a::completion_bridge::EventSource;
use betterlite_a2a::pydantic_ai_transformation::fake_streaming::StreamPacing;
use betterlite_a2a::watsonx_credentials::params::{WxoLitellmParams, extract_litellm_params};
use betterlite_a2a::watsonx_transform::extract::{
    WxoRunResult, WxoSendParams, extract_text_from_a2a_params, extract_text_from_wxo_result,
};
use betterlite_a2a::watsonx_transform::fake_streaming::{
    WxoStreamResult, WxoStreamStep, fake_streaming_from_text,
};
use betterlite_a2a::watsonx_transform::response::build_a2a_message_response;
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;
mod token;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Config {
        litellm_params: WxoLitellmParams,
    },
    Token {
        steps: Vec<token::Step>,
    },
    ParamsText {
        params: WxoSendParams,
    },
    ResultText {
        result: WxoRunResult,
    },
    Response {
        request_id: String,
        text: String,
    },
    Stream {
        text: String,
        request_id: Option<String>,
        chunk_size: Option<NonZeroUsize>,
        delay_ms: Option<u64>,
    },
}

#[derive(Serialize)]
struct Step<'event, E> {
    event: &'event E,
    delay_ms: u64,
}

struct Source {
    next: u128,
}

impl EventSource for Source {
    fn next_uuid(&mut self) -> Uuid {
        self.next = self.next.saturating_add(1);
        Uuid::from_u128(self.next)
    }
    fn now(&mut self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn config_value(raw: &WxoLitellmParams) -> HelperResult {
    let params = extract_litellm_params(raw.clone())
        .map_err(|error| failure("ValueError", error.to_string()))?;
    Ok(serde_json::json!({
        "cp4d_host": params.cp4d_host,
        "instance_id": params.instance_id,
        "wxo_agent_id": params.wxo_agent_id,
        "api_key": params.api_key.expose_secret(),
        "username": params.username,
        "auth_mode": params.auth_mode,
        "thread_id": params.thread_id,
    }))
}

fn pace(steps: &[WxoStreamStep]) -> Vec<Step<'_, WxoStreamResult>> {
    steps
        .iter()
        .map(|step| Step {
            event: &step.event,
            delay_ms: u64::try_from(step.delay_after.as_millis()).unwrap_or(u64::MAX),
        })
        .collect()
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut source = Source { next: 0 };
        let encoded = match input {
            Input::Config { litellm_params } => return config_value(litellm_params),
            Input::Token { steps } => return token::run_steps(steps),
            Input::ParamsText { params } => {
                serde_json::to_value(extract_text_from_a2a_params(params))
            }
            Input::ResultText { result } => {
                serde_json::to_value(extract_text_from_wxo_result(result))
            }
            Input::Response { request_id, text } => {
                serde_json::to_value(build_a2a_message_response(request_id, text, &mut source))
            }
            Input::Stream {
                text,
                request_id,
                chunk_size,
                delay_ms,
            } => {
                let defaults = StreamPacing::default();
                let pacing = StreamPacing {
                    chunk_size: chunk_size.unwrap_or(defaults.chunk_size),
                    delay: delay_ms.map_or(defaults.delay, Duration::from_millis),
                };
                let id = request_id.as_deref().unwrap_or("request");
                let steps = fake_streaming_from_text(text, id, pacing, &mut source);
                serde_json::to_value(pace(&steps))
            }
        };
        encoded.map_err(|error| failure("SerializationError", error.to_string()))
    }
}
