use betterlite_models::decisions::Json;

use super::chat_tools::{ChatFunction, ChatFunctionTool, ChatTool, ToolExtras, with_object_type};
use super::error::NamespaceToolsError;
use super::values::{Fields, has_value, member_list, text_field, tool_type};
use crate::api::responses_codex_tools::custom_tools::custom_to_function_tool;

const DESCRIPTION_SEPARATOR: &str = "\n\n";

struct Namespace<'tool> {
    name: &'tool str,
    description: &'tool str,
}

// B907 validates allowed_callers inside its custom tool conversion, so a probe custom tool borrows that check
fn validated_allowed_callers(value: Option<&Json>) -> Result<Option<Json>, NamespaceToolsError> {
    let probe = Fields::from([
        ("type".to_owned(), Json::Text("custom".to_owned())),
        (
            "allowed_callers".to_owned(),
            value.cloned().unwrap_or(Json::Null),
        ),
    ]);
    let callers = custom_to_function_tool(&probe)?.and_then(|tool| tool.allowed_callers);
    Ok(callers.map(|names| Json::List(names.into_iter().map(Json::Text).collect())))
}

fn member_description(namespace: &Namespace<'_>, raw: &str, nested: bool) -> String {
    if nested && !namespace.description.is_empty() && !raw.is_empty() {
        return format!("{}{DESCRIPTION_SEPARATOR}{raw}", namespace.description);
    }
    if nested && !namespace.description.is_empty() {
        return namespace.description.to_owned();
    }
    raw.to_owned()
}

// the flat fallback runs this on the namespace tool itself, so its name stays unprefixed
fn member_chat_tool(
    namespace: &Namespace<'_>,
    member: &Fields,
    nested: bool,
) -> Result<Option<ChatTool>, NamespaceToolsError> {
    let kind = tool_type(member);
    if nested && kind != "function" && kind != "custom" {
        return Ok(None);
    }
    let description = member_description(namespace, text_field(member, "description")?, nested);
    if nested && kind == "custom" {
        let mut custom = member.clone();
        custom.insert("description".to_owned(), Json::Text(description));
        return Ok(custom_to_function_tool(&custom)?.map(ChatTool::Custom));
    }
    let parameters = with_object_type(match member.get("parameters") {
        Some(Json::Object(fields)) => fields.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Fields::new()
        }
    });
    let tool_name = text_field(member, "name")?;
    Ok(Some(ChatTool::Function(Box::new(ChatFunctionTool {
        kind: "function",
        function: ChatFunction {
            name: if nested {
                format!("{}__{tool_name}", namespace.name)
            } else {
                tool_name.to_owned()
            },
            description,
            parameters,
            strict: Json::Bool(member.get("strict").is_some_and(has_value)),
        },
        extras: ToolExtras {
            cache_control: None,
            defer_loading: None,
            allowed_callers: validated_allowed_callers(member.get("allowed_callers"))?,
            input_examples: None,
            eager_input_streaming: None,
        },
    }))))
}

pub fn namespace_chat_tools(tool: &Fields) -> Result<Vec<ChatTool>, NamespaceToolsError> {
    let namespace = Namespace {
        name: text_field(tool, "name")?,
        description: text_field(tool, "description")?,
    };
    let Some(members) = member_list(tool) else {
        return Ok(member_chat_tool(&namespace, tool, false)?
            .into_iter()
            .collect());
    };
    let mut chat_tools = Vec::new();
    for member in members {
        if let Json::Object(fields) = member
            && let Some(chat_tool) = member_chat_tool(&namespace, fields, true)?
        {
            chat_tools.push(chat_tool);
        }
    }
    Ok(chat_tools)
}
