use std::mem;

use betterlite_models::decisions::Json;

use super::record::Record;

pub fn strip_encrypted_reasoning(
    input: &mut Record,
    should_strip: Option<&dyn Fn(&Record) -> bool>,
) {
    let Record::List(items) = input else {
        return;
    };
    let current = mem::take(items);
    let mut kept = Vec::with_capacity(current.len());
    for item in current {
        let selected = should_strip
            .is_none_or(|predicate| matches!(&item, Record::Dict(_)) && predicate(&item));
        let stripped = if selected {
            without_encrypted_reasoning(item)
        } else {
            Some(item)
        };
        if let Some(record) = stripped {
            kept.push(record);
        }
    }
    *items = kept;
}

// LiteLLM keeps a reasoning item only when its summary or content carries readable text, and the item loses its cipher and id
fn without_encrypted_reasoning(item: Record) -> Option<Record> {
    let mut dict = match item {
        Record::Dict(dict) => dict,
        Record::Scalar(Json::Null) => return None,
        other @ (Record::Scalar(_) | Record::List(_) | Record::Object { .. }) => {
            return Some(other);
        }
    };
    let is_reasoning = matches!(
        dict.values.get("type").and_then(Record::as_text),
        Some("reasoning")
    );
    let has_cipher = dict
        .values
        .get("encrypted_content")
        .is_some_and(Record::has_value);
    if !is_reasoning || !has_cipher {
        return Some(Record::Dict(dict));
    }
    let readable = ["summary", "content"]
        .iter()
        .any(|key| dict.values.get(*key).is_some_and(has_readable_text));
    if !readable {
        return None;
    }
    dict.values.shift_remove("encrypted_content");
    dict.values.shift_remove("id");
    Some(Record::Dict(dict))
}

fn has_readable_text(value: &Record) -> bool {
    match value {
        Record::Scalar(Json::Text(text)) => !text.trim().is_empty(),
        Record::List(blocks) => blocks.iter().any(|block| match block {
            Record::Dict(dict) => dict.values.get("text").is_some_and(Record::has_value),
            Record::Object { .. } | Record::List(_) | Record::Scalar(_) => false,
        }),
        Record::Dict(_) | Record::Object { .. } | Record::Scalar(_) => false,
    }
}
