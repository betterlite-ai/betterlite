use std::collections::BTreeMap;

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlertText {
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct AlertQueueItem {
    pub url: SecretString,
    pub headers: BTreeMap<String, String>,
    pub payload: AlertText,
    pub alert_type: AlertType,
    pub format: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HangingRequestData {
    pub request_id: String,
    pub model: String,
    pub api_base: Option<String>,
    pub key_alias: Option<String>,
    pub team_alias: Option<String>,
    pub alerting_metadata: Option<IndexMap<String, Json>>,
    pub created_at: f64,
    pub alerted: bool,
}

impl HangingRequestData {
    #[must_use]
    pub const fn new(request_id: String, model: String, created_at: f64) -> Self {
        Self {
            request_id,
            model,
            api_base: None,
            key_alias: None,
            team_alias: None,
            alerting_metadata: None,
            created_at,
            alerted: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DigestEntry {
    pub alert_type: String,
    pub request_model: String,
    pub api_base: String,
    pub first_message: String,
    pub level: String,
    pub count: u64,
    pub start_time: DateTime<FixedOffset>,
    pub last_time: DateTime<FixedOffset>,
    pub webhook_url: WebhookUrl,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentMetrics {
    pub id: String,
    pub failed_request: bool,
    pub latency_per_output_token: Option<f64>,
    pub updated_at: DateTime<FixedOffset>,
}
