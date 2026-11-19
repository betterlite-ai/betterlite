use std::convert::Infallible;
use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

use crate::alerting::model_alerts::ModelAlertSender;
use crate::alerting::model_alerts::key_event::{KeyEventAlerts, VirtualKeyEvent};

// LiteLLM's fallback for a key with no acting user or role
const UNKNOWN_ACTOR: &str = "Unknown";

// LiteLLM's management_function_to_event_name: the nine management handlers that raise a slack alert
#[must_use]
pub fn management_event_alert(function_name: &str) -> Option<AlertType> {
    match function_name {
        "generate_key_fn" => Some(AlertType::NewVirtualKeyCreated),
        "update_key_fn" => Some(AlertType::VirtualKeyUpdated),
        "delete_key_fn" => Some(AlertType::VirtualKeyDeleted),
        "new_team" => Some(AlertType::NewTeamCreated),
        "update_team" => Some(AlertType::TeamUpdated),
        "delete_team" => Some(AlertType::TeamDeleted),
        "new_user" => Some(AlertType::NewInternalUserCreated),
        "user_update" => Some(AlertType::InternalUserUpdated),
        "delete_user" => Some(AlertType::InternalUserDeleted),
        _ => None,
    }
}

// LiteLLM's `alert_type.replace("_", " ").title()` on the snake_case alert name
#[must_use]
pub fn displayed_event_name(alert_type: AlertType) -> String {
    let mut titled = String::new();
    let mut word_start = true;
    for character in alert_type.to_string().chars() {
        if character == '_' {
            titled.push(' ');
            word_start = true;
            continue;
        }
        if word_start {
            titled.extend(character.to_uppercase());
            word_start = false;
            continue;
        }
        titled.push(character);
    }
    titled
}

// The three UserAPIKeyAuth fields the alert reads, taken from the key's serde form
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ManagementActor {
    pub user_id: Option<String>,
    pub user_role: Option<UserRole>,
    pub key_alias: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManagementAlertError<E>
where
    E: StdError + 'static,
{
    #[error("the acting user cannot be read from the api key")]
    Actor(#[source] serde_json::Error),
    #[error("the management alert could not be sent")]
    Alert(#[source] E),
}

// The alert transport B4030 builds; its send swallows the slack failure, so the port cannot fail
#[async_trait]
pub trait ManagementAlertTransport: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn send_management_event_alert(
        &self,
        key_event: &VirtualKeyEvent,
        alert_type: AlertType,
        event_name: &str,
    ) -> Result<(), Self::Error>;
}

#[async_trait]
impl<Sender> ManagementAlertTransport for KeyEventAlerts<Sender>
where
    Sender: ModelAlertSender,
{
    type Error = Infallible;

    async fn send_management_event_alert(
        &self,
        key_event: &VirtualKeyEvent,
        alert_type: AlertType,
        event_name: &str,
    ) -> Result<(), Self::Error> {
        self.send_virtual_key_event_slack(key_event, alert_type, event_name)
            .await;
        Ok(())
    }
}

// an absent actor reads "Unknown"; an explicit "" is used as given, since truthiness is not copied
fn key_event(actor: &ManagementActor, request_kwargs: JsonObject) -> VirtualKeyEvent {
    VirtualKeyEvent {
        created_by_user_id: actor
            .user_id
            .clone()
            .unwrap_or_else(|| UNKNOWN_ACTOR.to_owned()),
        created_by_user_role: actor.user_role.map_or_else(
            || UNKNOWN_ACTOR.to_owned(),
            |user_role| user_role.to_string(),
        ),
        created_by_key_alias: actor.key_alias.clone(),
        request_kwargs,
    }
}

// LiteLLM's send_management_endpoint_alert, with the missing logger and slack sink both read as no transport
pub async fn send_management_endpoint_alert<Transport>(
    transport: Option<&Transport>,
    function_name: &str,
    request_kwargs: JsonObject,
    user_api_key_dict: &UserAPIKeyAuth,
) -> Result<(), ManagementAlertError<Transport::Error>>
where
    Transport: ManagementAlertTransport,
{
    let Some(transport) = transport else {
        return Ok(());
    };
    let Some(alert_type) = management_event_alert(function_name) else {
        return Ok(());
    };
    let key_fields =
        serde_json::to_value(user_api_key_dict).map_err(ManagementAlertError::Actor)?;
    let actor: ManagementActor =
        serde_json::from_value(key_fields).map_err(ManagementAlertError::Actor)?;
    let event_name = displayed_event_name(alert_type);
    let key_event = key_event(&actor, request_kwargs);
    transport
        .send_management_event_alert(&key_event, alert_type, &event_name)
        .await
        .map_err(ManagementAlertError::Alert)
}
