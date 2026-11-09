mod parse;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::agents::json::AgentJson;

use crate::send_params::error::SendParamsError;
use crate::send_params::v03::{
    DataKindV03, FileKindV03, FileV03, JsonMap, MessageKindV03, MessageSendParamsV03, MessageV03,
    PartV03, PushAuthV03, PushConfigV03, RoleV03, SendConfigV03, TextKindV03,
};

const ROLE_USER: i64 = 1;

#[derive(Default)]
struct RequestV10 {
    message: MessageV10,
    configuration: Option<ConfigV10>,
    metadata: Option<JsonMap>,
}

#[derive(Default)]
struct MessageV10 {
    message_id: String,
    context_id: String,
    task_id: String,
    role: i64,
    parts: Vec<PartV10>,
    metadata: Option<JsonMap>,
    extensions: Vec<String>,
    reference_task_ids: Vec<String>,
}

struct PartV10 {
    content: PartContentV10,
    metadata: Option<JsonMap>,
    media_type: String,
    filename: String,
}

enum PartContentV10 {
    Text(String),
    Raw(Vec<u8>),
    Url(String),
    DataObject(JsonMap),
    DataValue(AgentJson),
}

struct ConfigV10 {
    accepted_output_modes: Vec<String>,
    task_push_notification_config: Option<PushConfigV10>,
    history_length: Option<i64>,
    return_immediately: bool,
}

struct PushConfigV10 {
    url: String,
    id: String,
    token: String,
    authentication: Option<AuthV10>,
}

struct AuthV10 {
    scheme: String,
    credentials: String,
}

pub(crate) fn parse_and_convert(
    params: &AgentJson,
) -> Result<MessageSendParamsV03, SendParamsError> {
    let request = parse::request(params)?;
    Ok(MessageSendParamsV03 {
        message: convert_message(request.message),
        configuration: request.configuration.map(convert_configuration),
        metadata: request.metadata,
    })
}

fn convert_message(message: MessageV10) -> MessageV03 {
    let metadata = message.metadata.filter(|metadata| !metadata.is_empty());
    MessageV03 {
        kind: MessageKindV03::Message,
        message_id: message.message_id,
        role: if message.role == ROLE_USER {
            RoleV03::User
        } else {
            RoleV03::Agent
        },
        parts: message.parts.into_iter().map(convert_part).collect(),
        context_id: (!message.context_id.is_empty()).then_some(message.context_id),
        task_id: (!message.task_id.is_empty()).then_some(message.task_id),
        reference_task_ids: (!message.reference_task_ids.is_empty())
            .then_some(message.reference_task_ids),
        metadata,
        extensions: (!message.extensions.is_empty()).then_some(message.extensions),
    }
}

fn convert_part(part: PartV10) -> PartV03 {
    let media_type = part.media_type;
    let filename = part.filename;
    let metadata = part.metadata;
    match part.content {
        PartContentV10::Text(text) => PartV03::Text {
            kind: TextKindV03::Text,
            text,
            metadata,
        },
        PartContentV10::Raw(raw) => convert_raw_part(raw, media_type, filename, metadata),
        PartContentV10::Url(url) => convert_url_part(url, media_type, filename, metadata),
        PartContentV10::DataObject(data) => PartV03::Data {
            kind: DataKindV03::Data,
            data,
            metadata,
        },
        PartContentV10::DataValue(value) => convert_data_value(value, metadata),
    }
}

fn convert_raw_part(
    raw: Vec<u8>,
    media_type: String,
    filename: String,
    metadata: Option<JsonMap>,
) -> PartV03 {
    PartV03::File {
        kind: FileKindV03::File,
        file: FileV03::Bytes {
            bytes: STANDARD.encode(raw),
            mime_type: (!media_type.is_empty()).then_some(media_type),
            name: (!filename.is_empty()).then_some(filename),
        },
        metadata,
    }
}

fn convert_url_part(
    url: String,
    media_type: String,
    filename: String,
    metadata: Option<JsonMap>,
) -> PartV03 {
    PartV03::File {
        kind: FileKindV03::File,
        file: FileV03::Uri {
            uri: url,
            mime_type: (!media_type.is_empty()).then_some(media_type),
            name: (!filename.is_empty()).then_some(filename),
        },
        metadata,
    }
}

fn convert_data_value(value: AgentJson, metadata: Option<JsonMap>) -> PartV03 {
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

fn convert_configuration(config: ConfigV10) -> SendConfigV03 {
    SendConfigV03 {
        accepted_output_modes: (!config.accepted_output_modes.is_empty())
            .then_some(config.accepted_output_modes),
        push_notification_config: config
            .task_push_notification_config
            .map(convert_push_config),
        history_length: config.history_length,
        blocking: Some(!config.return_immediately),
    }
}

fn convert_push_config(config: PushConfigV10) -> PushConfigV03 {
    PushConfigV03 {
        url: config.url,
        id: (!config.id.is_empty()).then_some(config.id),
        token: (!config.token.is_empty()).then_some(config.token),
        authentication: config.authentication.map(convert_auth),
    }
}

fn convert_auth(auth: AuthV10) -> PushAuthV03 {
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
