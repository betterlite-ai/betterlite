use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use betterlite_logging::alert_dispatch::destination::AlertDestination;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use betterlite_logging::budget_email::event::WebhookEvent;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::failure_hook::ports::{BudgetAlertKind, CallInfo};

pub mod events;
pub mod thresholds;

#[path = "budget_alerts/tests/cache_key.rs"]
mod cache_key;
#[path = "budget_alerts/tests/cache_ttl.rs"]
mod cache_ttl;
#[path = "budget_alerts/tests/customer_spend.rs"]
mod customer_spend;
#[path = "budget_alerts/tests/entry.rs"]
mod entry;
#[path = "budget_alerts/tests/failed_tracking.rs"]
mod failed_tracking;
#[path = "budget_alerts/tests/max_budget.rs"]
mod max_budget;
#[path = "budget_alerts/tests/percent_calc.rs"]
mod percent_calc;
#[path = "budget_alerts/tests/send.rs"]
mod send;
#[path = "budget_alerts/tests/soft_budget.rs"]
mod soft_budget;
#[path = "budget_alerts/tests/support.rs"]
mod support;
#[path = "budget_alerts/tests/team_member_message.rs"]
mod team_member_message;
#[path = "budget_alerts/tests/threshold_15pct.rs"]
mod threshold_15pct;
#[path = "budget_alerts/tests/threshold_5pct.rs"]
mod threshold_5pct;
#[path = "budget_alerts/tests/threshold_logic.rs"]
mod threshold_logic;
#[path = "budget_alerts/tests/user_info_str.rs"]
mod user_info_str;
#[path = "budget_alerts/tests/webhook_event.rs"]
mod webhook_event;

use self::events::{budget_alert, customer_spend_event};

pub struct CustomerSpend<'spend> {
    pub token: Option<&'spend str>,
    pub key_alias: Option<&'spend str>,
    pub end_user_id: Option<&'spend str>,
    pub response_cost: Option<Decimal>,
    pub max_budget: Option<Decimal>,
}

#[async_trait]
pub trait BudgetAlertCache: Send + Sync {
    async fn get(&self, key: &str) -> Option<String>;

    async fn set(&self, key: &str, value: &str, ttl_seconds: u32);
}

#[async_trait]
pub trait BudgetAlertSender: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn send_alert(&self, alert: Alert<'_>) -> Result<(), Self::Error>;

    async fn send_webhook_alert(&self, event: &WebhookEvent) -> Result<(), Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum BudgetAlertError<E>
where
    E: StdError + 'static,
{
    #[error("the budget alert could not be sent")]
    Alert(#[source] E),
    #[error("the customer spend webhook could not be sent")]
    Webhook(#[source] E),
}

pub struct BudgetAlerts<C, S> {
    pub alerting: Option<Vec<AlertDestination>>,
    pub alert_types: Vec<AlertType>,
    pub alerting_args: SlackAlertingArgs,
    pub cache: C,
    pub sender: S,
}

impl<C, S> BudgetAlerts<C, S>
where
    C: BudgetAlertCache,
    S: BudgetAlertSender,
{
    pub async fn failed_tracking_alert(
        &self,
        error_message: &str,
        failing_model: &str,
    ) -> Result<(), BudgetAlertError<S::Error>> {
        if self.alerting.is_none() || !self.alert_types.contains(&AlertType::FailedTrackingSpend) {
            return Ok(());
        }
        let cache_key = format!("budget_alerts:failed_tracking:{failing_model}");
        if self.cache.get(&cache_key).await.is_some() {
            return Ok(());
        }
        let message = format!("Failed Tracking Cost for {error_message}");
        let metadata = IndexMap::new();
        let alert = Alert {
            message: &message,
            level: AlertLevel::High,
            alert_type: AlertType::FailedTrackingSpend,
            alerting_metadata: &metadata,
            user_info: None,
            request_model: None,
            api_base: None,
            kwargs: &metadata,
        };
        self.sender
            .send_alert(alert)
            .await
            .map_err(BudgetAlertError::Alert)?;
        self.cache
            .set(&cache_key, "SENT", self.alerting_args.budget_alert_ttl)
            .await;
        Ok(())
    }

    pub async fn budget_alerts(
        &self,
        kind: BudgetAlertKind,
        user_info: &CallInfo,
    ) -> Result<(), BudgetAlertError<S::Error>> {
        if self.alerting.is_none() || !self.alert_types.contains(&AlertType::BudgetAlerts) {
            return Ok(());
        }
        let Some(request) = budget_alert(kind, user_info) else {
            return Ok(());
        };
        let cache_key = format!("budget_alerts:{}:{}", request.event, request.id);
        if self.cache.get(&cache_key).await.is_some() {
            return Ok(());
        }
        let metadata = IndexMap::new();
        let alert = Alert {
            message: &request.message,
            level: AlertLevel::High,
            alert_type: AlertType::BudgetAlerts,
            alerting_metadata: &metadata,
            user_info: Some(&request.webhook_event),
            request_model: None,
            api_base: None,
            kwargs: &metadata,
        };
        self.sender
            .send_alert(alert)
            .await
            .map_err(BudgetAlertError::Alert)?;
        self.cache
            .set(&cache_key, "SENT", self.alerting_args.budget_alert_ttl)
            .await;
        Ok(())
    }

    pub async fn customer_spend_alert(
        &self,
        spend: CustomerSpend<'_>,
    ) -> Result<(), BudgetAlertError<S::Error>> {
        let Some(destinations) = self.alerting.as_ref() else {
            return Ok(());
        };
        if !destinations.contains(&AlertDestination::Webhook) {
            return Ok(());
        }
        let (Some(token), Some(end_user_id), Some(response_cost)) =
            (spend.token, spend.end_user_id, spend.response_cost)
        else {
            return Ok(());
        };
        let event = customer_spend_event(&spend, token, end_user_id, response_cost);
        self.sender
            .send_webhook_alert(&event)
            .await
            .map_err(BudgetAlertError::Webhook)?;
        Ok(())
    }
}
