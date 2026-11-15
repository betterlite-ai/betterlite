#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use betterlite_logging::alert_dispatch::destination::AlertDestination;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use betterlite_logging::budget_email::event::WebhookEvent;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{BudgetAlertCache, BudgetAlertSender, BudgetAlerts};
use crate::failure_hook::ports::{BudgetAlertKind, CallInfo, EntityType};

#[derive(Clone)]
pub(super) struct Journal<T>(Arc<Mutex<Vec<T>>>);

impl<T> Default for Journal<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

impl<T: Clone> Journal<T> {
    pub(super) fn record(&self, entry: T) {
        self.0.lock().expect("journal lock").push(entry);
    }

    pub(super) fn entries(&self) -> Vec<T> {
        self.0.lock().expect("journal lock").clone()
    }
}

#[derive(Clone, Debug)]
pub(super) struct SentAlert {
    pub(super) message: String,
    pub(super) level: AlertLevel,
    pub(super) alert_type: AlertType,
    pub(super) user_info: Option<WebhookEvent>,
    pub(super) metadata: IndexMap<String, Json>,
    pub(super) kwargs: IndexMap<String, Json>,
}

impl SentAlert {
    pub(super) fn of(alert: Alert<'_>) -> Self {
        Self {
            message: alert.message.to_owned(),
            level: alert.level,
            alert_type: alert.alert_type,
            user_info: alert.user_info.cloned(),
            metadata: alert.alerting_metadata.clone(),
            kwargs: alert.kwargs.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CacheWrite {
    pub(super) key: String,
    pub(super) value: String,
    pub(super) ttl_seconds: u32,
}

#[derive(Clone, Default)]
pub(super) struct FakeCache {
    entries: Arc<Mutex<BTreeMap<String, String>>>,
    writes: Journal<CacheWrite>,
}

impl FakeCache {
    pub(super) fn seeded(key: &str) -> Self {
        let cache = Self::default();
        cache
            .entries
            .lock()
            .expect("cache lock")
            .insert(key.to_owned(), "SENT".to_owned());
        cache
    }

    pub(super) fn writes(&self) -> Vec<CacheWrite> {
        self.writes.entries()
    }
}

#[async_trait]
impl BudgetAlertCache for FakeCache {
    async fn get(&self, key: &str) -> Option<String> {
        self.entries.lock().expect("cache lock").get(key).cloned()
    }

    async fn set(&self, key: &str, value: &str, ttl_seconds: u32) {
        self.entries
            .lock()
            .expect("cache lock")
            .insert(key.to_owned(), value.to_owned());
        self.writes.record(CacheWrite {
            key: key.to_owned(),
            value: value.to_owned(),
            ttl_seconds,
        });
    }
}

#[derive(Clone, Default)]
pub(super) struct FakeSender {
    pub(super) alerts: Journal<SentAlert>,
    pub(super) webhooks: Journal<WebhookEvent>,
    fails: bool,
}

impl FakeSender {
    pub(super) fn failing() -> Self {
        Self {
            alerts: Journal::default(),
            webhooks: Journal::default(),
            fails: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake sender failed")]
pub(super) struct FakeSendError;

#[async_trait]
impl BudgetAlertSender for FakeSender {
    type Error = FakeSendError;

    async fn send_alert(&self, alert: Alert<'_>) -> Result<(), Self::Error> {
        if self.fails {
            return Err(FakeSendError);
        }
        self.alerts.record(SentAlert::of(alert));
        Ok(())
    }

    async fn send_webhook_alert(&self, event: &WebhookEvent) -> Result<(), Self::Error> {
        if self.fails {
            return Err(FakeSendError);
        }
        self.webhooks.record(event.clone());
        Ok(())
    }
}

pub(super) fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the test decimal parses")
}

pub(super) fn call_info(spend: Decimal) -> CallInfo {
    CallInfo {
        spend,
        max_budget: None,
        soft_budget: None,
        token: None,
        customer_id: None,
        user_id: None,
        team_id: None,
        team_alias: None,
        organization_id: None,
        user_email: None,
        key_alias: None,
        projected_exceeded_date: None,
        projected_spend: None,
        event_group: EntityType::Key,
        alert_emails: None,
        max_budget_alert_emails: None,
    }
}

pub(super) fn alerting_args(budget_alert_ttl: u32) -> SlackAlertingArgs {
    SlackAlertingArgs {
        daily_report_frequency: 43_200,
        report_check_interval: 300,
        budget_alert_ttl,
        outage_alert_ttl: 60,
        region_outage_alert_ttl: 60,
        minor_outage_alert_threshold: 5,
        major_outage_alert_threshold: 10,
        max_outage_alert_list_size: 10,
        log_to_console: false,
        daily_user_spend_threshold: None,
        monthly_user_spend_threshold: None,
        spend_anomaly_multiplier: decimal("3"),
        spend_anomaly_baseline_days: 7,
        spend_anomaly_min_spend: decimal("10"),
        user_spend_check_interval: 3600,
    }
}

pub(super) fn service(cache: FakeCache, sender: FakeSender) -> BudgetAlerts<FakeCache, FakeSender> {
    BudgetAlerts {
        alerting: Some(vec![AlertDestination::Slack, AlertDestination::Webhook]),
        alert_types: vec![AlertType::BudgetAlerts, AlertType::FailedTrackingSpend],
        alerting_args: SlackAlertingArgs::default(),
        cache,
        sender,
    }
}

pub(super) async fn written_key(kind: BudgetAlertKind, info: &CallInfo) -> String {
    let alerts = service(FakeCache::default(), FakeSender::default());
    let result = alerts.budget_alerts(kind, info).await;
    result.unwrap();
    let writes = alerts.cache.writes();
    let recorded = writes.first().expect("one cache write");
    recorded.key.clone()
}
