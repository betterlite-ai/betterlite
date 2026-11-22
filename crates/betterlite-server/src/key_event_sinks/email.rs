use std::sync::Arc;

use betterlite_logging::budget_email::event::{EntityType, WebhookEvent, WebhookEventType};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Deserialize;
use tokio_util::task::TaskTracker;
use tracing::debug;

use super::error::EmailError;
use super::ports::KeyCreatedMailer;

// the dumped key response is a dict, so a key it lacks takes the event default while an explicit null stays null
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
pub enum Reported<T> {
    Value(T),
    Null(()),
    #[default]
    Missing,
}

impl<T> Reported<T> {
    fn or_missing(self, missing: Option<T>) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Null(()) => None,
            Self::Missing => missing,
        }
    }
}

// the dumped GenerateKeyResponse holds every key field, and the legacy event reads only these, never the new key
#[derive(Debug, Default, Deserialize)]
pub struct CreatedKey {
    #[serde(default)]
    pub token: Reported<SecretString>,
    #[serde(default)]
    pub spend: Reported<Decimal>,
    #[serde(default)]
    pub max_budget: Reported<Decimal>,
    #[serde(default)]
    pub user_id: Reported<String>,
    #[serde(default)]
    pub team_id: Reported<String>,
    #[serde(default)]
    pub key_alias: Reported<String>,
}

pub struct LegacyEmail<'deps> {
    // general_settings.alerting; the enterprise email loggers are not built
    pub alerting: &'deps [String],
    pub mailer: Arc<dyn KeyCreatedMailer>,
    pub tasks: &'deps TaskTracker,
}

impl LegacyEmail<'_> {
    // the email goes out on the tracker, so key creation never waits for delivery
    pub fn send_key_created_email(&self, response: CreatedKey) -> Result<(), EmailError> {
        if !self
            .alerting
            .iter()
            .any(|destination| destination == "email")
        {
            debug!("Email sending not enabled, skipping key created email");
            return Ok(());
        }
        let spend = match response.spend {
            Reported::Value(value) => value,
            Reported::Missing => Decimal::ZERO,
            Reported::Null(()) => return Err(EmailError::NullSpend),
        };
        let event = WebhookEvent {
            event: WebhookEventType::KeyCreated,
            event_message: "API Key Created".to_owned(),
            event_group: EntityType::Key,
            spend,
            max_budget: response.max_budget.or_missing(Some(Decimal::ZERO)),
            soft_budget: None,
            token: response.token.or_missing(Some(SecretString::from(""))),
            customer_id: None,
            user_id: response.user_id.or_missing(None),
            team_id: response.team_id.or_missing(Some("Default Team".to_owned())),
            team_alias: None,
            organization_id: None,
            user_email: None,
            key_alias: response.key_alias.or_missing(None),
            projected_exceeded_date: None,
            projected_spend: None,
            alert_emails: None,
            max_budget_alert_emails: None,
        };
        let mailer = Arc::clone(&self.mailer);
        let _task = self.tasks.spawn(async move { mailer.send(event).await });
        Ok(())
    }

    // v0 alerting has no rotated-key email, and the enterprise loggers that send one are not built
    pub fn send_key_rotated_email(&self) {
        if self
            .alerting
            .iter()
            .any(|destination| destination == "email")
        {
            debug!("Enterprise package not installed, skipping key rotated email");
        } else {
            debug!("Email sending not enabled, skipping key rotated email");
        }
    }
}
