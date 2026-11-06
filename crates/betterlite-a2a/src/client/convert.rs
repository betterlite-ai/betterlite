use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;
use thiserror::Error;

use crate::client::convert::parts::{parts_to_compat, to_core_part};
use crate::client::records::SendMessageResponse;
use crate::client::records::compat::{
    AuthenticationInfo, Message, MessageSendConfiguration, PushNotificationConfig, Role,
    SendMessageRequest,
};
use crate::client::records::core::{
    CoreMessage, CoreRole, CoreSendMessageRequest, CoreStreamEvent,
};

pub mod parts;
pub mod response;

#[must_use]
pub fn named(entries: Vec<(&str, AgentJson)>) -> Vec<(String, AgentJson)> {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}
pub fn text_field(value: &AgentJson, name: &str) -> Option<AgentJson> {
    value
        .get(name)
        .and_then(AgentJson::as_str)
        .filter(|text| !text.is_empty())
        .map(|text| AgentJson::String(text.to_owned()))
}
pub fn copy_fields(source: &AgentJson, target: &mut Vec<(String, AgentJson)>, names: &[&str]) {
    for name in names {
        if let Some(value) = source.get(*name) {
            target.push(((*name).to_owned(), value.clone()));
        }
    }
}
pub fn push_optional(target: &mut Vec<(String, AgentJson)>, name: &str, value: Option<AgentJson>) {
    if let Some(present) = value {
        target.push((name.to_owned(), present));
    }
}

#[derive(Debug, Error)]
pub enum ConvertError {
    #[error("the A2A part kind is not text, data or file")]
    UnknownPart { kind: String },
    #[error("the A2A core part carries no text, data, raw bytes or URL")]
    UnknownContent,
    #[error("the A2A data part marked data_part_compat carries no value")]
    MissingDataValue,
    #[error("the A2A file part carries neither bytes nor a URI")]
    FileContent,
    #[error("the A2A file part bytes are not base64")]
    FileBytes(#[source] base64::DecodeError),
}

pub fn to_core_send_message_request(
    request: SendMessageRequest,
) -> Result<CoreSendMessageRequest, ConvertError> {
    let Message {
        role,
        parts,
        common,
    } = request.params.message;
    let message = CoreMessage {
        role: match role {
            Role::User => CoreRole::User,
            Role::Agent => CoreRole::Agent,
        },
        parts: parts
            .iter()
            .map(to_core_part)
            .collect::<Result<Vec<_>, _>>()?,
        common,
    };
    Ok(CoreSendMessageRequest {
        message,
        configuration: request.params.configuration.map(to_core_configuration),
        metadata: request.params.metadata,
    })
}

pub fn to_compat_stream_response(
    event: CoreStreamEvent,
    request_id: JsonRpcId,
) -> Result<SendMessageResponse, ConvertError> {
    let result = match event {
        CoreStreamEvent::Message(message) => to_compat_message(&message)?,
        CoreStreamEvent::Task(task) => response::task_to_compat(&task)?,
        CoreStreamEvent::StatusUpdate(update) => response::status_update_to_compat(&update)?,
        CoreStreamEvent::ArtifactUpdate(update) => response::artifact_update_to_compat(&update)?,
    };
    Ok(SendMessageResponse {
        jsonrpc: "2.0".to_owned(),
        id: request_id,
        result,
    })
}
pub fn to_compat_message(message: &AgentJson) -> Result<AgentJson, ConvertError> {
    let mut fields = named(vec![
        ("kind", AgentJson::String("message".to_owned())),
        (
            "messageId",
            message.get("messageId").cloned().unwrap_or_default(),
        ),
        (
            "role",
            compat_role(message.get("role").and_then(AgentJson::as_str)),
        ),
        ("parts", parts_to_compat(message.get("parts"))?),
    ]);
    copy_fields(
        message,
        &mut fields,
        &[
            "contextId",
            "taskId",
            "referenceTaskIds",
            "metadata",
            "extensions",
        ],
    );
    Ok(AgentJson::Object(fields.into_iter().collect()))
}
fn compat_role(role: Option<&str>) -> AgentJson {
    let compat = if role == Some("ROLE_USER") {
        "user"
    } else {
        "agent"
    };
    AgentJson::String(compat.to_owned())
}

#[must_use]
pub fn to_core_configuration(configuration: MessageSendConfiguration) -> AgentJson {
    let mut fields = named(Vec::new());
    let accepted = configuration
        .accepted_output_modes
        .filter(|modes| !modes.is_empty());
    if let Some(modes) = accepted {
        let values = modes.into_iter().map(AgentJson::String).collect();
        fields.push(("acceptedOutputModes".to_owned(), AgentJson::Array(values)));
    }
    if let Some(push) = configuration.push_notification_config {
        fields.push(("taskPushNotificationConfig".to_owned(), push_config(push)));
    }
    if let Some(length) = configuration.history_length {
        let number = AgentJson::Number(serde_json::Number::from(length));
        fields.push(("historyLength".to_owned(), number));
    }
    let immediately = configuration.blocking.is_some_and(|blocking| !blocking);
    fields.push(("returnImmediately".to_owned(), AgentJson::Bool(immediately)));
    AgentJson::Object(fields.into_iter().collect())
}

fn push_config(configuration: PushNotificationConfig) -> AgentJson {
    let mut fields = named(vec![("url", AgentJson::String(configuration.url))]);
    push_optional(
        &mut fields,
        "id",
        configuration
            .id
            .filter(|text| !text.is_empty())
            .map(AgentJson::String),
    );
    push_optional(
        &mut fields,
        "token",
        configuration
            .token
            .filter(|text| !text.is_empty())
            .map(AgentJson::String),
    );
    push_optional(
        &mut fields,
        "authentication",
        configuration.authentication.map(to_core_authentication),
    );
    AgentJson::Object(fields.into_iter().collect())
}

fn to_core_authentication(authentication: AuthenticationInfo) -> AgentJson {
    let mut fields = named(Vec::new());
    let scheme = authentication
        .schemes
        .first()
        .filter(|name| !name.is_empty())
        .cloned();
    push_optional(&mut fields, "scheme", scheme.map(AgentJson::String));
    push_optional(
        &mut fields,
        "credentials",
        authentication
            .credentials
            .filter(|text| !text.is_empty())
            .map(AgentJson::String),
    );
    AgentJson::Object(fields.into_iter().collect())
}
