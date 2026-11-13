use betterlite_core::agents::json::AgentJson;

use crate::version_convert::JsonObject;

use crate::version_convert::version_conversion::error::ConvertError;
use crate::version_convert::version_conversion::json::encode;

use super::records::{
    AuthenticationRequest, PushConfigAuthentication, PushConfigEnvelope, PushConfigRequest,
    PushNotificationConfig, TaskPushNotificationConfig,
};

pub(super) fn lower_set(request: &AgentJson) -> Result<JsonObject, ConvertError> {
    let envelope: PushConfigEnvelope =
        serde_json::from_value(request.clone()).map_err(ConvertError::Decode)?;
    encode(&flatten(envelope)?)
}

fn flatten(mut envelope: PushConfigEnvelope) -> Result<TaskPushNotificationConfig, ConvertError> {
    let config = envelope.config.take();
    let push_config = envelope.push_notification_config.take();
    let request = match config.or(push_config) {
        Some(value) if value.is_object() => {
            let decoded: PushConfigRequest =
                serde_json::from_value(value).map_err(ConvertError::Decode)?;
            Some(decoded)
        }
        Some(_) | None => None,
    };
    let Some(nested) = request else {
        return Ok(flat_params(envelope));
    };
    Ok(merged_params(envelope, nested))
}

fn merged_params(
    envelope: PushConfigEnvelope,
    nested: PushConfigRequest,
) -> TaskPushNotificationConfig {
    let config_id = envelope.config_id.filter(|text| !text.is_empty());
    let authentication = nested
        .authentication
        .or(envelope.authentication)
        .map(compat_authentication);
    let task_id = nested
        .task_id
        .or(envelope.task_id)
        .or_else(|| parent_task_id(envelope.parent.as_deref()))
        .unwrap_or_default();
    TaskPushNotificationConfig {
        task_id,
        push_notification_config: PushNotificationConfig {
            url: nested.url.or(envelope.url).unwrap_or_default(),
            id: nested
                .id
                .or(config_id)
                .or(envelope.id)
                .filter(|text| !text.is_empty()),
            token: nested
                .token
                .or(envelope.token)
                .filter(|text| !text.is_empty()),
            authentication,
        },
    }
}

fn flat_params(envelope: PushConfigEnvelope) -> TaskPushNotificationConfig {
    TaskPushNotificationConfig {
        task_id: envelope.task_id.unwrap_or_default(),
        push_notification_config: PushNotificationConfig {
            url: envelope.url.unwrap_or_default(),
            id: envelope.id.filter(|text| !text.is_empty()),
            token: envelope.token.filter(|text| !text.is_empty()),
            authentication: envelope.authentication.map(compat_authentication),
        },
    }
}

fn parent_task_id(parent: Option<&str>) -> Option<String> {
    let rest = parent?.strip_prefix("tasks/")?;
    let task_id = rest.split('/').next().unwrap_or_default();
    Some(task_id.to_owned())
}

fn compat_authentication(auth: AuthenticationRequest) -> PushConfigAuthentication {
    let mut schemes = Vec::new();
    if let Some(scheme) = auth.scheme.filter(|text| !text.is_empty()) {
        schemes.push(scheme);
    }
    PushConfigAuthentication {
        schemes,
        credentials: auth.credentials.filter(|text| !text.is_empty()),
    }
}
