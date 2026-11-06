use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::thread_runs::decode::{classify, insert_keys};
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
    let Value::Object(object) = value else {
        return;
    };
    if let Some(Value::Array(tools)) = object.get_mut("tools") {
        tools.iter_mut().for_each(normalise_tool);
    }
    if let Some(format) = object.get_mut("response_format") {
        normalise_format(format);
    }
    if let Some(Value::Object(choice)) = object.get_mut("tool_choice") {
        choice.entry("function").or_insert(Value::Null);
    }
    if let Some(Value::Object(details)) = object.get_mut("incomplete_details") {
        details.entry("reason").or_insert(Value::Null);
    }
    if let Some(Value::Object(strategy)) = object.get_mut("truncation_strategy") {
        strategy.entry("last_messages").or_insert(Value::Null);
    }
}
fn normalise_tool(value: &mut Value) {
    let Value::Object(object) = value else {
        return;
    };
    match object.get("type").and_then(Value::as_str) {
        Some("file_search") => {
            let entry = object.entry("file_search").or_insert(Value::Null);
            if let Value::Object(list) = entry {
                insert_keys(list, "max_num_results ranking_options");
                if let Some(Value::Object(ranking)) = list.get_mut("ranking_options") {
                    insert_keys(ranking, "ranker score_threshold");
                }
            }
        }
        Some("function") => {
            if let Some(Value::Object(function)) = object.get_mut("function") {
                insert_keys(function, "description parameters strict");
            }
        }
        _ => {}
    }
}
fn normalise_format(value: &mut Value) {
    let Value::Object(object) = value else {
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
