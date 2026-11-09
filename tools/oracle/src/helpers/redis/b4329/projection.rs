use std::collections::BTreeMap;

use betterlite_redis::list_operations::results::{LpopPipelineEntry, LpopResult};
use serde_json::{Map, Value, json};

use super::operations::RecordedEvent;

pub(crate) fn lpop_result(result: &LpopResult) -> Value {
    match result {
        LpopResult::Missing => Value::Null,
        LpopResult::Item(text) => json!(text),
        LpopResult::ItemBytes(bytes) => json!({ "bytes": bytes }),
        LpopResult::Items(items) => json!(items),
        LpopResult::ItemsBytes(items) => json!(items),
    }
}

pub(crate) fn lists(lists: &BTreeMap<String, Vec<Vec<u8>>>) -> Value {
    let mut object = Map::new();
    for (key, rows) in lists {
        object.insert(key.clone(), json!(rows));
    }
    Value::Object(object)
}

pub(crate) fn events(events: &[RecordedEvent]) -> Value {
    Value::Array(
        events
            .iter()
            .map(|event| json!({ "call_type": event.call_type, "outcome": event.outcome }))
            .collect(),
    )
}

pub(crate) fn lpop_entry(entry: &LpopPipelineEntry) -> Value {
    match entry {
        LpopPipelineEntry::Missing => Value::Null,
        LpopPipelineEntry::Items(items) => json!(items),
        LpopPipelineEntry::ItemsBytes(items) => json!(items),
    }
}
