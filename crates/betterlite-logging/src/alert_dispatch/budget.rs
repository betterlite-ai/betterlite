use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::AlertType;

use crate::budget_email::event::WebhookEvent;

// LiteLLM's send_webhook_alert and send_email_alert_using_smtp run before the Slack/Teams destination check
#[async_trait]
pub trait BudgetAlertSink: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn send_budget_webhook(&self, event: &WebhookEvent) -> Result<(), Self::Error>;

    async fn send_budget_email(
        &self,
        event: &WebhookEvent,
        alert_type: AlertType,
    ) -> Result<(), Self::Error>;
}
