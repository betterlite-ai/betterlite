use betterlite_openai::api::responses_objects::sdk::text::Reasoning;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::{Map, Value};

// LiteLLM slices reasoning_content to 100 entries and appends "...", which only a short list survives
const REASONING_CONTENT_LIMIT: usize = 100;

// Azure rejects 'status' on reasoning items, so a validated item loses it and a failed one keeps its source shape
pub(super) fn sanitize_item(item: &Object) -> Object {
    match validated_item(item) {
        Some(fields) => fields,
        None => {
            tracing::debug!(
                "an Azure reasoning item failed validation; filtering its source fields"
            );
            source_filter(item)
        }
    }
}

fn validated_item(item: &Object) -> Option<Object> {
    let candidate = summary_candidate(item)?;
    let value = serde_json::to_value(&candidate).ok()?;
    let record: Reasoning = serde_json::from_value(value).ok()?;
    let mut serialized = serde_json::to_value(record).ok()?;
    let fields = serialized.as_object_mut()?;
    drop_model_nulls(fields);
    fields.remove("status");
    let dumped: Json = serde_json::from_value(serialized).ok()?;
    dumped.as_object().cloned()
}

fn summary_candidate(item: &Object) -> Option<Object> {
    if item.contains_key("summary") {
        return Some(item.clone());
    }
    let Some(Json::List(summary)) = item.get("reasoning_content") else {
        return None;
    };
    if summary.len() > REASONING_CONTENT_LIMIT {
        return None;
    }
    let mut candidate = item.clone();
    candidate.insert("summary".to_owned(), Json::List(summary.clone()));
    Some(candidate)
}

// The item LiteLLM keeps when validation fails, dropping these keys only when their value is null
fn source_filter(item: &Object) -> Object {
    let mut fields = Object::new();
    for (key, value) in item {
        if matches!(value, Json::Null)
            && matches!(key.as_str(), "status" | "content" | "encrypted_content")
        {
            continue;
        }
        fields.insert(key.clone(), value.clone());
    }
    fields
}

// model_dump(exclude_none=True) drops the null fields of the item and of each summary and content entry
fn drop_model_nulls(fields: &mut Map<String, Value>) {
    fields.retain(|_, value| !value.is_null());
    for key in ["summary", "content"] {
        let Some(Value::Array(entries)) = fields.get_mut(key) else {
            continue;
        };
        for entry in entries {
            if let Value::Object(entry_fields) = entry {
                entry_fields.retain(|_, value| !value.is_null());
            }
        }
    }
}
