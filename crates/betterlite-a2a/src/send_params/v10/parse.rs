use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use betterlite_core::agents::json::AgentJson;
use num_traits::ToPrimitive as _;

use crate::send_params::error::SendParamsError;
use crate::send_params::v03::JsonMap;
use crate::send_params::v10::{
    AuthV10, ConfigV10, MessageV10, PartContentV10, PartV10, PushConfigV10, RequestV10,
};

const ROLE_UNSPECIFIED: i64 = 0;

pub(super) fn request(params: &AgentJson) -> Result<RequestV10, SendParamsError> {
    let AgentJson::Object(_) = params else {
        return Err(SendParamsError::NotAnObject);
    };
    let message = match lookup(params, "message", "message") {
        None => MessageV10::default(),
        Some(value) => message(value)?,
    };
    Ok(RequestV10 {
        message,
        configuration: lookup(params, "configuration", "configuration")
            .map(configuration)
            .transpose()?,
        metadata: map_field(params, "metadata", "metadata", "metadata")?,
    })
}

fn message(value: &AgentJson) -> Result<MessageV10, SendParamsError> {
    let AgentJson::Object(_) = value else {
        return Err(SendParamsError::WrongType {
            field: "message",
            expected: "an object",
        });
    };
    Ok(MessageV10 {
        message_id: string_field(value, "messageId", "message_id", "message.messageId")?
            .unwrap_or_default(),
        context_id: string_field(value, "contextId", "context_id", "message.contextId")?
            .unwrap_or_default(),
        task_id: string_field(value, "taskId", "task_id", "message.taskId")?.unwrap_or_default(),
        role: role_field(value)?,
        parts: match lookup(value, "parts", "parts") {
            None => Vec::new(),
            Some(AgentJson::Array(items)) => items.iter().map(part).collect::<Result<_, _>>()?,
            Some(_) => {
                return Err(SendParamsError::WrongType {
                    field: "message.parts",
                    expected: "a list",
                });
            }
        },
        metadata: map_field(value, "metadata", "metadata", "message.metadata")?,
        extensions: string_list_field(value, "extensions", "extensions", "message.extensions")?,
        reference_task_ids: string_list_field(
            value,
            "referenceTaskIds",
            "reference_task_ids",
            "message.referenceTaskIds",
        )?,
    })
}

fn part(value: &AgentJson) -> Result<PartV10, SendParamsError> {
    let AgentJson::Object(_) = value else {
        return Err(SendParamsError::WrongType {
            field: "message.parts",
            expected: "an object",
        });
    };
    Ok(PartV10 {
        content: content(value)?,
        metadata: map_field(value, "metadata", "metadata", "message.parts.metadata")?,
        media_type: string_field(value, "mediaType", "media_type", "message.parts.mediaType")?
            .unwrap_or_default(),
        filename: string_field(value, "filename", "filename", "message.parts.filename")?
            .unwrap_or_default(),
    })
}

fn content(value: &AgentJson) -> Result<PartContentV10, SendParamsError> {
    let text = lookup(value, "text", "text").filter(|field| !field.is_null());
    let raw = lookup(value, "raw", "raw").filter(|field| !field.is_null());
    let url = lookup(value, "url", "url").filter(|field| !field.is_null());
    // a null data value is a Value message holding null, not an absent field
    let data = lookup(value, "data", "data");
    let present = [text.is_some(), raw.is_some(), url.is_some(), data.is_some()]
        .into_iter()
        .filter(|present| *present)
        .count();
    match present {
        0 => return Err(SendParamsError::MissingPartContent),
        1 => {}
        _ => return Err(SendParamsError::MultiplePartContents),
    }
    if let Some(field) = text {
        return text_content(field);
    }
    if let Some(field) = raw {
        return raw_content(field);
    }
    if let Some(field) = url {
        return url_content(field);
    }
    data.map_or(Err(SendParamsError::MissingPartContent), |field| {
        data_content(field)
    })
}

fn text_content(value: &AgentJson) -> Result<PartContentV10, SendParamsError> {
    match value {
        AgentJson::String(text) => Ok(PartContentV10::Text(text.clone())),
        AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::Array(_)
        | AgentJson::Object(_) => Err(SendParamsError::WrongType {
            field: "message.parts.text",
            expected: "a string",
        }),
    }
}

fn raw_content(value: &AgentJson) -> Result<PartContentV10, SendParamsError> {
    match value {
        AgentJson::String(text) => decode_base64(text).map_or_else(
            || {
                Err(SendParamsError::InvalidValue {
                    field: "message.parts.raw",
                    expected: "base64 string",
                })
            },
            |raw| Ok(PartContentV10::Raw(raw)),
        ),
        AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::Array(_)
        | AgentJson::Object(_) => Err(SendParamsError::WrongType {
            field: "message.parts.raw",
            expected: "a base64 string",
        }),
    }
}

