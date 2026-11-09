use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::agents::json::AgentJson;

use crate::send_params::error::SendParamsError;
use crate::send_params::v03::{
    DataKindV03, FileKindV03, FileV03, JsonMap, MessageKindV03, MessageSendParamsV03, MessageV03,
    PartV03, PushAuthV03, PushConfigV03, RoleV03, SendConfigV03, TextKindV03,
};

use super::values::{decode_base64, integer_from_number, normalize_map, normalize_value};
use super::{
    AuthV10, ConfigV10, DataV10, IntegerV10, MessageV10, PartV10, PushConfigV10, RequestV10,
    RoleV10,
};

enum Content {
    Text(String),
    Raw(Vec<u8>),
    Url(String),
    DataObject(JsonMap),
    DataValue(AgentJson),
}

pub(super) fn request(request: RequestV10) -> Result<MessageSendParamsV03, SendParamsError> {
    Ok(MessageSendParamsV03 {
        message: message(request.message.unwrap_or_default())?,
        configuration: request.configuration.map(configuration).transpose()?,
        metadata: request
            .metadata
            .map(|metadata| normalize_map(metadata, "metadata"))
            .transpose()?,
    })
}

fn message(message: MessageV10) -> Result<MessageV03, SendParamsError> {
    Ok(MessageV03 {
        kind: MessageKindV03::Message,
        message_id: message.message_id,
        role: role(message.role),
        parts: message.parts.iter().map(part).collect::<Result<_, _>>()?,
        context_id: (!message.context_id.is_empty()).then_some(message.context_id),
        task_id: (!message.task_id.is_empty()).then_some(message.task_id),
        reference_task_ids: (!message.reference_task_ids.is_empty())
            .then_some(message.reference_task_ids),
        metadata: message
            .metadata
            .map(|metadata| normalize_map(metadata, "message.metadata"))
            .transpose()?
            .filter(|metadata| !metadata.is_empty()),
        extensions: (!message.extensions.is_empty()).then_some(message.extensions),
    })
}

// ROLE_USER is the only name that reads as user, and an unknown name is ignored as ParseDict ignores it
fn role(value: Option<RoleV10>) -> RoleV03 {
    match value {
        Some(RoleV10::Number(1)) => RoleV03::User,
        Some(RoleV10::Name(name)) if name == "ROLE_USER" => RoleV03::User,
        Some(RoleV10::Name(name)) if matches!(name.parse::<i64>(), Ok(1)) => RoleV03::User,
        Some(RoleV10::Number(_) | RoleV10::Name(_)) | None => RoleV03::Agent,
    }
}

fn part(part: &PartV10) -> Result<PartV03, SendParamsError> {
    let metadata = part
        .metadata
        .clone()
        .map(|metadata| normalize_map(metadata, "message.parts.metadata"))
        .transpose()?;
    let media_type = (!part.media_type.is_empty()).then(|| part.media_type.clone());
    let filename = (!part.filename.is_empty()).then(|| part.filename.clone());
    match content(part)? {
        Content::Text(text) => Ok(PartV03::Text {
            kind: TextKindV03::Text,
            text,
            metadata,
        }),
        Content::Raw(raw) => Ok(PartV03::File {
            kind: FileKindV03::File,
            file: FileV03::Bytes {
                bytes: STANDARD.encode(raw),
                mime_type: media_type,
                name: filename,
            },
            metadata,
        }),
        Content::Url(uri) => Ok(PartV03::File {
            kind: FileKindV03::File,
            file: FileV03::Uri {
                uri,
                mime_type: media_type,
                name: filename,
            },
            metadata,
        }),
        Content::DataObject(data) => Ok(PartV03::Data {
            kind: DataKindV03::Data,
            data,
            metadata,
        }),
        Content::DataValue(value) => Ok(data_value_part(value, metadata)),
    }
}

fn content(part: &PartV10) -> Result<Content, SendParamsError> {
    let null_data = matches!(part.data, DataV10::Value(AgentJson::Null));
    // ParseDict does not count a null oneof member as set, so only non-null keys conflict
    let present = [
        part.text.is_some(),
        part.raw.is_some(),
        part.url.is_some(),
        !matches!(part.data, DataV10::Absent | DataV10::Value(AgentJson::Null)),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    match present {
        0 if null_data => return Ok(Content::DataValue(AgentJson::Null)),
        0 => return Err(SendParamsError::MissingPartContent),
        1 => {}
        _ => return Err(SendParamsError::MultiplePartContents),
    }
    if let Some(text) = &part.text {
        return Ok(Content::Text(text.clone()));
    }
    if let Some(raw) = &part.raw {
        let decoded = decode_base64(raw).ok_or(SendParamsError::InvalidValue {
            field: "message.parts.raw",
            expected: "base64 string",
        })?;
        return Ok(Content::Raw(decoded));
    }
    if let Some(url) = &part.url {
        return Ok(Content::Url(url.clone()));
    }
    data_content(&part.data)
}

fn data_content(data: &DataV10) -> Result<Content, SendParamsError> {
    match data {
        DataV10::Absent => Err(SendParamsError::MissingPartContent),
        DataV10::Value(AgentJson::Object(entries)) => {
            let mut fields = JsonMap::new();
            for (key, entry) in entries {
                fields.insert(key.clone(), normalize_value(entry, "message.parts.data")?);
            }
            Ok(Content::DataObject(fields))
        }
        DataV10::Value(value) => Ok(Content::DataValue(normalize_value(
            value,
            "message.parts.data",
        )?)),
    }
}

fn data_value_part(value: AgentJson, metadata: Option<JsonMap>) -> PartV03 {
    let mut data = JsonMap::new();
    data.insert("value".to_owned(), value);
    let mut flagged = metadata.unwrap_or_default();
    flagged.insert("data_part_compat".to_owned(), AgentJson::Bool(true));
    PartV03::Data {
        kind: DataKindV03::Data,
        data,
        metadata: Some(flagged),
    }
}

fn configuration(config: ConfigV10) -> Result<SendConfigV03, SendParamsError> {
    Ok(SendConfigV03 {
        accepted_output_modes: (!config.accepted_output_modes.is_empty())
            .then_some(config.accepted_output_modes),
        push_notification_config: config.task_push_notification_config.map(push_config),
        history_length: history_length(config.history_length)?,
        blocking: Some(!config.return_immediately),
    })
}

fn history_length(value: Option<IntegerV10>) -> Result<Option<i64>, SendParamsError> {
    match value {
        None => Ok(None),
        Some(IntegerV10::Number(number)) => {
            integer_from_number(&number, "configuration.historyLength")
                .map(|length| Some(i64::from(length)))
        }
        Some(IntegerV10::Text(text)) => text
            .parse::<i32>()
            .map(|length| Some(i64::from(length)))
            .map_err(|_error| SendParamsError::InvalidValue {
                field: "configuration.historyLength",
                expected: "32-bit integer",
            }),
    }
}

fn push_config(config: PushConfigV10) -> PushConfigV03 {
    PushConfigV03 {
        url: config.url,
        id: (!config.id.is_empty()).then_some(config.id),
        token: (!config.token.is_empty()).then_some(config.token),
        authentication: config.authentication.map(authentication),
    }
}

fn authentication(auth: AuthV10) -> PushAuthV03 {
    let schemes = if auth.scheme.is_empty() {
        Vec::new()
    } else {
        vec![auth.scheme]
    };
    PushAuthV03 {
        schemes,
        credentials: (!auth.credentials.is_empty()).then_some(auth.credentials),
    }
}
