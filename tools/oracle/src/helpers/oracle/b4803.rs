use betterlite_a2a::completion_bridge::streaming::{EventContent, stream_event};
use betterlite_a2a::completion_bridge::wire::{A2AMessage, CompletionParams, ModelResponse};
use betterlite_a2a::completion_bridge::{
    A2AStreamingContext, EventSource, apply_forward_metadata, extract_text, response_to_a2a,
    to_openai_messages,
};
use betterlite_core::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Extract,
    Metadata,
    Roles,
    Response,
    Task,
    Working,
    Completed,
    Artifact,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    message: A2AMessage,
    #[serde(default)]
    parts: Vec<AgentJson>,
    #[serde(default)]
    params: Option<CompletionParams>,
    #[serde(default)]
    completion_params: CompletionParams,
    #[serde(default)]
    response: ModelResponse,
}

struct Source {
    ids: RangeInclusive<u128>,
}

impl EventSource for Source {
    fn next_uuid(&mut self) -> Uuid {
        Uuid::from_u128(self.ids.next().unwrap_or_default())
    }
    fn now(&mut self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }
}

fn output<T>(value: &T) -> HelperResult
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|error| {
        Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut source = Source { ids: 1..=10 };
        match input.operation {
            Operation::Extract => output(&extract_text(&input.parts)),
            Operation::Metadata => {
                let mut completion = input.completion_params.clone();
                apply_forward_metadata(&mut completion, &input.message, input.params.as_ref());
                output(&completion)
            }
            Operation::Roles => output(&to_openai_messages(&input.message)),
            Operation::Response => output(&response_to_a2a(
                Some(&input.response),
                input.request_id.as_deref(),
                &mut source,
            )),
            Operation::Task => run_event(input, EventContent::Task, &mut source),
            Operation::Working => run_event(
                input,
                EventContent::Status {
                    state: "working",
                    final_event: false,
                    message: Some("thinking"),
                },
                &mut source,
            ),
            Operation::Completed => run_event(
                input,
                EventContent::Status {
                    state: "completed",
                    final_event: true,
                    message: None,
                },
                &mut source,
            ),
            Operation::Artifact => run_event(input, EventContent::Artifact("answer"), &mut source),
        }
    }
}

fn run_event(input: &Input, event: EventContent<'_>, source: &mut Source) -> HelperResult {
    let context = A2AStreamingContext::new(
        input
            .request_id
            .clone()
            .unwrap_or_else(|| "request".to_owned()),
        input.message.clone(),
        source,
    );
    output(&stream_event(&context, event, source))
}
