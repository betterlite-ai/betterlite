use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::destination::AlertDestination;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use chrono::NaiveDate;
use indexmap::IndexMap;

use super::{
    UserSpendAlertError, UserSpendAlertEvent, UserSpendAlerts, UserSpendChecks, UserSpendRow,
    UserSpendSource, evaluate_user_spend,
};
use crate::alerting::spend_reports::{SpendAlertCache, SpendAlertClock, SpendAlertSender};

impl<C, S, R, K> UserSpendAlerts<C, S, R, K>
where
    C: SpendAlertCache,
    S: SpendAlertSender,
    R: UserSpendSource,
    K: SpendAlertClock,
{
    pub async fn send_user_spend_alerts(&self, has_database: bool) {
        if let Err(error) = self.send_pass(has_database).await {
            tracing::error!("Error sending user spend alerts: {error}");
        }
    }

    async fn send_pass(
        &self,
        has_database: bool,
    ) -> Result<(), UserSpendAlertError<R::Error, S::Error>> {
        let Some(checks) = self.enabled_checks() else {
            return Ok(());
        };
        if !has_database {
            return Ok(());
        }
        let today = self.clock.utc_date();
        let rows = self
            .source
            .rows(today, self.alerting_args.spend_anomaly_baseline_days)
            .await
            .map_err(UserSpendAlertError::Rows)?;
        let events = self.evaluate_rows(&rows, today, checks);
        let new_events = self.uncached(events).await;
        self.send_events(&new_events).await
    }

    // None when no Slack destination is set or neither user spend type is enabled
    fn enabled_checks(&self) -> Option<UserSpendChecks> {
        let destinations = self.alerting.as_deref()?;
        if !destinations.contains(&AlertDestination::Slack) {
            return None;
        }
        let checks = UserSpendChecks {
            thresholds: self.alert_types.contains(&AlertType::UserSpendThresholds),
            anomalies: self.alert_types.contains(&AlertType::UserSpendAnomalies),
        };
        if !checks.thresholds && !checks.anomalies {
            return None;
        }
        Some(checks)
    }

    fn evaluate_rows(
        &self,
        rows: &[UserSpendRow],
        today: NaiveDate,
        checks: UserSpendChecks,
    ) -> Vec<UserSpendAlertEvent> {
        let mut events = Vec::new();
        for row in rows {
            events.extend(evaluate_user_spend(
                row,
                &self.alerting_args,
                today,
                checks,
                self.hours_in_a_day,
            ));
        }
        events
    }

    async fn uncached(&self, events: Vec<UserSpendAlertEvent>) -> Vec<UserSpendAlertEvent> {
        let mut new_events = Vec::new();
        for event in events {
            if self.cache.get(&event.cache_key).await.is_none() {
                new_events.push(event);
            }
        }
        new_events
    }

    async fn send_events(
        &self,
        events: &[UserSpendAlertEvent],
    ) -> Result<(), UserSpendAlertError<R::Error, S::Error>> {
        for alert_type in [
            AlertType::UserSpendThresholds,
            AlertType::UserSpendAnomalies,
        ] {
            self.send_type(events, alert_type).await?;
        }
        Ok(())
    }

    async fn send_type(
        &self,
        events: &[UserSpendAlertEvent],
        alert_type: AlertType,
    ) -> Result<(), UserSpendAlertError<R::Error, S::Error>> {
        let typed: Vec<&UserSpendAlertEvent> = events
            .iter()
            .filter(|event| event.alert_type == alert_type)
            .collect();
        if typed.is_empty() {
            return Ok(());
        }
        let message = typed
            .iter()
            .map(|event| event.message.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
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
        self.sender
            .send_alert(alert)
            .await
            .map_err(UserSpendAlertError::Send)?;
        for event in typed {
            self.cache
                .set(&event.cache_key, "SENT", event.cache_ttl)
                .await;
        }
        Ok(())
    }
}
