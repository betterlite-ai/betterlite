use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

pub const SLACK_WEBHOOK_URL_ENV: &str = "SLACK_WEBHOOK_URL";
pub const ALERTING_WEBHOOK_URL_ENV: &str = "ALERTING_WEBHOOK_URL";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertDestination {
    Slack,
    Email,
    MsTeams,
    Sentry,
    Webhook,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("Missing SLACK_WEBHOOK_URL / ALERTING_WEBHOOK_URL from environment")]
pub struct MissingWebhookUrl;

// LiteLLM's `os.getenv("SLACK_WEBHOOK_URL") or os.getenv("ALERTING_WEBHOOK_URL")` treats an empty value as unset
pub fn resolve_slack_webhook(
    alert_type: AlertType,
    alert_to_webhook_url: &BTreeMap<AlertType, WebhookUrl>,
    default_webhook_url: Option<&WebhookUrl>,
    environment: &dyn EnvironmentLookup,
) -> Result<WebhookUrl, MissingWebhookUrl> {
    if let Some(webhook_url) = alert_to_webhook_url.get(&alert_type) {
        return Ok(webhook_url.clone());
    }
    if let Some(webhook_url) = default_webhook_url {
        return Ok(webhook_url.clone());
    }
    let from_environment = environment
        .lookup(SLACK_WEBHOOK_URL_ENV)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            environment
                .lookup(ALERTING_WEBHOOK_URL_ENV)
                .filter(|value| !value.is_empty())
        });
    from_environment
        .map(|url| WebhookUrl::Single(SecretString::from(url)))
        .ok_or(MissingWebhookUrl)
}
