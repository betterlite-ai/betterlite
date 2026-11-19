use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::UsageStartTime;
use super::catalog::MODEL_INSIGHT_TASKS;

const INTERNAL_CALL_ORIGIN: &str = "internal_call_origin";
const TASK_TAG_PREFIX: &str = "task:";
const UNCATEGORIZED_TASK: &str = "uncategorized";
const DATE_FORMAT: &str = "%Y-%m-%d";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelUsageFields {
    pub internal_call: bool,
    pub task_type: String,
}

// model_usage_task_type and _is_internal_call: both fields are JSON text, and text pydantic rejects is neither internal nor classified.
#[must_use]
pub fn decode_model_usage_fields(metadata: &[u8], request_tags: &[u8]) -> ModelUsageFields {
    let internal_call = match serde_json::from_slice::<IndexMap<String, Json>>(metadata) {
        Ok(decoded) => decoded
            .get(INTERNAL_CALL_ORIGIN)
            .is_some_and(json_has_value),
        Err(_invalid) => false,
    };
    let task_type = match serde_json::from_slice::<Vec<Json>>(request_tags) {
        Ok(tags) => first_known_task(&tags).unwrap_or_else(|| UNCATEGORIZED_TASK.to_owned()),
        Err(_invalid) => UNCATEGORIZED_TASK.to_owned(),
    };
    ModelUsageFields {
        internal_call,
        task_type,
    }
}

// _date_from_start_time: a real datetime keeps its own date, while a string gives up its first ten characters only when it has that many.
#[must_use]
pub(crate) fn usage_date(start_time: &UsageStartTime) -> Option<String> {
    match start_time {
        UsageStartTime::Parsed(instant) => {
            Some(instant.date_naive().format(DATE_FORMAT).to_string())
        }
        UsageStartTime::Text(text) => {
            let prefix: String = text.chars().take(10).collect();
            (prefix.chars().count() == 10).then_some(prefix)
        }
    }
}

// the first string tag holding a known task: the prefix is removed before the catalog lookup, so "task:code_review" selects code_review.
fn first_known_task(tags: &[Json]) -> Option<String> {
    tags.iter().find_map(|tag| {
        let Json::Text(text) = tag else {
            return None;
        };
        let task = text.strip_prefix(TASK_TAG_PREFIX)?;
        MODEL_INSIGHT_TASKS
            .iter()
            .any(|known| known.task_type == task)
            .then(|| task.to_owned())
    })
}

// bool(decoded.get(...)): Python's truthiness over a JSON value, the only reading an object-typed metadata field has.
fn json_has_value(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_none_or(|value| value != 0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(entries) => !entries.is_empty(),
    }
}
