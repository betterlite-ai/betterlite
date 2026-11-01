use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::redact::{TextUpdate, set_tool_response_text};
use crate::cisco_redaction::set_serializable_field;

#[must_use]
pub fn apply_redaction_mcp_input(
    request_data: &mut Fields,
    sanitized_text: Option<&str>,
    sanitized_mcp_arguments: Option<&Fields>,
) -> bool {
    if let Some(arguments) = sanitized_mcp_arguments {
        set_serializable_field(request_data, "mcp_arguments", arguments);
        set_serializable_field(request_data, "modified_arguments", arguments);
        if let Json::Object(mut params) = Json::field(request_data, "params") {
            set_serializable_field(&mut params, "arguments", arguments);
            set_serializable_field(request_data, "params", &params);
        }
        if matches!(Json::field(request_data, "arguments"), Json::Object(_)) {
            set_serializable_field(request_data, "arguments", arguments);
        }
        return true;
    }
    let Some(text) = sanitized_text.filter(|text| !text.is_empty()) else {
        return false;
    };
    let mut applied = rewrite_argument(request_data, ArgumentPath::Field("mcp_arguments"), text);
    applied |= rewrite_argument(request_data, ArgumentPath::Field("arguments"), text);
    applied |= rewrite_argument(request_data, ArgumentPath::ParamsField("arguments"), text);
    applied
}

#[must_use]
pub fn apply_redaction_mcp_output(
    response: Option<&Json>,
    sanitized_text: Option<&str>,
) -> Option<TextUpdate> {
    let present_response = response?;
    let text = sanitized_text.filter(|text| !text.is_empty())?;
    Some(set_tool_response_text(present_response, text))
}

fn rewrite_single_argument(arguments: &mut Fields, text: &str) -> bool {
    let Some(target_key) = single_string_key(arguments) else {
        return false;
    };
    set_serializable_field(arguments, &target_key, text);
    true
}

fn rewrite_scoped_argument<Read, Write>(
    request_data: &mut Fields,
    text: &str,
    read: Read,
    write: Write,
) -> bool
where
    Read: FnOnce(&Fields) -> Option<Fields>,
    Write: FnOnce(&mut Fields, &Fields),
{
    let Some(mut arguments) = read(request_data) else {
        return false;
    };
    if !rewrite_single_argument(&mut arguments, text) {
        return false;
    }
    write(request_data, &arguments);
    set_serializable_field(request_data, "modified_arguments", &arguments);
    true
}

#[derive(Clone, Copy)]
enum ArgumentPath<'path> {
    Field(&'path str),
    ParamsField(&'path str),
}

fn rewrite_argument(request_data: &mut Fields, path: ArgumentPath<'_>, text: &str) -> bool {
    match path {
        ArgumentPath::Field(key) => rewrite_scoped_argument(
            request_data,
            text,
            |data| match Json::field(data, key) {
                Json::Object(arguments) => Some(arguments),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    None
                }
            },
            |data, arguments| set_serializable_field(data, key, arguments),
        ),
        ArgumentPath::ParamsField(key) => rewrite_scoped_argument(
            request_data,
            text,
            |data| {
                let Json::Object(params) = Json::field(data, "params") else {
                    return None;
                };
                match Json::field(&params, key) {
                    Json::Object(arguments) => Some(arguments),
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::Text(_)
                    | Json::List(_) => None,
                }
            },
            |data, arguments| {
                let Json::Object(mut params) = Json::field(data, "params") else {
                    return;
                };
                set_serializable_field(&mut params, key, arguments);
                set_serializable_field(data, "params", &params);
            },
        ),
    }
}

// LiteLLM only rewrites the argument object when exactly one value is a string
fn single_string_key(arguments: &Fields) -> Option<String> {
    let mut string_keys = arguments
        .iter()
        .filter(|(_, value)| matches!(Json::parse(value.get()), Json::Text(_)))
        .map(|(key, _)| key.clone());
    let key = string_keys.next()?;
    if string_keys.next().is_some() {
        return None;
    }
    Some(key)
}
