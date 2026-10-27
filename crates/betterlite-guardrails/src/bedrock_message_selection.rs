use serde_json::Value;
use std::collections::BTreeMap;

use crate::bedrock_records::{
    ContentItem, MessageFilterResult, MessageSelection, Qualifier, Request, Source, TextContent,
};

mod tests;

pub type BedrockContentItem = ContentItem;
pub type BedrockRequest = Request;
pub type BedrockTextContent = TextContent;
pub type GuardrailMessageFilterResult<Message> = MessageFilterResult<Message>;
pub type ApplyGuardrailMessageSelection<Message> = MessageSelection<Message>;

const TRUSTED_ROLES: [&str; 2] = ["system", "developer"];

pub fn get_content_items_for_message(
    message: &Value,
) -> Option<Vec<crate::bedrock_records::QualifiedTextBlock>> {
    let content = message.get("content")?;
    let mut blocks = Vec::new();
    match content {
        Value::String(text) => blocks.push(crate::bedrock_records::QualifiedTextBlock {
            text: text.clone(),
            qualifier: None,
        }),
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(text) => {
                        blocks.push(crate::bedrock_records::QualifiedTextBlock {
                            text: text.clone(),
                            qualifier: None,
                        })
                    }
                    Value::Object(item) => {
                        let Some(Value::String(text)) = item.get("text") else {
                            continue;
                        };
                        let qualifier = item
                            .get("type")
                            .and_then(Value::as_str)
                            .and_then(qualifier_for_type);
                        blocks.push(crate::bedrock_records::QualifiedTextBlock {
                            text: text.clone(),
                            qualifier,
                        });
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    Some(blocks)
}

fn qualifier_for_type(value: &str) -> Option<Qualifier> {
    match value {
        "grounding_source" => Some(Qualifier::GroundingSource),
        "query" => Some(Qualifier::Query),
        "guard_content" => Some(Qualifier::GuardContent),
        _ => None,
    }
}

pub fn build_content_item(
    block: &crate::bedrock_records::QualifiedTextBlock,
) -> BedrockContentItem {
    let qualifiers = block.qualifier.map(|qualifier| vec![qualifier]);
    BedrockContentItem {
        text: Some(BedrockTextContent {
            text: Some(block.text.clone()),
            qualifiers,
            extra: BTreeMap::new(),
        }),
        extra: BTreeMap::new(),
    }
}

pub fn create_input_content_request(messages: Option<&[Value]>) -> BedrockRequest {
    let content = messages
        .into_iter()
        .flatten()
        .filter_map(get_content_items_for_message)
        .flatten()
        .map(|block| {
            build_content_item(&crate::bedrock_records::QualifiedTextBlock {
                text: block.text,
                qualifier: None,
            })
        })
        .collect();
    BedrockRequest {
        source: Some(Source::Input),
        content: Some(content),
        extra: BTreeMap::new(),
    }
}

pub fn collect_grounding_blocks(
    messages: Option<&[Value]>,
    contextual_fallback: bool,
) -> Vec<crate::bedrock_records::QualifiedTextBlock> {
    let messages = messages.unwrap_or_default();
    let mut grounding = Vec::new();
    for message in messages {
        let trusted = message
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| TRUSTED_ROLES.contains(&role));
        if let Some(blocks) = get_content_items_for_message(message) {
            for block in blocks {
                if block.qualifier == Some(Qualifier::Query)
                    || trusted && block.qualifier == Some(Qualifier::GroundingSource)
                {
                    grounding.push(block);
                }
            }
        }
    }
    if !grounding.is_empty() || !contextual_fallback {
        return grounding;
    }
    derive_grounding_blocks(messages)
}

fn derive_grounding_blocks(messages: &[Value]) -> Vec<crate::bedrock_records::QualifiedTextBlock> {
    let Some(latest) = find_latest_message_index(messages, "user") else {
        return Vec::new();
    };
    let mut blocks = Vec::new();
    for message in messages {
        let trusted = message
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| TRUSTED_ROLES.contains(&role));
        if trusted {
            append_plain_blocks(&mut blocks, message, Qualifier::GroundingSource);
        }
    }
    append_plain_blocks(&mut blocks, &messages[latest], Qualifier::Query);
    blocks
}

fn append_plain_blocks(
    output: &mut Vec<crate::bedrock_records::QualifiedTextBlock>,
    message: &Value,
    qualifier: Qualifier,
) {
    if let Some(blocks) = get_content_items_for_message(message) {
        output.extend(
            blocks
                .into_iter()
                .filter(|block| !block.text.is_empty())
                .map(|block| crate::bedrock_records::QualifiedTextBlock {
                    text: block.text,
                    qualifier: Some(qualifier),
                }),
        );
    }
}

pub fn create_output_content_request(
    response: Option<&Value>,
    messages: Option<&[Value]>,
    contextual_fallback: bool,
) -> BedrockRequest {
    let grounding = collect_grounding_blocks(messages, contextual_fallback);
    let has_grounding = !grounding.is_empty();
    let mut content: Vec<_> = grounding.iter().map(build_content_item).collect();
    if let Some(response) = response {
        content.extend(response_content_items(response, has_grounding));
    }
    BedrockRequest {
        source: Some(Source::Output),
        content: Some(content),
        extra: BTreeMap::new(),
    }
}

