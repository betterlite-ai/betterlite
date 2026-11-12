use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::message_content::ContentPart;
use betterlite_prompts::reasoning_fields::replay_groups::{
    ReasoningItemKind, reasoning_items_from_blocks,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::error::HistoryError;
use super::history::HistoryMessage;
use crate::api::responses_input_content::is_truthy;

#[derive(Debug, Default)]
pub struct ReasoningIds {
    assigned: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReasoningSummaryBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReasoningRecord {
    #[serde(rename = "type")]
    pub kind: ReasoningItemKind,
    pub id: String,
    pub encrypted_content: Option<String>,
    pub summary: Vec<ReasoningSummaryBlock>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReasoningOutputItem {
    Dict { value: Map<String, Value> },
    Response { value: TypedReasoningItem },
    ResponseOther,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TypedReasoningItem {
    pub id: String,
    #[serde(default)]
    pub encrypted_content: Option<String>,
    #[serde(default)]
    pub summary: Vec<TypedSummaryBlock>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TypedSummaryBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

pub fn reasoning_input_items(
    message: &HistoryMessage,
    ids: &mut ReasoningIds,
) -> Result<Vec<Value>, HistoryError> {
    let stored = message.reasoning_items.as_deref().unwrap_or_default();
    if !stored.is_empty() {
        return stored
            .iter()
            .map(|item| stored_reasoning_item(item, ids))
            .collect();
    }
    let replay_blocks = message.thinking_blocks.as_deref().unwrap_or_default();
    let blocks = replay_blocks
        .iter()
        .map(thinking_block)
        .collect::<Vec<ContentPart>>();
    let mut items = Vec::new();
    for item in reasoning_items_from_blocks(&blocks) {
        let value =
            serde_json::to_value(&item).map_err(|source| HistoryError::Serialize { source })?;
        items.push(value);
    }
    Ok(items)
}

// The Responses API rejects an id it did not mint; an item without one gets an opaque id that the same item reuses
fn stored_reasoning_item(
    item: &Map<String, Value>,
    ids: &mut ReasoningIds,
) -> Result<Value, HistoryError> {
    let id = match item.get("id") {
        Some(value) if is_truthy(Some(value)) => value.clone(),
        _ => Value::String(ids.identifier(item)?),
    };
    let summary = item
        .get("summary")
        .filter(|value| is_truthy(Some(value)))
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let mut output = Map::new();
    output.insert("type".to_owned(), Value::String("reasoning".to_owned()));
    output.insert("id".to_owned(), id);
    output.insert("summary".to_owned(), summary);
    if let Some(encrypted_content) = item
        .get("encrypted_content")
        .filter(|value| is_truthy(Some(value)))
    {
        output.insert("encrypted_content".to_owned(), encrypted_content.clone());
    }
    Ok(Value::Object(output))
}

impl ReasoningIds {
    fn identifier(&mut self, item: &Map<String, Value>) -> Result<String, HistoryError> {
        let key =
            serde_json::to_string(item).map_err(|source| HistoryError::Serialize { source })?;
        if let Some(existing) = self.assigned.get(&key) {
            return Ok(existing.clone());
        }
        let identifier = format!("rs_{}", self.assigned.len());
        self.assigned.insert(key, identifier.clone());
        Ok(identifier)
    }
}

fn thinking_block(block: &Map<String, Value>) -> ContentPart {
    let kind = block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut fields = Object::new();
    for (key, value) in block {
        if key != "type" {
            fields.insert(key.clone(), prompt_json(value));
        }
    }
    ContentPart { kind, fields }
}

fn prompt_json(value: &Value) -> PromptJson {
    match value {
        Value::Null => PromptJson::Null,
        Value::Bool(flag) => PromptJson::Bool(*flag),
        Value::Number(number) => PromptJson::Number(number.clone()),
        Value::String(text) => PromptJson::Text(text.clone()),
        Value::Array(items) => PromptJson::List(items.iter().map(prompt_json).collect()),
        Value::Object(fields) => PromptJson::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), prompt_json(item)))
                .collect(),
        ),
    }
}

pub fn reasoning_records(
    items: &[ReasoningOutputItem],
) -> Result<Vec<ReasoningRecord>, HistoryError> {
    let mut records = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match item {
            ReasoningOutputItem::Response { value } => records.push(response_record(value)),
            ReasoningOutputItem::Dict { value }
                if value.get("type").and_then(Value::as_str) == Some("reasoning") =>
            {
                records.push(dict_record(value, index)?);
            }
            ReasoningOutputItem::Dict { .. } | ReasoningOutputItem::ResponseOther => {}
        }
    }
    Ok(records)
}

fn response_record(item: &TypedReasoningItem) -> ReasoningRecord {
    ReasoningRecord {
        kind: ReasoningItemKind::Reasoning,
        id: item.id.clone(),
        encrypted_content: item.encrypted_content.clone(),
        summary: item
            .summary
            .iter()
            .map(|block| ReasoningSummaryBlock {
                kind: block.kind.clone(),
                text: block.text.clone(),
            })
            .collect(),
    }
}

fn dict_record(value: &Map<String, Value>, index: usize) -> Result<ReasoningRecord, HistoryError> {
    let id = match value.get("id") {
        None => String::new(),
        Some(Value::String(id)) => id.clone(),
        Some(_) => return Err(HistoryError::ReasoningField { index, field: "id" }),
    };
    let encrypted_content = match value.get("encrypted_content") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(_) => {
            return Err(HistoryError::ReasoningField {
                index,
                field: "encrypted_content",
            });
        }
    };
    let summary = dict_summary(value.get("summary"), index)?;
    Ok(ReasoningRecord {
        kind: ReasoningItemKind::Reasoning,
        id,
        encrypted_content,
        summary,
    })
}

fn dict_summary(
    summary: Option<&Value>,
    index: usize,
) -> Result<Vec<ReasoningSummaryBlock>, HistoryError> {
    let entries = match summary {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(HistoryError::SummaryShape { index }),
    };
    let mut blocks = Vec::new();
    for (position, entry) in entries.iter().enumerate() {
        let Some(fields) = entry.as_object() else {
            return Err(HistoryError::SummaryShape { index });
        };
        let kind = match fields.get("type") {
            None => "summary_text".to_owned(),
            Some(Value::String(kind)) => kind.clone(),
            Some(_) => {
                return Err(HistoryError::SummaryBlockField {
                    index,
                    block: position,
                    field: "type",
                });
            }
        };
        let text = match fields.get("text") {
            None => String::new(),
            Some(Value::String(text)) => text.clone(),
            Some(_) => {
                return Err(HistoryError::SummaryBlockField {
                    index,
                    block: position,
                    field: "text",
                });
            }
        };
        blocks.push(ReasoningSummaryBlock { kind, text });
    }
    Ok(blocks)
}

#[must_use]
pub fn chat_reasoning_items(records: &[ReasoningRecord]) -> Option<Vec<ReasoningRecord>> {
    if records.is_empty() {
        return None;
    }
    Some(records.to_vec())
}
