use std::mem;

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
    let mut fields = match item {
        Record::Dict(fields) => fields,
        Record::Null => return None,
        other @ (Record::Bool(_)
        | Record::Number(_)
        | Record::Text(_)
        | Record::List(_)
        | Record::Object { .. }) => return Some(other),
    };
    let is_reasoning = matches!(
        fields.get("type").and_then(Record::as_text),
        Some("reasoning")
    );
    let has_cipher = fields
        .get("encrypted_content")
        .is_some_and(Record::has_value);
    if !is_reasoning || !has_cipher {
        return Some(Record::Dict(fields));
    }
    let readable = ["summary", "content"]
        .iter()
        .any(|key| fields.get(*key).is_some_and(has_readable_text));
    if !readable {
        return None;
    }
    fields.shift_remove("encrypted_content");
    fields.shift_remove("id");
    Some(Record::Dict(fields))
}

fn has_readable_text(value: &Record) -> bool {
    match value {
        Record::Text(text) => !text.trim().is_empty(),
        Record::List(blocks) => blocks.iter().any(block_has_text),
        Record::Null
        | Record::Bool(_)
        | Record::Number(_)
        | Record::Dict(_)
        | Record::Object { .. } => false,
    }
}

fn block_has_text(block: &Record) -> bool {
    match block {
        Record::Dict(fields) => fields.get("text").is_some_and(Record::has_value),
        Record::Null
        | Record::Bool(_)
        | Record::Number(_)
        | Record::Text(_)
        | Record::List(_)
        | Record::Object { .. } => false,
    }
}