fn response_content_items(response: &Value, grounded: bool) -> Vec<BedrockContentItem> {
    let Some(choices) = response.get("choices").and_then(Value::as_array) else {
        return Vec::new();
    };
    choices
        .iter()
        .filter_map(|choice| {
            choice
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
        })
        .filter(|text| !text.is_empty())
        .map(|text| {
            build_content_item(&crate::bedrock_records::QualifiedTextBlock {
                text: text.to_owned(),
                qualifier: grounded.then_some(Qualifier::GuardContent),
            })
        })
        .collect()
}

pub fn find_latest_message_index(messages: &[Value], role: &str) -> Option<usize> {
    messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some(role))
}

pub fn prepare_guardrail_messages_for_role(
    messages: Option<&[Value]>,
    latest_only: bool,
) -> GuardrailMessageFilterResult<Value> {
    let Some(messages) = messages else {
        return GuardrailMessageFilterResult {
            payload_messages: None,
            original_messages: None,
            target_indices: None,
        };
    };
    if !latest_only {
        return GuardrailMessageFilterResult {
            payload_messages: Some(messages.to_vec()),
            original_messages: None,
            target_indices: None,
        };
    }
    let Some(index) = find_latest_message_index(messages, "user") else {
        return GuardrailMessageFilterResult {
            payload_messages: None,
            original_messages: None,
            target_indices: None,
        };
    };
    GuardrailMessageFilterResult {
        payload_messages: Some(vec![messages[index].clone()]),
        original_messages: Some(messages.to_vec()),
        target_indices: Some(vec![index]),
    }
}

pub fn count_message_texts(message: &Value) -> usize {
    get_content_items_for_message(message).map_or(0, |blocks| blocks.len())
}

pub fn locate_message_texts_slice(
    messages: &[Value],
    target_index: usize,
    texts: &[String],
) -> Option<(usize, usize)> {
    let total: usize = messages.iter().map(count_message_texts).sum();
    let target = messages
        .get(target_index)
        .map(count_message_texts)
        .unwrap_or(0);
    if total != texts.len() || target == 0 {
        return None;
    }
    Some((
        messages
            .iter()
            .take(target_index)
            .map(count_message_texts)
            .sum(),
        target,
    ))
}

pub fn select_messages_for_apply_guardrail(
    texts: &[String],
    messages: Option<&[Value]>,
    latest_only: bool,
) -> ApplyGuardrailMessageSelection<Value> {
    if !latest_only {
        return ApplyGuardrailMessageSelection {
            filtered_messages: Some(texts.iter().map(|text| json_message(text)).collect()),
            scanned_slice: None,
            scanned_role_subset: false,
            skip_scan: false,
        };
    }
    let Some(messages) = messages else {
        return legacy_selection(texts);
    };
    let Some(index) = find_latest_message_index(messages, "user") else {
        return ApplyGuardrailMessageSelection {
            filtered_messages: None,
            scanned_slice: None,
            scanned_role_subset: true,
            skip_scan: true,
        };
    };
    if count_message_texts(&messages[index]) == 0 {
        return ApplyGuardrailMessageSelection {
            filtered_messages: None,
            scanned_slice: None,
            scanned_role_subset: true,
            skip_scan: true,
        };
    }
    ApplyGuardrailMessageSelection {
        filtered_messages: Some(vec![messages[index].clone()]),
        scanned_slice: locate_message_texts_slice(messages, index, texts),
        scanned_role_subset: true,
        skip_scan: false,
    }
}

fn json_message(text: &str) -> Value {
    serde_json::json!({"role": "user", "content": text})
}

fn legacy_selection(texts: &[String]) -> ApplyGuardrailMessageSelection<Value> {
    ApplyGuardrailMessageSelection {
        filtered_messages: Some(texts.iter().map(|text| json_message(text)).collect()),
        scanned_slice: None,
        scanned_role_subset: false,
        skip_scan: false,
    }
}

pub fn merge_masked_texts(
    masked: &[String],
    texts: &[String],
    scanned_slice: Option<(usize, usize)>,
    scanned_role_subset: bool,
) -> Vec<String> {
    if masked.is_empty() {
        return texts.to_vec();
    }
    if let Some((offset, length)) = scanned_slice {
        let mut merged = texts.to_vec();
        for (index, text) in masked.iter().take(length).enumerate() {
            if let Some(slot) = merged.get_mut(offset + index) {
                *slot = text.clone();
            }
        }
        return merged;
    }
    if scanned_role_subset {
        tracing::warn!(
            "could not align masked texts with request texts, skipping masking write-back"
        );
        texts.to_vec()
    } else {
        masked.to_vec()
    }
}

pub fn merge_filtered_messages(
    original: Option<&[Value]>,
    updated: &[Value],
    indices: Option<&[usize]>,
) -> Vec<Value> {
    let Some(indices) = indices else {
        return updated.to_vec();
    };
    let mut merged = original.map_or_else(Vec::new, ToOwned::to_owned);
    if merged.is_empty() {
        merged = updated.to_vec();
    }
    for (index, message) in indices.iter().zip(updated) {
        if let Some(slot) = merged.get_mut(*index) {
            *slot = message.clone();
        }
    }
    merged
}
