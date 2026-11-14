use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::slice;
use std::sync::Arc;
use std::sync::PoisonError;

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;

use crate::alert_batching::alert_delivery::AlertDelivery;
use crate::alert_batching::alert_headers;
use crate::alert_digest::DigestAlert;
use crate::alert_settings::records::{AlertQueueItem, AlertText};
use crate::alert_teams::enqueue_ms_teams_alert;
use crate::budget_email::event::WebhookEvent;
use crate::http_analytics_loggers::delivery::Delivery;

pub mod budget;
pub mod destination;
pub mod format;

mod tests;

use self::budget::BudgetAlertSink;
use self::destination::{AlertDestination, MissingWebhookUrl, resolve_slack_webhook};
use self::format::format_message;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertLevel {
    Low,
    Medium,
    High,
}

impl AlertLevel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Alert<'alert> {
    pub message: &'alert str,
    pub level: AlertLevel,
    pub alert_type: AlertType,
    pub alerting_metadata: &'alert IndexMap<String, Json>,
    pub user_info: Option<&'alert WebhookEvent>,
    pub request_model: Option<&'alert str>,
    pub api_base: Option<&'alert str>,
    pub kwargs: &'alert IndexMap<String, Json>,
}

#[derive(Debug, thiserror::Error)]
pub enum SendAlertError<E>
where
    E: StdError + 'static,
{
    #[error(transparent)]
    MissingWebhookUrl(#[from] MissingWebhookUrl),
    #[error("the budget alert could not be delivered")]
    Budget(#[source] E),
}

pub struct AlertDispatcher<'dispatch, D, S> {
    pub alerting: Option<Vec<AlertDestination>>,
    pub alert_types: Vec<AlertType>,
    pub alert_to_webhook_url: BTreeMap<AlertType, WebhookUrl>,
    pub default_webhook_url: Option<WebhookUrl>,
    pub delivery: &'dispatch Arc<AlertDelivery<D>>,
    pub sink: &'dispatch S,
    pub tasks: &'dispatch TaskTracker,
}

impl<D, S> AlertDispatcher<'_, D, S>
where
    D: Delivery + 'static,
    S: BudgetAlertSink,
{
    pub async fn send_alert(&self, alert: Alert<'_>) -> Result<(), SendAlertError<S::Error>> {
        let Some(destinations) = self.alerting.as_deref() else {
            return Ok(());
        };
        if !destinations.is_empty() {
            self.delivery.ensure_started(self.tasks);
        }
        self.send_budget_alerts(destinations, &alert).await?;
        let send_to_slack = destinations.contains(&AlertDestination::Slack);
        let send_to_ms_teams = destinations.contains(&AlertDestination::MsTeams);
        if !send_to_slack && !send_to_ms_teams {
            return Ok(());
        }
        if !self.alert_types.contains(&alert.alert_type) {
            return Ok(());
        }
        let formatted = format_message(
            &alert,
            self.delivery.digests.clock.as_ref(),
            self.delivery.environment.as_ref(),
        );
        if send_to_ms_teams {
            let _enqueued = enqueue_ms_teams_alert(
                &self.delivery.queue,
                self.delivery.environment.as_ref(),
                &formatted,
                alert.alert_type,
            );
        }
        if !send_to_slack {
            self.flush_queue_if_full().await;
            return Ok(());
        }
        self.enqueue_slack_alert(&alert, &formatted).await
    }

    async fn send_budget_alerts(
        &self,
        destinations: &[AlertDestination],
        alert: &Alert<'_>,
    ) -> Result<(), SendAlertError<S::Error>> {
        if alert.alert_type != AlertType::BudgetAlerts {
            return Ok(());
        }
        let Some(event) = alert.user_info else {
            return Ok(());
        };
        if destinations.contains(&AlertDestination::Webhook) {
            self.sink
                .send_budget_webhook(event)
                .await
                .map_err(SendAlertError::Budget)?;
        }
        if destinations.contains(&AlertDestination::Email) {
            self.sink
                .send_budget_email(event, alert.alert_type)
                .await
                .map_err(SendAlertError::Budget)?;
        }
        Ok(())
    }

    async fn enqueue_slack_alert(
        &self,
        alert: &Alert<'_>,
        formatted: &str,
    ) -> Result<(), SendAlertError<S::Error>> {
        let webhook_url = resolve_slack_webhook(
            alert.alert_type,
            &self.alert_to_webhook_url,
            self.default_webhook_url.as_ref(),
            self.delivery.environment.as_ref(),
        )?;
        if self.delivery.digests.digest_enabled(alert.alert_type) {
            self.delivery.digests.record(DigestAlert {
                alert_type: alert.alert_type,
                request_model: alert.request_model,
                api_base: alert.api_base,
                message: alert.message,
                level: alert.level.as_str(),
                webhook_url,
            });
            return Ok(());
        }
        self.enqueue_slack(&webhook_url, alert.alert_type, formatted);
        self.flush_queue_if_full().await;
        Ok(())
    }

    fn enqueue_slack(&self, webhook_url: &WebhookUrl, alert_type: AlertType, formatted: &str) {
        let urls: &[SecretString] = match webhook_url {
            WebhookUrl::Single(url) => slice::from_ref(url),
            WebhookUrl::Multiple(urls) => urls.as_slice(),
        };
        for url in urls {
            let _enqueued = self.delivery.queue.enqueue(AlertQueueItem {
                url: url.clone(),
                headers: alert_headers(),
                payload: AlertText {
                    text: formatted.to_owned(),
                },
                alert_type,
                format: None,
            });
        }
    }

    async fn flush_queue_if_full(&self) {
        let pending = self
            .delivery
            .queue
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        if pending >= self.delivery.queue.settings.batch_size {
            let _report = self.delivery.async_send_batch().await;
        }
    }
}
