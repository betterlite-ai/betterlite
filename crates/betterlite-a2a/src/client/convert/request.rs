use crate::client::records::compat::{
    AuthenticationInfo, MessageSendConfiguration, PushNotificationConfig,
};
use crate::client::records::core::{
    CoreAuthenticationInfo, CorePushNotificationConfig, CoreSendMessageConfiguration,
};

#[must_use]
pub fn to_core_configuration(
    configuration: MessageSendConfiguration,
) -> CoreSendMessageConfiguration {
    CoreSendMessageConfiguration {
        accepted_output_modes: configuration.accepted_output_modes.unwrap_or_default(),
        task_push_notification_config: configuration
            .push_notification_config
            .map(to_core_push_configuration),
        history_length: configuration.history_length,
        return_immediately: configuration.blocking.is_some_and(|blocking| !blocking),
    }
}

fn to_core_push_configuration(configuration: PushNotificationConfig) -> CorePushNotificationConfig {
    CorePushNotificationConfig {
        url: configuration.url,
        id: configuration.id.filter(|value| !value.is_empty()),
        token: configuration.token.filter(|value| !value.is_empty()),
        authentication: configuration.authentication.map(to_core_authentication),
    }
}

fn to_core_authentication(authentication: AuthenticationInfo) -> CoreAuthenticationInfo {
    CoreAuthenticationInfo {
        scheme: authentication
            .schemes
            .first()
            .filter(|name| !name.is_empty())
            .cloned(),
        credentials: authentication.credentials.filter(|value| !value.is_empty()),
    }
}
