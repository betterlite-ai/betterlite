use betterlite_core::api::responses_codex_tools::custom_tools::custom_to_function_tool;
use betterlite_core::api::responses_tool_namespaces::error::NamespaceToolsError;
use betterlite_core::api::responses_tool_namespaces::values::{Fields, text_field, tool_type};
use betterlite_models::decisions::Json;

use super::as_tool::as_tool;
use super::error::ToolMergeError;
use super::group::{CHAT_MEMBER_TYPES, DESCRIPTION_SEPARATOR, FlattenedGroup};

mod tests;

const CHAT_TOP_LEVEL_KEYS: [&str; 2] = ["type", "function"];

// B907 keeps its grammar suffix private, so a probe custom tool borrows the suffix from its description.
fn grammar_suffix(format_value: Option<&Json>) -> Result<String, NamespaceToolsError> {
    let mut probe = Fields::from([("type".to_owned(), Json::Text("custom".to_owned()))]);
    if let Some(definition) = format_value {
        probe.insert("format".to_owned(), definition.clone());
    }
    Ok(custom_to_function_tool(&probe)?.map_or_else(String::new, |tool| tool.function.description))
}

// The flattened member carried the namespace description and the grammar, so a changed description sheds both.
fn member_description(key: &str, value: &Json, prefix: &str, suffix: &str) -> Json {
    if key == "description"
        && let Json::Text(text) = value
    {
        return Json::Text(text.replacen(prefix, "", 1).replacen(suffix, "", 1));
    }
    value.clone()
}

// Only what the guardrail changed from the flattened tool is written onto the original member; a null equals an absent key.
pub fn rebuilt_member(
    member: &Fields,
    flattened: &Fields,
    guardrailed: &Fields,
    namespace_description: &str,
) -> Result<Fields, ToolMergeError> {
    let prefix = if namespace_description.is_empty() {
        String::new()
    } else {
        format!("{namespace_description}{DESCRIPTION_SEPARATOR}")
    };
    let suffix = if tool_type(member) == "custom" {
        grammar_suffix(member.get("format"))?
    } else {
        String::new()
    };
    let mut rebuilt = member.clone();
    for (key, value) in guardrailed {
        if !CHAT_TOP_LEVEL_KEYS.contains(&key.as_str())
            && flattened.get(key).unwrap_or(&Json::Null) != value
        {
            rebuilt.insert(key.clone(), value.clone());
        }
    }
    let flattened_function = flattened.get("function").and_then(as_tool);
    let function = guardrailed.get("function").and_then(as_tool);
    for (key, value) in function.into_iter().flatten() {
        let before = flattened_function.and_then(|fields| fields.get(key));
        if before.unwrap_or(&Json::Null) != value {
            rebuilt.insert(
                key.clone(),
                member_description(key, value, &prefix, &suffix),
            );
        }
    }
    Ok(rebuilt)
}

// A member whose key the guardrail no longer returns is None; one it returned unchanged stays as it was.
fn rebuilt_flattened_members(
    chat_members: &[&Fields],
    group: &FlattenedGroup<'_>,
    namespace_description: &str,
) -> Result<Vec<Option<Fields>>, ToolMergeError> {
    let mut rebuilt = Vec::with_capacity(chat_members.len());
    for ((member, flattened), key) in chat_members.iter().zip(group.tools).zip(group.keys) {
        let Some(guardrailed) = group.guardrailed.get(key) else {
            rebuilt.push(None);
            continue;
        };
        if **guardrailed == *flattened {
            rebuilt.push(Some((*member).clone()));
        } else {
            rebuilt.push(Some(rebuilt_member(
                member,
                flattened,
                guardrailed,
                namespace_description,
            )?));
        }
    }
    Ok(rebuilt)
}

// The namespace keeps its shape and its non-chat members; a namespace left without members is removed.
pub fn rebuilt_namespace(
    original: &Fields,
    members: &[&Fields],
    group: &FlattenedGroup<'_>,
) -> Result<Vec<Fields>, ToolMergeError> {
    let namespace_description = text_field(original, "description")?;
    let chat_members: Vec<&Fields> = members
        .iter()
        .copied()
        .filter(|member| CHAT_MEMBER_TYPES.contains(&tool_type(member)))
        .collect();
    let mut rebuilt_chat =
        rebuilt_flattened_members(&chat_members, group, namespace_description)?.into_iter();
    let mut rebuilt_members = Vec::with_capacity(members.len());
    for member in members {
        if CHAT_MEMBER_TYPES.contains(&tool_type(member)) {
            rebuilt_members.extend(rebuilt_chat.next().flatten());
        } else {
            rebuilt_members.push((*member).clone());
        }
    }
    if rebuilt_members.is_empty() {
        return Ok(Vec::new());
    }
    let mut namespace = original.clone();
    namespace.insert(
        "tools".to_owned(),
        Json::List(rebuilt_members.into_iter().map(Json::Object).collect()),
    );
    Ok(vec![namespace])
}
