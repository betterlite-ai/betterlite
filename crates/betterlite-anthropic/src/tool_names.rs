use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
pub const REVERSE_MAP_KEY: &str = "_anthropic_tool_name_map";
const MAX_TOOL_NAME_LENGTH: usize = 128;

pub type ToolName = String;

#[expect(
    clippy::disallowed_types,
    reason = "serde_json::Value preserves provider fields not modeled by this adapter"
)]
pub type AdditionalFields = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Clone, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct ToolNameMaps {
    pub forward: BTreeMap<String, String>,
    pub reverse: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ToolRecord {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub additional_fields: AdditionalFields,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ToolChoiceRecord {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub choice_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    pub additional_fields: AdditionalFields,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RequestParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolRecord>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoiceRecord>,
}

pub type MessageEntry = MessageRecord;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MessageRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_call: Option<NamedCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ContentBlock>>,
}

pub type NamedCall = FunctionRecord;

pub type ToolCallEntry = ToolCallRecord;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ToolCallRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<FunctionRecord>,
    #[serde(flatten)]
    pub additional_fields: AdditionalFields,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FunctionRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ContentBlock {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub block_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    pub additional_fields: AdditionalFields,
}

#[must_use]
pub fn basic_sanitize_tool_name(name: &ToolName) -> ToolName {
    if name.is_empty() {
        return name.clone();
    }
    sanitize_text(name)
}

#[must_use]
pub fn build_tool_name_maps(original_names: &[ToolName]) -> ToolNameMaps {
    let mut used = BTreeSet::new();
    for entry in original_names {
        let Some(original) = text_name(entry) else {
            continue;
        };
        let candidate = sanitize_text(original);
        if candidate == original {
            used.insert(candidate);
        }
    }
    let mut forward = BTreeMap::new();
    for entry in original_names {
        let Some(original) = text_name(entry) else {
            continue;
        };
        let candidate = sanitize_text(original);
        if candidate == original || forward.contains_key(original) {
            continue;
        }
        let mut unique = candidate.clone();
        let mut suffix = 1_u32;
        while used.contains(&unique) {
            suffix = suffix.saturating_add(1);
            let suffix_text = format!("_{suffix}");
            let head_length = MAX_TOOL_NAME_LENGTH.saturating_sub(suffix_text.chars().count());
            unique = format!("{}{}", take_chars(&candidate, head_length), suffix_text);
        }
        used.insert(unique.clone());
        forward.insert(original.to_owned(), unique);
    }
    let reverse = forward
        .iter()
        .map(|(original, sanitized)| (sanitized.clone(), original.clone()))
        .collect();
    ToolNameMaps { forward, reverse }
}

pub fn sanitize_tool_names_in_request(params: &mut RequestParams) -> ToolNameMaps {
    let Some(tools_ref) = params.tools.as_ref() else {
        return ToolNameMaps::default();
    };
    if tools_ref.is_empty() {
        return ToolNameMaps::default();
    }
    let names = tools_ref
        .iter()
        .filter_map(custom_name)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let maps = build_tool_name_maps(&names);
    if maps.forward.is_empty() {
        return maps;
    }
    if let Some(tools_mut) = params.tools.as_mut() {
        for tool in tools_mut {
            rewrite_tool(tool, &maps.forward);
        }
    }
    if let Some(choice) = params.tool_choice.as_mut()
        && choice.choice_type.as_deref() == Some("tool")
    {
        rewrite_name(&mut choice.name, &maps.forward);
    }
    maps
}

#[must_use]
pub fn rewrite_tool_names_in_messages(
    messages: &[MessageEntry],
    forward: &BTreeMap<String, String>,
) -> Vec<MessageEntry> {
    if forward.is_empty() {
        return messages.to_vec();
    }
    messages
        .iter()
        .map(|message| rewrite_message(message, forward))
        .collect()
}

fn sanitize_text(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    take_chars(&sanitized, MAX_TOOL_NAME_LENGTH)
}

fn take_chars(value: &str, length: usize) -> String {
    let mut result = String::new();
    for character in value.chars().take(length) {
        result.push(character);
    }
    result
}

const fn text_name(value: &str) -> Option<&str> {
    if value.is_empty() {
        return None;
    }
    Some(value)
}

fn custom_name(tool: &ToolRecord) -> Option<&str> {
    if tool.tool_type.as_deref() == Some("custom") {
        return tool.name.as_deref().and_then(text_name);
    }
    None
}

fn rewrite_tool(tool: &mut ToolRecord, forward: &BTreeMap<String, String>) {
    if tool.tool_type.as_deref() != Some("custom") {
        return;
    }
    rewrite_name(&mut tool.name, forward);
}

fn rewrite_name(name: &mut Option<String>, forward: &BTreeMap<String, String>) {
    let Some(original) = name.as_ref() else {
        return;
    };
    let Some(mapped) = forward.get(original) else {
        return;
    };
    *name = Some(mapped.clone());
}

fn rewrite_message(message: &MessageEntry, forward: &BTreeMap<String, String>) -> MessageEntry {
    let mut rewritten = message.clone();
    if let Some(calls) = rewritten.tool_calls.as_mut() {
        for call in calls {
            rewrite_call(call, forward);
        }
    }
    if let Some(call) = rewritten.function_call.as_mut() {
        rewrite_name(&mut call.name, forward);
    }
    rewritten
}

fn rewrite_call(call: &mut ToolCallEntry, forward: &BTreeMap<String, String>) {
    if let Some(function) = call.function.as_mut() {
        rewrite_name(&mut function.name, forward);
    }
}

mod tests;