fn url_content(value: &AgentJson) -> Result<PartContentV10, SendParamsError> {
    match value {
        AgentJson::String(url) => Ok(PartContentV10::Url(url.clone())),
        AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::Array(_)
        | AgentJson::Object(_) => Err(SendParamsError::WrongType {
            field: "message.parts.url",
            expected: "a string",
        }),
    }
}

fn data_content(value: &AgentJson) -> Result<PartContentV10, SendParamsError> {
    match value {
        AgentJson::Object(entries) => {
            let mut fields = JsonMap::new();
            for (key, entry) in entries {
                fields.insert(key.clone(), struct_value(entry, "message.parts.data")?);
            }
            Ok(PartContentV10::DataObject(fields))
        }
        other @ (AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::String(_)
        | AgentJson::Array(_)) => Ok(PartContentV10::DataValue(struct_value(
            other,
            "message.parts.data",
        )?)),
    }
}

fn role_field(value: &AgentJson) -> Result<i64, SendParamsError> {
    match lookup(value, "role", "role") {
        None | Some(AgentJson::Null) => Ok(ROLE_UNSPECIFIED),
        Some(AgentJson::String(name)) => Ok(role_name(name)),
        Some(AgentJson::Number(number)) => integer_number(number, "message.role"),
        Some(_) => Err(SendParamsError::WrongType {
            field: "message.role",
            expected: "a role name or number",
        }),
    }
}

// protobuf ignores a role name it does not know and reads a numeric name as the enum number
fn role_name(name: &str) -> i64 {
    match name {
        "ROLE_USER" => 1,
        "ROLE_AGENT" => 2,
        "ROLE_UNSPECIFIED" => ROLE_UNSPECIFIED,
        other => other.parse::<i64>().unwrap_or(ROLE_UNSPECIFIED),
    }
}

fn configuration(value: &AgentJson) -> Result<ConfigV10, SendParamsError> {
    let AgentJson::Object(_) = value else {
        return Err(SendParamsError::WrongType {
            field: "configuration",
            expected: "an object",
        });
    };
    Ok(ConfigV10 {
        accepted_output_modes: string_list_field(
            value,
            "acceptedOutputModes",
            "accepted_output_modes",
            "configuration.acceptedOutputModes",
        )?,
        task_push_notification_config: lookup(
            value,
            "taskPushNotificationConfig",
            "task_push_notification_config",
        )
        .map(push_config)
        .transpose()?,
        history_length: integer_field(
            value,
            "historyLength",
            "history_length",
            "configuration.historyLength",
        )?,
        return_immediately: bool_field(
            value,
            "returnImmediately",
            "return_immediately",
            "configuration.returnImmediately",
        )?
        .unwrap_or(false),
    })
}

fn push_config(value: &AgentJson) -> Result<PushConfigV10, SendParamsError> {
    let AgentJson::Object(_) = value else {
        return Err(SendParamsError::WrongType {
            field: "configuration.taskPushNotificationConfig",
            expected: "an object",
        });
    };
    Ok(PushConfigV10 {
        url: string_field(
            value,
            "url",
            "url",
            "configuration.taskPushNotificationConfig.url",
        )?
        .unwrap_or_default(),
        id: string_field(
            value,
            "id",
            "id",
            "configuration.taskPushNotificationConfig.id",
        )?
        .unwrap_or_default(),
        token: string_field(
            value,
            "token",
            "token",
            "configuration.taskPushNotificationConfig.token",
        )?
        .unwrap_or_default(),
        authentication: lookup(value, "authentication", "authentication")
            .map(authentication)
            .transpose()?,
    })
}

fn authentication(value: &AgentJson) -> Result<AuthV10, SendParamsError> {
    let AgentJson::Object(_) = value else {
        return Err(SendParamsError::WrongType {
            field: "configuration.taskPushNotificationConfig.authentication",
            expected: "an object",
        });
    };
    Ok(AuthV10 {
        scheme: string_field(
            value,
            "scheme",
            "scheme",
            "configuration.taskPushNotificationConfig.authentication.scheme",
        )?
        .unwrap_or_default(),
        credentials: string_field(
            value,
            "credentials",
            "credentials",
            "configuration.taskPushNotificationConfig.authentication.credentials",
        )?
        .unwrap_or_default(),
    })
}

fn lookup<'value>(value: &'value AgentJson, json: &str, proto: &str) -> Option<&'value AgentJson> {
    let AgentJson::Object(entries) = value else {
        return None;
    };
    entries.get(json).or_else(|| entries.get(proto))
}

