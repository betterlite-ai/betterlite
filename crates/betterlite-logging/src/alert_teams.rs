use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::alert_settings::types::AlertType;
use secrecy::SecretString;
use serde::Serialize;

use crate::alert_batching::alert_headers;
use crate::alert_settings::records::{AlertQueueItem, AlertText};
use crate::batch::logger::{BatchQueue, Enqueued};

pub const MS_TEAMS_WEBHOOK_URL_ENV: &str = "MS_TEAMS_WEBHOOK_URL";
pub const MS_TEAMS_ALERTING_DESTINATION: &str = "ms_teams";

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MSTeamsTextBlock {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: String,
    pub wrap: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MSTeamsAdaptiveCard {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub version: &'static str,
    pub body: Vec<MSTeamsTextBlock>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MSTeamsAttachment {
    #[serde(rename = "contentType")]
    pub content_type: &'static str,
    pub content: MSTeamsAdaptiveCard,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MSTeamsMessage {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub attachments: Vec<MSTeamsAttachment>,
}

#[must_use]
pub fn build_ms_teams_payload(text: &str) -> MSTeamsMessage {
    MSTeamsMessage {
        kind: "message",
        attachments: vec![MSTeamsAttachment {
            content_type: "application/vnd.microsoft.card.adaptive",
            content: MSTeamsAdaptiveCard {
                kind: "AdaptiveCard",
                version: "1.4",
                body: vec![MSTeamsTextBlock {
                    kind: "TextBlock",
                    text: text.to_owned(),
                    wrap: true,
                }],
            },
        }],
    }
}

#[must_use]
pub fn enqueue_ms_teams_alert(
    queue: &BatchQueue<AlertQueueItem>,
    environment: &dyn EnvironmentLookup,
    formatted_message: &str,
    alert_type: AlertType,
) -> Option<Enqueued> {
    let Some(url) = environment
        .lookup(MS_TEAMS_WEBHOOK_URL_ENV)
        .filter(|value| !value.is_empty())
    else {
        tracing::error!(
            %alert_type,
            "MS Teams alerting is enabled but {MS_TEAMS_WEBHOOK_URL_ENV} is not set; dropping the alert"
        );
        return None;
    };
    let item = AlertQueueItem {
        url: SecretString::from(url),
        headers: alert_headers(),
        payload: AlertText {
            text: formatted_message.to_owned(),
        },
        alert_type,
        format: Some(MS_TEAMS_ALERTING_DESTINATION.to_owned()),
    };
    Some(queue.enqueue(item))
}

mod tests;
