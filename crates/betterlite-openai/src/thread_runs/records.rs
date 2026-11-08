use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::thread_runs::decode::{add_keys, classify, fill_map, fill_object, insert_keys};
use crate::thread_runs::error::ThreadRunsError;
use serde_json::{Number, Value};

// Declared fields the provider left out stay null in the reply, as the SDK model dump keeps them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReply {
    pub id: String,
    pub assistant_id: String,
    pub cancelled_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub failed_at: Option<i64>,
    pub incomplete_details: Option<Value>,
    pub instructions: String,
    pub last_error: Option<Value>,
    pub max_completion_tokens: Option<i64>,
    pub max_prompt_tokens: Option<i64>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub model: String,
    pub object: String,
    pub parallel_tool_calls: bool,
    pub required_action: Option<Value>,
    pub response_format: Option<Value>,
    pub started_at: Option<i64>,
    pub status: String,
    pub thread_id: String,
    pub tool_choice: Option<Value>,
    pub tools: Vec<Value>,
    pub truncation_strategy: Option<Value>,
    pub usage: Option<Value>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum RunEvent {
    Named {
        event: String,
        data: Value,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    Bare(Value),
}
impl RunEvent {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Named { event, .. } => event,
            Self::Bare(_) => "payload",
        }
    }
}
// The keyword arguments LiteLLM's run wrappers receive; thread_id is the path and never the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub thread_id: String,
    pub assistant_id: String,
    pub additional_instructions: Option<String>,
    pub instructions: Option<String>,
    pub metadata: Option<Value>,
    pub model: Option<String>,
    pub tools: Option<Value>,
}

pub(crate) fn reply(bytes: &[u8]) -> Result<RunReply, ThreadRunsError> {
    serde_json::from_slice::<Value>(bytes)
        .map_err(classify)
        .and_then(reply_value)
}
pub(crate) fn reply_value(mut value: Value) -> Result<RunReply, ThreadRunsError> {
    normalise_reply(&mut value);
    serde_json::from_value(value).map_err(classify)
}
fn normalise_reply(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(Value::Array(tools)) = object.get_mut("tools") {
        tools.iter_mut().for_each(normalise_tool);
    }
    if let Some(format) = object.get_mut("response_format") {
        normalise_format(format);
    }
    for (key, inner) in [
        ("tool_choice", "function"),
        ("incomplete_details", "reason"),
        ("truncation_strategy", "last_messages"),
    ] {
        fill_map(object, key, inner);
    }
}
fn normalise_tool(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    match object.get("type").and_then(Value::as_str) {
        Some("file_search") => {
            let list = object.entry("file_search").or_insert(Value::Null);
            add_keys(list, "max_num_results ranking_options");
            fill_object(list, "ranking_options", "ranker score_threshold");
        }
        Some("function") => fill_map(object, "function", "description parameters strict"),
        _ => {}
    }
}
fn normalise_format(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object.get("type").and_then(Value::as_str) != Some("json_schema") {
        return;
    }
    let Some(Value::Object(schema)) = object.get_mut("json_schema") else {
        return;
    };
    insert_keys(schema, "description strict");
    if let Some(item) = schema.remove("schema") {
        schema.insert(String::from("schema_"), item);
    }
    schema.entry("schema_").or_insert(Value::Null);
}
