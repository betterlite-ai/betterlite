#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use betterlite_logging::alert_dispatch::destination::AlertDestination;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::{UserSpendAlerts, UserSpendRow, UserSpendSource};
use crate::alerting::spend_reports::{SpendAlertCache, SpendAlertClock, SpendAlertSender};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CacheWrite {
    pub(super) key: String,
    pub(super) value: String,
    pub(super) ttl_seconds: u64,
}

#[derive(Clone, Default)]
pub(super) struct FakeCache {
    entries: Arc<Mutex<BTreeMap<String, String>>>,
    writes: Arc<Mutex<Vec<CacheWrite>>>,
}

impl FakeCache {
    pub(super) fn seeded(keys: &[&str]) -> Self {
        let cache = Self::default();
        cache.entries.lock().unwrap().extend(
            keys.iter()
                .map(|key| ((*key).to_owned(), "SENT".to_owned())),
        );
        cache
    }

    pub(super) fn written(&self) -> Vec<CacheWrite> {
        self.writes.lock().unwrap().clone()
    }
}

#[async_trait]
impl SpendAlertCache for FakeCache {
    async fn get(&self, key: &str) -> Option<String> {
        self.entries.lock().unwrap().get(key).cloned()
    }

    async fn set(&self, key: &str, value: &str, ttl_seconds: u64) {
        self.entries
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.to_owned());
        self.writes.lock().unwrap().push(CacheWrite {
            key: key.to_owned(),
            value: value.to_owned(),
            ttl_seconds,
        });
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SentAlert {
    pub(super) message: String,
    pub(super) level: AlertLevel,
    pub(super) alert_type: AlertType,
    pub(super) metadata_len: usize,
    pub(super) kwargs_len: usize,
}

impl SentAlert {
    fn of(alert: Alert<'_>) -> Self {
        Self {
            message: alert.message.to_owned(),
            level: alert.level,
            alert_type: alert.alert_type,
            metadata_len: alert.alerting_metadata.len(),
            kwargs_len: alert.kwargs.len(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake sender failed")]
pub(super) struct FakeSendError;

#[derive(Clone, Default)]
pub(super) struct FakeSender {
    sent: Arc<Mutex<Vec<SentAlert>>>,
    fails_on: Option<AlertType>,
}

impl FakeSender {
    pub(super) fn failing_on(alert_type: AlertType) -> Self {
        Self {
            sent: Arc::default(),
            fails_on: Some(alert_type),
        }
    }

    pub(super) fn alerts(&self) -> Vec<SentAlert> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl SpendAlertSender for FakeSender {
    type Error = FakeSendError;

    async fn send_alert(&self, alert: Alert<'_>) -> Result<(), Self::Error> {
        if self.fails_on == Some(alert.alert_type) {
            return Err(FakeSendError);
        }
        self.sent.lock().unwrap().push(SentAlert::of(alert));
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake source failed")]
pub(super) struct FakeSourceError;

#[derive(Clone, Default)]
pub(super) struct FakeSource {
    rows: Vec<UserSpendRow>,
    fails: bool,
    asked: Arc<Mutex<Vec<(NaiveDate, u32)>>>,
}

impl FakeSource {
    pub(super) fn returning(rows: Vec<UserSpendRow>) -> Self {
        Self {
            rows,
            fails: false,
            asked: Arc::default(),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            rows: Vec::new(),
            fails: true,
            asked: Arc::default(),
        }
    }

    pub(super) fn queried(&self) -> Arc<Mutex<Vec<(NaiveDate, u32)>>> {
        self.asked.clone()
    }
}

#[async_trait]
impl UserSpendSource for FakeSource {
    type Error = FakeSourceError;

    async fn rows(
        &self,
        today: NaiveDate,
        baseline_days: u32,
    ) -> Result<Vec<UserSpendRow>, FakeSourceError> {
        self.asked.lock().unwrap().push((today, baseline_days));
        if self.fails {
            return Err(FakeSourceError);
        }
        Ok(self.rows.clone())
    }
}

#[derive(Clone, Copy)]
pub(super) struct FakeClock {
    pub(super) local: NaiveDate,
    pub(super) utc: NaiveDate,
}

impl SpendAlertClock for FakeClock {
    fn local_date(&self) -> NaiveDate {
        self.local
    }

    fn utc_date(&self) -> NaiveDate {
        self.utc
    }
}

pub(super) fn date(value: &str) -> NaiveDate {
    value.parse().unwrap()
}

pub(super) fn decimal(value: &str) -> Decimal {
    Decimal::from_str(value).unwrap()
}

pub(super) fn row(user_id: &str, daily: f64, monthly: f64, baseline: f64) -> UserSpendRow {
    UserSpendRow {
        user_id: user_id.to_owned(),
        daily_spend: Decimal::from_f64_retain(daily).unwrap(),
        monthly_spend: Decimal::from_f64_retain(monthly).unwrap(),
        baseline_spend: Decimal::from_f64_retain(baseline).unwrap(),
    }
}

pub(super) fn args() -> SlackAlertingArgs {
    SlackAlertingArgs {
        daily_report_frequency: 43_200,
        report_check_interval: 300,
        budget_alert_ttl: 86_400,
        outage_alert_ttl: 60,
        region_outage_alert_ttl: 60,
        minor_outage_alert_threshold: 5,
        major_outage_alert_threshold: 10,
        max_outage_alert_list_size: 10,
        log_to_console: false,
        daily_user_spend_threshold: Some(decimal("10")),
        monthly_user_spend_threshold: Some(decimal("25")),
        spend_anomaly_multiplier: decimal("3"),
        spend_anomaly_baseline_days: 7,
        spend_anomaly_min_spend: decimal("10"),
        user_spend_check_interval: 3600,
    }
}

pub(super) fn reporting(
    cache: FakeCache,
    sender: FakeSender,
    source: FakeSource,
) -> UserSpendAlerts<FakeCache, FakeSender, FakeSource, FakeClock> {
    UserSpendAlerts {
        alerting: Some(vec![AlertDestination::Slack]),
        alert_types: vec![
            AlertType::UserSpendThresholds,
            AlertType::UserSpendAnomalies,
        ],
        alerting_args: args(),
        hours_in_a_day: 24,
        cache,
        sender,
        source,
        clock: FakeClock {
            local: date("1999-12-31"),
            utc: date("2026-01-15"),
        },
    }
}
