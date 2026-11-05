use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::ops::RangeInclusive;
use std::time::Duration;

use betterlite_a2a::completion_bridge::EventSource;
use betterlite_a2a::pydantic_ai_transformation::extract::extract_response_text;
use betterlite_a2a::pydantic_ai_transformation::fake_streaming::events::StreamEvent;
use betterlite_a2a::pydantic_ai_transformation::fake_streaming::{
    StreamPacing, fake_streaming_from_response,
};
use betterlite_a2a::pydantic_ai_transformation::transform::transform_to_a2a_response;
use betterlite_a2a::pydantic_ai_transformation::wire::TaskResponse;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Extract,
    Transform,
    Stream,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    request_id: Option<String>,
    #[serde(default)]
    response: TaskResponse,
    chunk_size: Option<NonZeroUsize>,
    delay_ms: Option<u64>,
}

#[derive(Serialize)]
struct Step<'event> {
    event: &'event StreamEvent,
    delay_ms: u64,
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

fn serialization_error(error: &serde_json::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn normalize(value: &mut serde_json::Value, seen: &mut HashMap<String, String>) {
    match value {
        serde_json::Value::String(text) => {
            if text.len() == 36 && text.starts_with("00000000-0000-0000-0000-") {
                let next = format!("uuid-{}", seen.len());
                let replacement = seen.entry(text.clone()).or_insert(next);
                text.clone_from(replacement);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                normalize(item, seen);
            }
        }
        serde_json::Value::Object(fields) => {
            for field in fields.values_mut() {
                normalize(field, seen);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut source = Source { ids: 1..=99 };
        let request_id = input.request_id.as_deref().unwrap_or("request");
        let mut value = match input.operation {
            Operation::Extract => {
                let extracted = extract_response_text(&input.response, &mut source);
                serde_json::to_value((extracted.text, extracted.message_id, extracted.parts))
            }
            Operation::Transform => serde_json::to_value(transform_to_a2a_response(
                &input.response,
                request_id,
                &mut source,
            )),
            Operation::Stream => {
                let defaults = StreamPacing::default();
                let pacing = StreamPacing {
                    chunk_size: input.chunk_size.unwrap_or(defaults.chunk_size),
                    delay: input.delay_ms.map_or(defaults.delay, Duration::from_millis),
                };
                let steps =
                    fake_streaming_from_response(&input.response, request_id, pacing, &mut source);
                serde_json::to_value(
                    steps
                        .iter()
                        .map(|step| Step {
                            event: &step.event,
                            delay_ms: u64::try_from(step.delay_after.as_millis())
                                .unwrap_or(u64::MAX),
                        })
                        .collect::<Vec<_>>(),
                )
            }
        }
        .map_err(|error| serialization_error(&error))?;
        normalize(&mut value, &mut HashMap::new());
        Ok(value)
    }
}
