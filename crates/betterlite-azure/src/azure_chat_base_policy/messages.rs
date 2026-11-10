use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::azure::convert::to_azure;
use betterlite_prompts::message_content::Message;
use betterlite_prompts::message_fields::image_hoist::hoist_tool_message_images;
use betterlite_prompts::message_fields::system_runs::system_messages_first;
use betterlite_prompts::message_fields::tool_reference::drop_tool_reference_parts;
use betterlite_prompts::schema::drop_non_python_regex_patterns;
use betterlite_prompts::schema::flatten::{flatten_top_level, tool_with_sanitized_parameters};

use super::error::PolicyError;

#[must_use]
pub fn sanitized_tools_update(optional: &Object) -> Object {
    let Some(Json::List(tools)) = optional.get("tools") else {
        return Object::new();
    };
    let sanitized: Vec<Json> = tools
        .iter()
        .map(|tool| match tool {
            Json::Object(fields) => {
                Json::Object(tool_with_sanitized_parameters(fields.clone(), |schema| {
                    flatten_top_level(&drop_non_python_regex_patterns(schema))
                }))
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                tool.clone()
            }
        })
        .collect();
    Object::from_iter([(String::from("tools"), Json::List(sanitized))])
}

pub fn transform_request(
    model: &str,
    messages: &[Message],
    optional: &Object,
    openai_system_messages_first: bool,
) -> Result<Object, PolicyError> {
    let ordered = if openai_system_messages_first {
        system_messages_first(messages)
    } else {
        messages.to_vec()
    };
    let stripped = drop_tool_reference_parts(&ordered);
    let hoisted = hoist_tool_message_images(&stripped);
    let azure = to_azure(&hoisted).map_err(|source| PolicyError::Messages { source })?;
    let encoded = serde_json::to_value(&azure).map_err(|source| PolicyError::Encode { source })?;
    let message_value =
        serde_json::from_value(encoded).map_err(|source| PolicyError::Encode { source })?;
    let mut body = Object::from_iter([
        (String::from("model"), Json::Text(model.to_owned())),
        (String::from("messages"), message_value),
    ]);
    for (name, value) in optional {
        if name == "tool_choice" && !has_tools_or_functions(optional) {
            continue;
        }
        body.insert(name.clone(), value.clone());
    }
    body.extend(sanitized_tools_update(optional));
    Ok(body)
}

fn has_tools_or_functions(optional: &Object) -> bool {
    ["tools", "functions"]
        .iter()
        .any(|name| optional.get(*name).is_some_and(Json::is_truthy))
}
