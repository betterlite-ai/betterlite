use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::schema_strings::schema_string_messages;
use crate::content_inspection::build_inspection_messages;

// the field each extra source came from, so a redaction slice maps back to it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtraField {
    Input,
    Instructions,
    Prompt,
    SchemaStrings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtraSource {
    pub field: ExtraField,
    pub messages: Vec<GuardrailValue>,
}

pub(super) fn synthetic_message(role: &str, content: &str) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        (
            "content".to_owned(),
            GuardrailValue::Text(content.to_owned()),
        ),
        ("role".to_owned(), GuardrailValue::Text(role.to_owned())),
    ]))
}

// LiteLLM's _inspection_messages: chat messages stay 1:1 by index, every other
// forwarded field is appended after them as synthetic messages
#[must_use]
pub fn inspection_messages(data: &BTreeMap<String, GuardrailValue>) -> Vec<GuardrailValue> {
    let mut flattened = Vec::new();
    if let Some(GuardrailValue::Array(messages)) = data.get("messages") {
        for message in messages {
            flattened.push(flatten_message(message));
        }
    }
    for source in extra_inspection_sources(data) {
        flattened.extend(source.messages);
    }
    flattened
}

fn flatten_message(message: &GuardrailValue) -> GuardrailValue {
    let GuardrailValue::Object(object) = message else {
        return message.clone();
    };
    let Some(GuardrailValue::Array(_)) = object.get("content") else {
        return message.clone();
    };
    let single = BTreeMap::from([(
        "messages".to_owned(),
        GuardrailValue::Array(vec![message.clone()]),
    )]);
    let content = build_inspection_messages(&single)
        .first()
        .map(|built| built.content.clone())
        .unwrap_or_default();
    let mut flattened = object.clone();
    flattened.insert("content".to_owned(), GuardrailValue::Text(content));
    GuardrailValue::Object(flattened)
}

#[must_use]
pub fn extra_inspection_sources(data: &BTreeMap<String, GuardrailValue>) -> Vec<ExtraSource> {
    let mut sources = Vec::new();
    let mut input_only = BTreeMap::new();
    if let Some(input) = data.get("input") {
        input_only.insert("input".to_owned(), input.clone());
    }
    let input_messages = build_inspection_messages(&input_only)
        .into_iter()
        .map(|message| synthetic_message(&message.role, &message.content))
        .collect::<Vec<_>>();
    if !input_messages.is_empty() {
        sources.push(ExtraSource {
            field: ExtraField::Input,
            messages: input_messages,
        });
    }
    if let Some(GuardrailValue::Text(instructions)) = data.get("instructions")
        && !instructions.is_empty()
    {
        sources.push(ExtraSource {
            field: ExtraField::Instructions,
            messages: vec![synthetic_message("system", instructions)],
        });
    }
    let prompt = prompt_inspection_messages(data.get("prompt"));
    if !prompt.is_empty() {
        sources.push(ExtraSource {
            field: ExtraField::Prompt,
            messages: prompt,
        });
    }
    let schema = schema_string_messages(data);
    if !schema.is_empty() {
        sources.push(ExtraSource {
            field: ExtraField::SchemaStrings,
            messages: schema,
        });
    }
    sources
}

fn prompt_inspection_messages(prompt: Option<&GuardrailValue>) -> Vec<GuardrailValue> {
    match prompt {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => {
            vec![synthetic_message("user", text)]
        }
        Some(GuardrailValue::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                GuardrailValue::Text(text) if !text.is_empty() => {
                    Some(synthetic_message("user", text))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}
