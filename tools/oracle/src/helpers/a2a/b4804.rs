use std::collections::BTreeMap;

use betterlite_a2a::completion_bridge::wire::MessageSendParams;
use betterlite_core::agents::json::AgentJson;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};
use calls::{Mode, call, config, sse};

mod calls;
mod fixture;
mod tests;

pub(crate) struct Op;

#[derive(Clone, Deserialize)]
pub(crate) struct Answer {
    status: u16,
    body: AgentJson,
}

#[derive(Clone, Deserialize)]
pub(crate) struct StreamAnswer {
    status: u16,
    content_type: String,
    lines: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct Transport {
    request_id: String,
    params: MessageSendParams,
    litellm_params: BTreeMap<String, AgentJson>,
    #[serde(default)]
    agent_extra_headers: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    SignedRequest {
        #[serde(flatten)]
        transport: Transport,
    },
    ParseSse {
        lines: Vec<String>,
    },
    NonStreaming {
        #[serde(flatten)]
        transport: Transport,
        answer: Answer,
    },
    Streaming {
        #[serde(flatten)]
        transport: Transport,
        answer: StreamAnswer,
    },
    Config {
        request_id: String,
        params: MessageSendParams,
        #[serde(default)]
        litellm_params: Option<BTreeMap<String, AgentJson>>,
        answer: Answer,
        stream_answer: StreamAnswer,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::SignedRequest { transport } => call(transport, Mode::Signed),
            Input::ParseSse { lines } => sse(lines),
            Input::NonStreaming { transport, answer } => call(transport, Mode::Buffered(answer)),
            Input::Streaming { transport, answer } => call(transport, Mode::Streamed(answer)),
            Input::Config {
                request_id,
                params,
                litellm_params,
                answer,
                stream_answer,
            } => config(
                request_id,
                params,
                litellm_params.as_ref(),
                answer,
                stream_answer,
            ),
        }
    }
}