fn string_field(
    value: &AgentJson,
    json: &str,
    proto: &str,
    field: &'static str,
) -> Result<Option<String>, SendParamsError> {
    match lookup(value, json, proto) {
        None | Some(AgentJson::Null) => Ok(None),
        Some(AgentJson::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(SendParamsError::WrongType {
            field,
            expected: "a string",
        }),
    }
}

fn string_list_field(
    value: &AgentJson,
    json: &str,
    proto: &str,
    field: &'static str,
) -> Result<Vec<String>, SendParamsError> {
    let Some(found) = lookup(value, json, proto).filter(|present| !present.is_null()) else {
        return Ok(Vec::new());
    };
    let AgentJson::Array(items) = found else {
        return Err(SendParamsError::WrongType {
            field,
            expected: "a list of strings",
        });
    };
    items
        .iter()
        .map(|item| match item {
            AgentJson::String(text) => Ok(text.clone()),
            AgentJson::Null
            | AgentJson::Bool(_)
            | AgentJson::Number(_)
            | AgentJson::Array(_)
            | AgentJson::Object(_) => Err(SendParamsError::WrongType {
                field,
                expected: "a list of strings",
            }),
        })
        .collect()
}

fn bool_field(
    value: &AgentJson,
    json: &str,
    proto: &str,
    field: &'static str,
) -> Result<Option<bool>, SendParamsError> {
    match lookup(value, json, proto) {
        None | Some(AgentJson::Null) => Ok(None),
        Some(AgentJson::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(SendParamsError::WrongType {
            field,
            expected: "a boolean",
        }),
    }
}

fn integer_field(
    value: &AgentJson,
    json: &str,
    proto: &str,
    field: &'static str,
) -> Result<Option<i64>, SendParamsError> {
    match lookup(value, json, proto) {
        None | Some(AgentJson::Null) => Ok(None),
        Some(AgentJson::Number(number)) => integer_number(number, field).map(Some),
        // protobuf JSON reads an int64 from a decimal string as well as a number
        Some(AgentJson::String(text)) => {
            text.parse::<i64>()
                .map(Some)
                .map_err(|_error| SendParamsError::InvalidValue {
                    field,
                    expected: "64-bit integer",
                })
        }
        Some(_) => Err(SendParamsError::WrongType {
            field,
            expected: "an integer",
        }),
    }
}

fn integer_number(
    number: &serde_json::Number,
    field: &'static str,
) -> Result<i64, SendParamsError> {
    if let Some(value) = number.as_i64() {
        return Ok(value);
    }
    if let Some(value) = number.as_u64() {
        return i64::try_from(value).map_err(|_error| SendParamsError::InvalidValue {
            field,
            expected: "64-bit integer",
        });
    }
    let Some(float) = number.as_f64() else {
        return Err(SendParamsError::InvalidValue {
            field,
            expected: "64-bit integer",
        });
    };
    if float.fract().to_bits() != 0.0_f64.to_bits() {
        return Err(SendParamsError::InvalidValue {
            field,
            expected: "integer",
        });
    }
    float.to_i64().ok_or(SendParamsError::InvalidValue {
        field,
        expected: "64-bit integer",
    })
}

fn map_field(
    value: &AgentJson,
    json: &str,
    proto: &str,
    field: &'static str,
) -> Result<Option<JsonMap>, SendParamsError> {
    let Some(found) = lookup(value, json, proto).filter(|present| !present.is_null()) else {
        return Ok(None);
    };
    let AgentJson::Object(entries) = found else {
        return Err(SendParamsError::WrongType {
            field,
            expected: "an object",
        });
    };
    let mut mapped = JsonMap::new();
    for (key, entry) in entries {
        mapped.insert(key.clone(), struct_value(entry, field)?);
    }
    Ok(Some(mapped))
}

// protobuf stores every Struct and Value number as a double, so an integer comes back widened
fn struct_value(value: &AgentJson, field: &'static str) -> Result<AgentJson, SendParamsError> {
    match value {
        AgentJson::Number(number) => number
            .as_f64()
            .filter(|float| float.is_finite())
            .map_or_else(
                || {
                    Err(SendParamsError::InvalidValue {
                        field,
                        expected: "double",
                    })
                },
                |float| Ok(AgentJson::from(float)),
            ),
        AgentJson::Array(items) => items
            .iter()
            .map(|item| struct_value(item, field))
            .collect::<Result<Vec<_>, _>>()
            .map(AgentJson::Array),
        AgentJson::Object(entries) => {
            let mut normalized = entries.clone();
            for (key, entry) in entries {
                normalized.insert(key.clone(), struct_value(entry, field)?);
            }
            Ok(AgentJson::Object(normalized))
        }
        AgentJson::Null | AgentJson::Bool(_) | AgentJson::String(_) => Ok(value.clone()),
    }
}

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut cleaned = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' => cleaned.push(character),
            '-' => cleaned.push('+'),
            '_' => cleaned.push('/'),
            _ => {}
        }
    }
    STANDARD_NO_PAD.decode(cleaned.as_bytes()).ok()
}
