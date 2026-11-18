use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use super::ModelAlertSender;

pub struct VirtualKeyEvent {
    pub created_by_user_id: String,
    pub created_by_user_role: String,
    pub created_by_key_alias: Option<String>,
    pub request_kwargs: JsonObject,
}

pub struct KeyEventAlerts<Sender> {
    pub sender: Sender,
}

impl<Sender> KeyEventAlerts<Sender>
where
    Sender: ModelAlertSender,
{
    pub async fn send_virtual_key_event_slack(
        &self,
        key_event: &VirtualKeyEvent,
        alert_type: AlertType,
        event_name: &str,
    ) {
        let message = match render_key_event_message(key_event, event_name) {
            Ok(message) => message,
            Err(error) => {
                tracing::error!("Error sending send_virtual_key_event_slack {error}");
                return;
            }
        };
        let metadata = IndexMap::new();
        let alert = Alert {
            message: &message,
            level: AlertLevel::High,
            alert_type,
            alerting_metadata: &metadata,
            user_info: None,
            request_model: None,
            api_base: None,
            kwargs: &metadata,
        };
        if let Err(error) = self.sender.send_alert(alert).await {
            tracing::error!("Error sending send_virtual_key_event_slack {error}");
        }
    }
}

fn render_key_event_message(
    key_event: &VirtualKeyEvent,
    event_name: &str,
) -> Result<String, serde_json::Error> {
    let alias = key_event.created_by_key_alias.as_deref().unwrap_or("None");
    let mut lines = vec![
        format!("`{event_name}`"),
        "*Action Done by:*".to_owned(),
        format!("created_by_user_id: `{}`", key_event.created_by_user_id),
        format!("created_by_user_role: `{}`", key_event.created_by_user_role),
        format!("created_by_key_alias: `{alias}`"),
        String::new(),
        "*Arguments passed:*".to_owned(),
    ];
    for (key, value) in &key_event.request_kwargs {
        if key == "user_api_key_dict" {
            continue;
        }
        lines.push(format!("{key}: `{}`", render_argument(value)?));
    }
    let mut message = lines.join("\n");
    message.push('\n');
    Ok(message)
}

fn render_argument(value: &Json) -> Result<String, serde_json::Error> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        Json::Null => Ok("None".to_owned()),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value)
        }
    }
}

mod tests;
