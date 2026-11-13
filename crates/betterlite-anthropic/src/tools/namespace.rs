use betterlite_prompts::arguments::json::{Json, Object};

use super::helper::push;
use super::wire::ToolDefinition;
use super::{ToolMapping, ToolsError, map_tool_helper};

pub(super) fn expand_namespace(
    fields: &Object,
    mapping: &mut ToolMapping,
) -> Result<(), ToolsError> {
    let members = match fields.get("tools") {
        None | Some(Json::Null) => return Ok(()),
        Some(Json::List(members)) => members,
        Some(_) => {
            return Err(ToolsError::WrongType {
                field: "tools",
                expected: "a list",
            });
        }
    };
    for member in members {
        add_member(member, mapping)?;
    }
    Ok(())
}

fn add_member(member: &Json, mapping: &mut ToolMapping) -> Result<(), ToolsError> {
    let fields = member.as_object().ok_or(ToolsError::WrongType {
        field: "tools",
        expected: "a list of objects",
    })?;
    if fields.contains_key("input_schema") {
        mapping.tools.push(ToolDefinition::Native(fields.clone()));
        return Ok(());
    }
    if !fields.contains_key("function") && fields.contains_key("name") {
        push(mapping, map_tool_helper(&wrap_flat(fields)?)?);
        return Ok(());
    }
    if fields.contains_key("function") {
        push(mapping, map_tool_helper(member)?);
    }
    Ok(())
}

fn wrap_flat(fields: &Object) -> Result<Json, ToolsError> {
    let kind = match fields.get("type") {
        None | Some(Json::Null) => "function",
        Some(Json::Text(text)) => text.as_str(),
        Some(_) => {
            return Err(ToolsError::WrongType {
                field: "type",
                expected: "a string",
            });
        }
    };
    let function: Object = fields
        .iter()
        .filter(|(key, _)| key.as_str() != "type")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Ok(Json::Object(Object::from_iter([
        ("type".to_owned(), Json::Text(kind.to_owned())),
        ("function".to_owned(), Json::Object(function)),
    ])))
}
