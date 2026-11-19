#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::compression_interception_state::Settings;
use betterlite_context::agentic_callback::AgenticLogging;
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, FollowupMessage, JsonMap};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use serde_json::value::to_raw_value;
use thiserror::Error;

use crate::api::compression_interception::cache::Clock;
use crate::api::compression_interception::engine::{
    CompressionCall, CompressionEngine, CompressionOutput,
};
use crate::api::compression_interception::hook::CompressionInterception;
use crate::api::compression_interception::tools::CONTENT_RETRIEVE_TOOL_NAME;
use crate::api::compression_retrieval_loop::plan::RetrievalLoopRequest;

pub(super) const MODEL: &str = "claude-sonnet-4";
pub(super) const NOW: f64 = 1_000.0;

#[derive(Debug, Error)]
#[error("the retrieval loop never compresses")]
pub(super) struct Unused;

pub(super) struct IdleEngine;

#[async_trait]
impl CompressionEngine for IdleEngine {
    type Failure = Unused;

    async fn compress(&self, _call: &CompressionCall<'_>) -> Result<CompressionOutput, Unused> {
        Err(Unused)
    }
}

pub(super) fn settings(enabled: bool) -> Settings {
    Settings {
        enabled,
        compression_trigger: 200_000,
        compression_target: None,
        embedding_model: None,
        embedding_model_params: None,
    }
}

pub(super) fn interception(enabled: bool) -> CompressionInterception<IdleEngine> {
    CompressionInterception::with_clock(settings(enabled), Arc::new(IdleEngine), fixed_clock(NOW))
}

pub(super) fn interception_at(clock: Clock) -> CompressionInterception<IdleEngine> {
    CompressionInterception::with_clock(settings(true), Arc::new(IdleEngine), clock)
}

pub(super) fn fixed_clock(now: f64) -> Clock {
    Arc::new(move || now)
}

pub(super) fn manual_clock(start: f64) -> (Clock, Arc<Mutex<f64>>) {
    let value = Arc::new(Mutex::new(start));
    let handle = Arc::clone(&value);
    (
        Arc::new(move || *handle.lock().unwrap_or_else(PoisonError::into_inner)),
        value,
    )
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(super) fn list(items: Vec<Json>) -> Json {
    Json::List(items)
}

pub(super) fn object(pairs: Vec<(&str, Json)>) -> Json {
    Json::Object(index(pairs))
}

pub(super) fn index(pairs: Vec<(&str, Json)>) -> IndexMap<String, Json> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(super) fn retrieval_tools() -> Json {
    list(vec![object(vec![
        ("type", text("custom")),
        ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
    ])])
}

pub(super) fn other_tools() -> Json {
    list(vec![object(vec![
        ("type", text("custom")),
        ("name", text("other_tool")),
    ])])
}

pub(super) fn response(content: Vec<Json>) -> Json {
    object(vec![("content", list(content))])
}

pub(super) fn retrieval_block(id: Option<&str>, input: Option<Json>) -> Json {
    let mut pairs = vec![
        ("type", text("tool_use")),
        ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
    ];
    if let Some(id) = id {
        pairs.push(("id", text(id)));
    }
    if let Some(input) = input {
        pairs.push(("input", input));
    }
    object(pairs)
}

pub(super) fn tool_use(id: Json, input: Json) -> Json {
    object(vec![
        ("type", text("tool_use")),
        ("id", id),
        ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
        ("input", input),
    ])
}

pub(super) fn retrieval_call(id: &str, key: Option<&str>) -> Json {
    let input = match key {
        Some(key) => object(vec![("key", text(key))]),
        None => object(Vec::new()),
    };
    tool_use(text(id), input)
}

pub(super) fn tool_result(id: Json, content: &str) -> Json {
    object(vec![
        ("type", text("tool_result")),
        ("tool_use_id", id),
        ("content", text(content)),
    ])
}

pub(super) fn loop_tools(
    tool_calls: Vec<Json>,
    thinking_blocks: Vec<Json>,
) -> IndexMap<String, Json> {
    index(vec![
        ("tool_calls", list(tool_calls)),
        ("thinking_blocks", list(thinking_blocks)),
    ])
}

pub(super) fn cache_map(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn logging(call_id: &str, agentic_model: Option<&str>) -> AgenticLogging {
    AgenticLogging {
        call_id: Some(call_id.to_owned()),
        agentic_loop_params: agentic_model.map(|model| raw_map(vec![("model", text(model))])),
    }
}

pub(super) fn request<'request>(
    messages: &'request [Json],
    optional_params: &'request IndexMap<String, Json>,
    logging: Option<&'request AgenticLogging>,
    kwargs: &'request IndexMap<String, Json>,
) -> RetrievalLoopRequest<'request> {
    RetrievalLoopRequest {
        model: MODEL,
        messages,
        optional_params,
        logging,
        kwargs,
    }
}

pub(super) fn plan(
    interception: &CompressionInterception<IdleEngine>,
    tools: &IndexMap<String, Json>,
    request: &RetrievalLoopRequest<'_>,
) -> AgenticLoopPlan {
    interception
        .build_agentic_loop_plan(tools, request)
        .expect("the plan builds")
}

pub(super) fn raw_map(pairs: Vec<(&str, Json)>) -> JsonMap {
    let mut map = JsonMap::new();
    for (key, value) in pairs {
        let raw = to_raw_value(&value).expect("the value serializes");
        map.insert(key.to_owned(), raw);
    }
    map
}

pub(super) fn raw_json(map: &JsonMap) -> IndexMap<String, Json> {
    map.iter()
        .map(|(key, value)| {
            (
                key.clone(),
                serde_json::from_str(value.get()).expect("the value parses"),
            )
        })
        .collect()
}

pub(super) fn message_json(message: &FollowupMessage) -> Json {
    serde_json::from_str(message.0.get()).expect("the message parses")
}

pub(super) fn followups(plan: &AgenticLoopPlan) -> Vec<Json> {
    let patch = plan.request_patch.as_ref().expect("the patch");
    let messages = patch.messages.as_ref().expect("the messages");
    messages.iter().map(message_json).collect()
}

pub(super) fn tool_results(plan: &AgenticLoopPlan) -> Vec<Json> {
    let last = followups(plan).pop().expect("the user message");
    let Json::Object(fields) = last else {
        panic!("the user message is an object");
    };
    let Some(Json::List(results)) = fields.get("content") else {
        panic!("the user message content is a list");
    };
    results.clone()
}
