use crate::api::response_id_codecs::container::{build_container_id, decode_container_id};

use super::metadata::AffinityMetadata;
use super::record::Record;

pub fn encode_containers(
    response: &mut Record,
    custom_llm_provider: Option<&str>,
    metadata: &AffinityMetadata,
) {
    let model_id = metadata
        .model_info
        .as_ref()
        .and_then(|info| info.id.as_deref());
    let Some(Record::List(items)) = response.field_mut("output") else {
        return;
    };
    for item in items {
        encode_container_id(item, custom_llm_provider, model_id);
    }
}

pub fn encode_container_id(
    item: &mut Record,
    custom_llm_provider: Option<&str>,
    model_id: Option<&str>,
) {
    let encoded = item
        .field("container_id")
        .and_then(Record::as_text)
        .and_then(|container_id| maybe_encode(container_id, custom_llm_provider, model_id));
    if let Some(wrapped) = encoded
        && item.set_text("container_id", wrapped).is_err()
    {
        tracing::debug!("Could not set container_id on streaming output item");
    }
    // a mapping's nested call is visited only when it is a mapping; an object's attribute is visited whatever it holds
    let parent_is_mapping = matches!(item, Record::Dict(_));
    if let Some(nested) = item.field_mut("code_interpreter_call")
        && (!parent_is_mapping || matches!(nested, Record::Dict(_)))
    {
        encode_container_id(nested, custom_llm_provider, model_id);
    }
    if matches!(
        item.field("type").and_then(Record::as_text),
        Some("message")
    ) && let Some(content) = item.field_mut("content")
    {
        encode_message_content(content, custom_llm_provider, model_id);
    }
}

fn encode_message_content(
    content: &mut Record,
    custom_llm_provider: Option<&str>,
    model_id: Option<&str>,
) {
    let Record::List(parts) = content else {
        return;
    };
    for part in parts {
        if let Some(Record::List(annotations)) = part.field_mut("annotations") {
            for annotation in annotations {
                encode_container_id(annotation, custom_llm_provider, model_id);
            }
        }
    }
}

// a managed container id whose decoded provider is known already routes, so only raw ids and provider-less envelopes are wrapped
fn maybe_encode(
    container_id: &str,
    custom_llm_provider: Option<&str>,
    model_id: Option<&str>,
) -> Option<String> {
    if matches!(
        decode_container_id(container_id).custom_llm_provider,
        Some(Some(_))
    ) {
        return None;
    }
    Some(build_container_id(
        custom_llm_provider,
        model_id,
        container_id,
    ))
}

#[must_use]
pub fn collect_container_ids(response: Option<&Record>) -> Vec<String> {
    let Some(found) = response else {
        return Vec::new();
    };
    let Some(Record::List(items)) = found.field("output") else {
        return Vec::new();
    };
    let mut collected: Vec<&str> = Vec::new();
    for item in items {
        collect_from_item(item, &mut collected);
    }
    collected.into_iter().map(str::to_owned).collect()
}

// an explicit stack keeps the walk off the call stack; the result carries no promised order
fn collect_from_item<'record>(item: &'record Record, collected: &mut Vec<&'record str>) {
    let mut pending = vec![item];
    while let Some(current) = pending.pop() {
        if let Some(container_id) = current
            .field("container_id")
            .and_then(Record::as_text)
            .filter(|text| !text.is_empty())
        {
            push_unique(collected, container_id);
        }
        if let Some(nested) = current.field("code_interpreter_call")
            && (!matches!(current, Record::Dict(_)) || matches!(nested, Record::Dict(_)))
        {
            pending.push(nested);
        }
        if matches!(
            current.field("type").and_then(Record::as_text),
            Some("message")
        ) && let Some(content) = current.field("content")
        {
            pending_annotations(content, &mut pending);
        }
    }
}

fn pending_annotations<'record>(content: &'record Record, pending: &mut Vec<&'record Record>) {
    let Record::List(parts) = content else {
        return;
    };
    for part in parts {
        if let Some(Record::List(annotations)) = part.field("annotations") {
            pending.extend(annotations);
        }
    }
}

fn push_unique<'record>(collected: &mut Vec<&'record str>, container_id: &'record str) {
    if !collected.contains(&container_id) {
        collected.push(container_id);
    }
}
