use std::collections::BTreeMap;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_to_chat_stream::error::ResponsesChatStreamError;
use betterlite_core::api::responses_to_chat_stream::records::{ChatChunk, ChunkFactory};
use betterlite_core::api::responses_to_chat_stream::stream::{
    ParsedChatLine, get_model_response_iterator,
};
use betterlite_core::api::responses_to_chat_stream::translate::{
    ToolCallIndices, translate_responses_chunk_to_openai_stream,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult};

const FIXED_ID: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;
const FIXED_CLOCK: i64 = 1_700_000_000;

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Event {
        events: Vec<Map<String, Value>>,
    },
    Line {
        lines: Vec<String>,
    },
    Stateless {
        events: Vec<Map<String, Value>>,
        #[serde(default)]
        tool_call_index_map: Option<BTreeMap<i64, i64>>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Event { events } => event_steps(events),
            Input::Line { lines } => line_steps(lines),
            Input::Stateless {
                events,
                tool_call_index_map,
            } => stateless_steps(events, tool_call_index_map.clone()),
        }
    }
}

fn event_steps(events: &[Map<String, Value>]) -> HelperResult {
    let mut iterator = get_model_response_iterator(Some(false), || Uuid::from_u128(FIXED_ID), || FIXED_CLOCK);
    let mut steps = Vec::with_capacity(events.len());
    for event in events {
        let chunk = iterator
            .chunk_parser(&Value::Object(event.clone()))
            .map_err(|error| failure(&error))?;
        steps.push(rendered(&chunk)?);
    }
    Ok(json!({ "steps": steps }))
}

// the Python side can only answer with a chunk, so a padding line is the helper's own error there too
fn line_steps(lines: &[String]) -> HelperResult {
    let mut iterator = get_model_response_iterator(Some(false), || Uuid::from_u128(FIXED_ID), || FIXED_CLOCK);
    let mut steps = Vec::with_capacity(lines.len());
    for line in lines {
        match iterator
            .parse_line(line.as_bytes(), &ApiLimits::default())
            .map_err(|error| failure(&error))?
        {
            ParsedChatLine::Padding(_) => {
                return Err(Box::new(HelperError {
                    class: "ValueError".to_owned(),
                    status: None,
                    error_type: Some(None),
                    param: Some(None),
                    code: Some(None),
                    message: "the padding line carries no chunk".to_owned(),
                }));
            }
            ParsedChatLine::Chunk(chunk) => steps.push(rendered(&chunk)?),
        }
    }
    Ok(json!({ "steps": steps }))
}

fn stateless_steps(
    events: &[Map<String, Value>],
    shared_map: Option<BTreeMap<i64, i64>>,
) -> HelperResult {
    let shares_map = shared_map.is_some();
    let mut map: ToolCallIndices = shared_map.unwrap_or_default();
    let factory = ChunkFactory {
        identifier: || Uuid::from_u128(FIXED_ID),
        clock: || FIXED_CLOCK,
    };
    let mut steps = Vec::with_capacity(events.len());
    for event in events {
        let indices = if shares_map { Some(&mut map) } else { None };
        let chunk = translate_responses_chunk_to_openai_stream(
            &Value::Object(event.clone()),
            indices,
            factory,
        )
        .map_err(|error| failure(&error))?;
        steps.push(rendered(&chunk)?);
    }
    Ok(json!({ "steps": steps }))
}

fn rendered(chunk: &ChatChunk) -> HelperResult {
    serde_json::to_value(chunk).map_err(|source| {
        Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: source.to_string(),
        })
    })
}

fn failure(error: &ResponsesChatStreamError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
