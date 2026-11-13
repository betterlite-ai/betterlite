use indexmap::IndexMap;

use crate::api::response_id_codecs::container::{build_container_id, decode_container_id};

use super::metadata::AffinityMetadata;
use super::record::{AttributeObject, Record};

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
    match item {
        Record::Dict(fields) => encode_dict_item(fields, custom_llm_provider, model_id),
        Record::Object { object } => encode_object_item(object, custom_llm_provider, model_id),
        Record::Null | Record::Bool(_) | Record::Number(_) | Record::Text(_) | Record::List(_) => {}
    }
}

fn encode_dict_item(
    fields: &mut IndexMap<String, Record>,
    custom_llm_provider: Option<&str>,
    model_id: Option<&str>,
) {
    let encoded = fields
        .get("container_id")
        .and_then(Record::as_text)
        .and_then(|container_id| maybe_encode(container_id, custom_llm_provider, model_id));
    if let Some(wrapped) = encoded {
        fields.insert("container_id".to_owned(), Record::Text(wrapped));
    }
    if let Some(Record::Dict(nested)) = fields.get_mut("code_interpreter_call") {
        let nested_encoded = nested
            .get("container_id")
            .and_then(Record::as_text)
            .and_then(|container_id| maybe_encode(container_id, custom_llm_provider, model_id));
        if let Some(wrapped) = nested_encoded {
            nested.insert("container_id".to_owned(), Record::Text(wrapped));
        }
    }
    if matches!(
        fields.get("type").and_then(Record::as_text),
        Some("message")
    ) && let Some(content) = fields.get_mut("content")
    {
        encode_message_content(content, custom_llm_provider, model_id);
    }
}

fn encode_object_item(
    object: &mut AttributeObject,
    custom_llm_provider: Option<&str>,
    model_id: Option<&str>,
) {
    let encoded = object
        .fields
        .get("container_id")
        .and_then(Record::as_text)
        .and_then(|container_id| maybe_encode(container_id, custom_llm_provider, model_id));
    if let Some(wrapped) = encoded
        && object.set_text("container_id", wrapped).is_err()
    {
        tracing::debug!("Could not set container_id on streaming output item");
    }
    if let Some(nested) = object.fields.get_mut("code_interpreter_call") {
        encode_container_id(nested, custom_llm_provider, model_id);
    }
    if matches!(
        object.fields.get("type").and_then(Record::as_text),
        Some("message")
    ) && let Some(content) = object.fields.get_mut("content")
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
        if let Some(annotations) = part.field_mut("annotations") {
            encode_annotations(annotations, custom_llm_provider, model_id);
        }
    }
}

fn encode_annotations(
    annotations: &mut Record,
    custom_llm_provider: Option<&str>,
    model_id: Option<&str>,
) {
    let Record::List(items) = annotations else {
        return;
    };
    for annotation in items {
        encode_container_id(annotation, custom_llm_provider, model_id);
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

fn collect_from_item<'record>(item: &'record Record, collected: &mut Vec<&'record str>) {
    match item {
        Record::Dict(fields) => collect_from_dict(fields, collected),
        Record::Object { object } => collect_from_object(object, collected),
        Record::Null | Record::Bool(_) | Record::Number(_) | Record::Text(_) | Record::List(_) => {}
    }
}

fn collect_from_dict<'record>(
    fields: &'record IndexMap<String, Record>,
    collected: &mut Vec<&'record str>,
) {
    if let Some(container_id) = fields
        .get("container_id")
        .and_then(Record::as_text)
        .filter(|text| !text.is_empty())
    {
        push_unique(collected, container_id);
    }
    if let Some(Record::Dict(nested)) = fields.get("code_interpreter_call")
        && let Some(container_id) = nested
            .get("container_id")
            .and_then(Record::as_text)
            .filter(|text| !text.is_empty())
    {
        push_unique(collected, container_id);
    }
    if matches!(
        fields.get("type").and_then(Record::as_text),
        Some("message")
    ) && let Some(content) = fields.get("content")
    {
        collect_from_message_content(content, collected);
    }
}

fn collect_from_object<'record>(
    object: &'record AttributeObject,
    collected: &mut Vec<&'record str>,
) {
    if let Some(container_id) = object
        .fields
        .get("container_id")
        .and_then(Record::as_text)
        .filter(|text| !text.is_empty())
    {
        push_unique(collected, container_id);
    }
    if let Some(nested) = object.fields.get("code_interpreter_call") {
        collect_from_item(nested, collected);
    }
    if matches!(
        object.fields.get("type").and_then(Record::as_text),
        Some("message")
    ) && let Some(content) = object.fields.get("content")
    {
        collect_from_message_content(content, collected);
    }
}

fn collect_from_message_content<'record>(
    content: &'record Record,
    collected: &mut Vec<&'record str>,
) {
    let Record::List(parts) = content else {
        return;
    };
    for part in parts {
        if let Some(annotations) = part.field("annotations") {
            collect_from_annotations(annotations, collected);
        }
    }
}

fn collect_from_annotations<'record>(
    annotations: &'record Record,
    collected: &mut Vec<&'record str>,
) {
    let Record::List(items) = annotations else {
        return;
    };
    for annotation in items {
        collect_from_item(annotation, collected);
    }
}

fn push_unique<'record>(collected: &mut Vec<&'record str>, container_id: &'record str) {
    if !collected.contains(&container_id) {
        collected.push(container_id);
    }
}
