use betterlite_core::agents::json::AgentJson;

use crate::version_convert::JsonObject;

use crate::version_convert::version_conversion::error::ConvertError;
use crate::version_convert::version_conversion::json::encode;
use crate::version_convert::version_conversion::request::original;

use super::records::{
    AuthenticationRequest, PushConfigAuthentication, PushConfigEnvelope, PushNotificationConfig,
    TaskPushNotificationConfig,
};

pub(super) fn lower_set(request: &AgentJson) -> Result<JsonObject, ConvertError> {
    let envelope: PushConfigEnvelope =
        serde_json::from_value(request.clone()).map_err(ConvertError::Decode)?;
    let Some(params) = flatten(envelope) else {
        return original(request);
    };
    encode(&params)
}

fn flatten(envelope: PushConfigEnvelope) -> Option<TaskPushNotificationConfig> {
    let nested = envelope.config.or(envelope.push_notification_config)?;
    let config_id = envelope.config_id.filter(|text| !text.is_empty());
    let authentication = nested
        .authentication
        .or(envelope.authentication)
        .map(authentication);
    let task_id = envelope
        .task_id
        .or_else(|| parent_task_id(envelope.parent.as_deref()))
        .unwrap_or_default();
    Some(TaskPushNotificationConfig {
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
    })
}

fn parent_task_id(parent: Option<&str>) -> Option<String> {
    let rest = parent?.strip_prefix("tasks/")?;
    let task_id = rest.split('/').next().unwrap_or_default();
    Some(task_id.to_owned())
}

fn authentication(auth: AuthenticationRequest) -> PushConfigAuthentication {
    let mut schemes = Vec::new();
    if let Some(scheme) = auth.scheme.filter(|text| !text.is_empty()) {
        schemes.push(scheme);
    }
    PushConfigAuthentication {
        schemes,
        credentials: auth.credentials.filter(|text| !text.is_empty()),
    }
}
