use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
pub const REVERSE_MAP_KEY: &str = "_anthropic_tool_name_map";
const MAX_TOOL_NAME_LENGTH: usize = 128;

pub type ToolName = String;

pub mod wire;

use wire::{Call, Calls, Function, Message, Name, RequestParams, Tool, Tools};

#[derive(Debug, Clone, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct ToolNameMaps {
    pub forward: BTreeMap<String, String>,
    pub reverse: BTreeMap<String, String>,
}

#[must_use]
pub fn basic_sanitize_tool_name(name: &Name) -> Name {
    match name {
        Name::Known(text) => Name::Known(sanitize_text(text)),
        Name::Other(raw) => Name::Other(raw.clone()),
    }
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
        let mut suffix = 1_usize;
        while used.contains(&unique) {
            suffix = suffix.saturating_add(1);
            let suffix_text = format!("_{suffix}");
            let head_length = MAX_TOOL_NAME_LENGTH.saturating_sub(suffix_text.chars().count());
            unique = format!(
                "{}{}",
                candidate.chars().take(head_length).collect::<String>(),
                suffix_text
            );
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
    let Some(Tools::Known(tools_ref)) = params.head.tools.as_ref() else {
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
    if let Some(Tools::Known(tools_mut)) = params.head.tools.as_mut() {
        for tool in tools_mut {
            rewrite_tool(tool, &maps.forward);
        }
    }
    if let Some(Tool::Known(choice)) = params.head.tool_choice.as_mut()
        && matches!(&choice.head.kind, Some(Name::Known(kind)) if kind == "tool")
    {
        rewrite_name(&mut choice.head.name, &maps.forward);
    }
    maps
}

#[must_use]
pub fn rewrite_tool_names_in_messages(
    messages: &[Message],
    forward: &BTreeMap<String, String>,
) -> Vec<Message> {
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
    sanitized.chars().take(MAX_TOOL_NAME_LENGTH).collect()
}

const fn text_name(value: &str) -> Option<&str> {
    if value.is_empty() {
        return None;
    }
    Some(value)
}

fn custom_name(tool: &Tool) -> Option<&str> {
    if let Tool::Known(record) = tool
        && matches!(&record.head.kind, Some(Name::Known(kind)) if kind == "custom")
        && let Some(Name::Known(name)) = &record.head.name
    {
        return text_name(name);
    }
    None
}

fn rewrite_tool(tool: &mut Tool, forward: &BTreeMap<String, String>) {
    if let Tool::Known(record) = tool
        && matches!(&record.head.kind, Some(Name::Known(kind)) if kind == "custom")
    {
        rewrite_name(&mut record.head.name, forward);
    }
}

fn rewrite_name(name: &mut Option<Name>, forward: &BTreeMap<String, String>) {
    if let Some(Name::Known(original)) = name
        && let Some(mapped) = forward.get(original)
    {
        *original = mapped.clone();
    }
}

fn rewrite_message(message: &Message, forward: &BTreeMap<String, String>) -> Message {
    let mut rewritten = message.clone();
    let Message::Known(record) = &mut rewritten else {
        return rewritten;
    };
    if let Some(Calls::Known(calls)) = record.head.tool_calls.as_mut() {
        for call in calls {
            if let Call::Known(entry) = call
                && let Some(Function::Known(function)) = entry.head.function.as_mut()
            {
                rewrite_name(&mut function.head.name, forward);
            }
        }
    }
    if let Some(Function::Known(function)) = record.head.function_call.as_mut() {
        rewrite_name(&mut function.head.name, forward);
    }
    rewritten
}

mod tests;
