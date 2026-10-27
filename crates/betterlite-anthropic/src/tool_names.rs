use std::collections::{BTreeMap, BTreeSet};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
pub const REVERSE_MAP_KEY: &str = "_anthropic_tool_name_map";
const MAX_TOOL_NAME_LENGTH: usize = 128;

pub type ToolName = String;

pub type AdditionalFields = BTreeMap<String, Box<RawValue>>;

#[derive(Debug, Clone, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct ToolNameMaps {
    pub forward: BTreeMap<String, String>,
    pub reverse: BTreeMap<String, String>,
}

// a JSON object split into the keys this module reads (head) and every other key kept raw (rest)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(
    try_from = "Box<RawValue>",
    bound(deserialize = "Head: DeserializeOwned + Serialize")
)]
pub struct Record<Head> {
    #[serde(flatten)]
    pub head: Head,
    #[serde(flatten)]
    pub rest: AdditionalFields,
}

// a tool entry or a tool_choice
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct NamedHead {
    #[serde(rename = "type", default, deserialize_with = "lenient")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MessageHead {
    #[serde(default, deserialize_with = "lenient")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Record<CallHead>>>,
    #[serde(default, deserialize_with = "lenient")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_call: Option<Record<FunctionHead>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CallHead {
    #[serde(default, deserialize_with = "lenient")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<Record<FunctionHead>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FunctionHead {
    #[serde(default, deserialize_with = "lenient")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RequestParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Record<NamedHead>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Record<NamedHead>>,
}

// flatten buffers values and RawValue cannot come back out of that buffer, so the object is read twice
impl<Head> TryFrom<Box<RawValue>> for Record<Head>
where
    Head: DeserializeOwned + Serialize,
{
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let head = serde_json::from_str::<Head>(raw.get())?;
        let modeled = serde_json::from_str::<AdditionalFields>(&serde_json::to_string(&head)?)?;
        let mut rest = serde_json::from_str::<AdditionalFields>(raw.get())?;
        rest.retain(|key, _| !modeled.contains_key(key));
        Ok(Self { head, rest })
    }
}

// LiteLLM skips a value of the wrong JSON type, so it reads as absent and stays in rest unchanged
fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: DeserializeOwned,
{
    let raw = Box::<RawValue>::deserialize(deserializer)?;
    Ok(serde_json::from_str(raw.get()).ok())
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
        && choice.head.kind.as_deref() == Some("tool")
    {
        rewrite_name(&mut choice.head.name, &maps.forward);
    }
    maps
}

#[must_use]
pub fn rewrite_tool_names_in_messages(
    messages: &[Record<MessageHead>],
    forward: &BTreeMap<String, String>,
) -> Vec<Record<MessageHead>> {
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
    value.chars().take(length).collect()
}

const fn text_name(value: &str) -> Option<&str> {
    if value.is_empty() {
        return None;
    }
    Some(value)
}

fn custom_name(tool: &Record<NamedHead>) -> Option<&str> {
    if tool.head.kind.as_deref() == Some("custom") {
        return tool.head.name.as_deref().and_then(text_name);
    }
    None
}

fn rewrite_tool(tool: &mut Record<NamedHead>, forward: &BTreeMap<String, String>) {
    if tool.head.kind.as_deref() != Some("custom") {
        return;
    }
    rewrite_name(&mut tool.head.name, forward);
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

fn rewrite_message(
    message: &Record<MessageHead>,
    forward: &BTreeMap<String, String>,
) -> Record<MessageHead> {
    let mut rewritten = message.clone();
    if let Some(calls) = rewritten.head.tool_calls.as_mut() {
        for call in calls {
            rewrite_call(call, forward);
        }
    }
    if let Some(call) = rewritten.head.function_call.as_mut() {
        rewrite_name(&mut call.head.name, forward);
    }
    rewritten
}

fn rewrite_call(call: &mut Record<CallHead>, forward: &BTreeMap<String, String>) {
    if let Some(function) = call.head.function.as_mut() {
        rewrite_name(&mut function.head.name, forward);
    }
}

mod tests;
