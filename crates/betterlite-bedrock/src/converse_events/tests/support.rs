use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::converse_events::ConverseStreamState;
use crate::converse_events::error::ConverseEventError;

pub(super) const FIRST: &str = "00000000-0000-4000-8000-000000000001";
pub(super) const SECOND: &str = "00000000-0000-4000-8000-000000000002";

pub(super) struct FixedIds {
    values: Mutex<VecDeque<Uuid>>,
}

impl FixedIds {
    fn new(values: &[&str]) -> Self {
        let parsed = values
            .iter()
            .map(|value| Uuid::parse_str(value).unwrap())
            .collect();
        Self {
            values: Mutex::new(parsed),
        }
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or_else(Uuid::nil)
    }
}

pub(super) fn state(model: &str, json_mode: bool, ids: &[&str]) -> ConverseStreamState {
    ConverseStreamState {
        content_blocks: Vec::new(),
        tool_calls_index: None,
        response_id: None,
        current_tool_name: None,
        thinking_ran: false,
        provider_reasoning_tokens: None,
        json_mode,
        model: model.to_owned(),
        mappings: IndexMap::new(),
        ids: Arc::new(FixedIds::new(ids)),
    }
}

pub(super) fn event(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn as_json<Payload>(value: &Payload) -> Json
where
    Payload: serde::Serialize,
{
    let encoded = serde_json::to_string(value).unwrap();
    serde_json::from_str(&encoded).unwrap()
}

pub(super) fn converse(state: &mut ConverseStreamState, source: &str) -> Json {
    let chunk = state.converse_chunk_parser(&event(source)).unwrap();
    as_json(&chunk)
}

pub(super) fn assert_converse(state: &mut ConverseStreamState, source: &str, expected: &str) {
    let parsed: Json = serde_json::from_str(expected).unwrap();
    assert_eq!(converse(state, source), parsed);
}

pub(super) fn project(state: &mut ConverseStreamState, source: &str) -> Json {
    let chunk = state.chunk_parser(&event(source)).unwrap();
    as_json(&chunk)
}

pub(super) fn assert_projection(state: &mut ConverseStreamState, source: &str, expected: &str) {
    let parsed: Json = serde_json::from_str(expected).unwrap();
    assert_eq!(project(state, source), parsed);
}

pub(super) fn converse_error(state: &mut ConverseStreamState, source: &str) -> ConverseEventError {
    let ConverseEventError::Streaming(inner) =
        state.converse_chunk_parser(&event(source)).unwrap_err()
    else {
        panic!("the Converse parser must wrap its failures");
    };
    *inner
}

pub(super) fn chunk_error(state: &mut ConverseStreamState, source: &str) -> ConverseEventError {
    state.chunk_parser(&event(source)).unwrap_err()
}

pub(super) fn empty_delta(id: &str, model: &str) -> String {
    format!(
        r#"{{"id":"{id}","model":"{model}","finish_reason":null,"index":0,"delta":{{"content":null,"role":null,"tool_calls":null,"provider_specific_fields":null,"thinking_blocks":null,"reasoning_content":null}},"usage":null,"provider_specific_fields":{{}}}}"#
    )
}

pub(super) fn text_delta(id: &str, model: &str, content: &str) -> String {
    format!(
        r#"{{"id":"{id}","model":"{model}","finish_reason":null,"index":0,"delta":{{"content":{content},"role":"assistant","tool_calls":null,"provider_specific_fields":null,"thinking_blocks":null,"reasoning_content":null}},"usage":null,"provider_specific_fields":{{}}}}"#
    )
}

pub(super) fn generic(text: &str, is_finished: bool, finish_reason: &str) -> String {
    format!(
        r#"{{"generic":{{"text":{text},"tool_use":null,"is_finished":{is_finished},"finish_reason":{finish_reason},"usage":null,"index":0,"provider_specific_fields":null}}}}"#
    )
}
