use std::collections::BTreeMap;

use betterlite_a2a::completion_bridge::wire::MessageSendParams;
use betterlite_core::agents::json::AgentJson;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};
use calls::{config, non_streaming, signed, sse, streaming};
use fixture::{Answer, StreamAnswer};

mod calls;
mod fixture;
mod tests;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    SignedRequest {
        request_id: String,
        params: MessageSendParams,
        litellm_params: BTreeMap<String, AgentJson>,
        #[serde(default)]
        agent_extra_headers: Option<BTreeMap<String, String>>,
    },
    ParseSse {
        lines: Vec<String>,
    },
    NonStreaming {
        request_id: String,
        params: MessageSendParams,
        litellm_params: BTreeMap<String, AgentJson>,
        #[serde(default)]
        agent_extra_headers: Option<BTreeMap<String, String>>,
        answer: Answer,
    },
    Streaming {
        request_id: String,
        params: MessageSendParams,
        litellm_params: BTreeMap<String, AgentJson>,
        #[serde(default)]
        agent_extra_headers: Option<BTreeMap<String, String>>,
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
            Input::SignedRequest {
                request_id,
                params,
                litellm_params,
                agent_extra_headers,
            } => signed(
                request_id,
                params,
                litellm_params,
                agent_extra_headers.as_ref(),
            ),
            Input::ParseSse { lines } => sse(lines),
            Input::NonStreaming {
                request_id,
                params,
                litellm_params,
                agent_extra_headers,
                answer,
            } => non_streaming(
                request_id,
                params,
                litellm_params,
                agent_extra_headers.as_ref(),
                answer,
            ),
            Input::Streaming {
                request_id,
                params,
                litellm_params,
                agent_extra_headers,
                answer,
            } => streaming(
                request_id,
                params,
                litellm_params,
                agent_extra_headers.as_ref(),
                answer,
            ),
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
