use betterlite_openai::api::responses_objects::sdk::text::Reasoning;
use betterlite_prompts::arguments::json::{Json, Object};

// LiteLLM slices reasoning_content to 100 entries and appends "...", which only a short list survives
const REASONING_CONTENT_LIMIT: usize = 100;

// Azure rejects 'status' on reasoning items, so a validated item loses it and a failed one keeps its source shape
pub(super) fn sanitize_item(item: &Object) -> Object {
    validated_item(item).unwrap_or_else(|| {
        tracing::debug!("an Azure reasoning item failed validation; filtering its source fields");
        source_filter(item)
    })
}

fn validated_item(item: &Object) -> Option<Object> {
    let candidate = summary_candidate(item)?;
    let text = serde_json::to_string(&candidate).ok()?;
    let record: Reasoning = serde_json::from_str(&text).ok()?;
    let dumped = serde_json::to_string(&record).ok()?;
    let mut fields: Object = serde_json::from_str(&dumped).ok()?;
    drop_model_nulls(&mut fields);
    fields.shift_remove("status");
    Some(fields)
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
fn drop_model_nulls(fields: &mut Object) {
    fields.retain(|_, value| !matches!(value, Json::Null));
    for key in ["summary", "content"] {
        let Some(Json::List(entries)) = fields.get_mut(key) else {
            continue;
        };
        for entry in entries {
            if let Json::Object(entry_fields) = entry {
                entry_fields.retain(|_, value| !matches!(value, Json::Null));
            }
        }
    }
}
