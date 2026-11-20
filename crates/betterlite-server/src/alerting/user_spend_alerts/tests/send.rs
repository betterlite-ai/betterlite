#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::AlertLevel;
use betterlite_logging::alert_dispatch::destination::AlertDestination;
use tracing::subscriber::{DefaultGuard, set_default};

use super::support::{
    CacheWrite, FakeCache, FakeSender, FakeSource, date, decimal, reporting, row,
};

const DAILY_MESSAGE: &str = "User Daily Spend Threshold Crossed:\nUser: `user-1`\nSpend Today: `$12.50`\nDaily Threshold: `$10.00`";

const MONTHLY_MESSAGE: &str = "User Monthly Spend Threshold Crossed:\nUser: `user-1`\nSpend This Month: `$40.00`\nMonthly Threshold: `$25.00`";

const ANOMALY_MESSAGE: &str = "User Spend Anomaly Detected:\nUser: `user-1`\nSpend Today: `$12.50`\nDaily Average (last 7 days): `$3.00`\nTrigger: spend above `3.0x` the daily average (minimum `$10.00`)";

const SPENDER: &str = "user-1";

struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("the captured logs were poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured(Arc::default());
    let writer = Arc::clone(&captured.0);
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || Captured(Arc::clone(&writer)))
        .with_max_level(tracing::Level::ERROR)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

fn spender() -> FakeSource {
    FakeSource::returning(vec![row(SPENDER, 12.5, 40.0, 21.0)])
}

#[tokio::test]
async fn send_returns_without_alerting() {
    let source = spender();
    let queried = source.queried();
    let mut alerts = reporting(FakeCache::default(), FakeSender::default(), source);
    alerts.alerting = None;
    alerts.send_user_spend_alerts(true).await;
    assert!(queried.lock().unwrap().is_empty());
    assert!(alerts.sender.alerts().is_empty());
    assert!(alerts.cache.written().is_empty());
}

#[tokio::test]
async fn send_returns_without_slack() {
    let source = spender();
    let queried = source.queried();
    let mut alerts = reporting(FakeCache::default(), FakeSender::default(), source);
    alerts.alerting = Some(vec![AlertDestination::Email]);
    alerts.send_user_spend_alerts(true).await;
    assert!(queried.lock().unwrap().is_empty());
    assert!(alerts.sender.alerts().is_empty());
}

#[tokio::test]
async fn send_returns_without_an_enabled_type() {
    let source = spender();
    let queried = source.queried();
    let mut alerts = reporting(FakeCache::default(), FakeSender::default(), source);
    alerts.alert_types = Vec::new();
    alerts.send_user_spend_alerts(true).await;
    assert!(queried.lock().unwrap().is_empty());
    assert!(alerts.sender.alerts().is_empty());
}

#[tokio::test]
async fn send_returns_without_a_database() {
    let source = spender();
    let queried = source.queried();
    let alerts = reporting(FakeCache::default(), FakeSender::default(), source);
    alerts.send_user_spend_alerts(false).await;
    assert!(queried.lock().unwrap().is_empty());
    assert!(alerts.sender.alerts().is_empty());
    assert!(alerts.cache.written().is_empty());
}

#[tokio::test]
async fn send_uses_the_utc_date_and_the_baseline_days() {
    let source = spender();
    let queried = source.queried();
    let mut alerts = reporting(FakeCache::default(), FakeSender::default(), source);
    alerts.alerting_args.spend_anomaly_baseline_days = 3;
    alerts.send_user_spend_alerts(true).await;
    assert_eq!(
        queried.lock().unwrap().clone(),
        vec![(date("2026-01-15"), 3)]
    );
}

#[tokio::test]
async fn send_drops_events_whose_cache_key_is_set() {
    let alerts = reporting(
        FakeCache::seeded(&["user_spend_alert_daily_user-1_2026-01-15"]),
        FakeSender::default(),
        spender(),
    );
    alerts.send_user_spend_alerts(true).await;
    let sent = alerts.sender.alerts();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].alert_type, AlertType::UserSpendThresholds);
    assert_eq!(sent[0].message, MONTHLY_MESSAGE);
    assert_eq!(sent[1].alert_type, AlertType::UserSpendAnomalies);
}

#[tokio::test]
async fn send_returns_when_every_key_is_set() {
    let alerts = reporting(
        FakeCache::seeded(&[
            "user_spend_alert_daily_user-1_2026-01-15",
            "user_spend_alert_monthly_user-1_2026-01",
            "user_spend_alert_anomaly_user-1_2026-01-15",
        ]),
        FakeSender::default(),
        spender(),
    );
    alerts.send_user_spend_alerts(true).await;
    assert!(alerts.sender.alerts().is_empty());
    assert!(alerts.cache.written().is_empty());
}

#[tokio::test]
async fn send_groups_messages_by_type_at_high_with_empty_metadata() {
    let mut alerts = reporting(
        FakeCache::default(),
        FakeSender::default(),
        FakeSource::returning(vec![
            row("user-1", 12.5, 0.0, 0.0),
            row("user-2", 20.0, 0.0, 0.0),
        ]),
    );
    alerts.alerting_args.spend_anomaly_min_spend = decimal("1000");
    alerts.send_user_spend_alerts(true).await;
    let sent = alerts.sender.alerts();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].alert_type, AlertType::UserSpendThresholds);
    assert_eq!(sent[0].level, AlertLevel::High);
    assert_eq!(sent[0].metadata_len, 0);
    assert_eq!(sent[0].kwargs_len, 0);
    assert_eq!(
        sent[0].message,
        format!(
            "{DAILY_MESSAGE}\n\nUser Daily Spend Threshold Crossed:\nUser: `user-2`\nSpend Today: `$20.00`\nDaily Threshold: `$10.00`"
        )
    );
}

#[tokio::test]
async fn send_sends_thresholds_then_anomalies() {
    let alerts = reporting(FakeCache::default(), FakeSender::default(), spender());
    alerts.send_user_spend_alerts(true).await;
    let sent = alerts.sender.alerts();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].alert_type, AlertType::UserSpendThresholds);
    assert_eq!(
        sent[0].message,
        format!("{DAILY_MESSAGE}\n\n{MONTHLY_MESSAGE}")
    );
    assert_eq!(sent[1].alert_type, AlertType::UserSpendAnomalies);
    assert_eq!(sent[1].message, ANOMALY_MESSAGE);
    assert_eq!(sent[1].level, AlertLevel::High);
}

#[tokio::test]
async fn send_runs_with_only_the_thresholds_enabled() {
    let mut alerts = reporting(FakeCache::default(), FakeSender::default(), spender());
    alerts.alert_types = vec![AlertType::UserSpendThresholds];
    alerts.send_user_spend_alerts(true).await;
    let sent = alerts.sender.alerts();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].alert_type, AlertType::UserSpendThresholds);
    assert_eq!(
        sent[0].message,
        format!("{DAILY_MESSAGE}\n\n{MONTHLY_MESSAGE}")
    );
    assert_eq!(alerts.cache.written().len(), 2);
}

#[tokio::test]
async fn send_runs_with_only_the_anomalies_enabled() {
    let mut alerts = reporting(FakeCache::default(), FakeSender::default(), spender());
    alerts.alert_types = vec![AlertType::UserSpendAnomalies];
    alerts.send_user_spend_alerts(true).await;
    let sent = alerts.sender.alerts();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].alert_type, AlertType::UserSpendAnomalies);
    assert_eq!(sent[0].message, ANOMALY_MESSAGE);
    assert_eq!(
        alerts.cache.written(),
        vec![CacheWrite {
            key: "user_spend_alert_anomaly_user-1_2026-01-15".to_owned(),
            value: "SENT".to_owned(),
            ttl_seconds: 86_400,
        }]
    );
}

#[tokio::test]
async fn send_writes_each_key_after_its_type_was_sent() {
    let alerts = reporting(FakeCache::default(), FakeSender::default(), spender());
    alerts.send_user_spend_alerts(true).await;
    assert_eq!(
        alerts.cache.written(),
        vec![
            CacheWrite {
                key: "user_spend_alert_daily_user-1_2026-01-15".to_owned(),
                value: "SENT".to_owned(),
                ttl_seconds: 86_400,
            },
            CacheWrite {
                key: "user_spend_alert_monthly_user-1_2026-01".to_owned(),
                value: "SENT".to_owned(),
                ttl_seconds: 2_764_800,
            },
            CacheWrite {
                key: "user_spend_alert_anomaly_user-1_2026-01-15".to_owned(),
                value: "SENT".to_owned(),
                ttl_seconds: 86_400,
            },
        ]
    );
}

#[tokio::test]
async fn send_stops_when_the_threshold_send_fails() {
    let (logs, _guard) = capture_logs();
    let alerts = reporting(
        FakeCache::default(),
        FakeSender::failing_on(AlertType::UserSpendThresholds),
        spender(),
    );
    alerts.send_user_spend_alerts(true).await;
    assert!(logs.text().contains("Error sending user spend alerts"));
    assert!(alerts.sender.alerts().is_empty());
    assert!(alerts.cache.written().is_empty());
}

#[tokio::test]
async fn send_keeps_threshold_keys_when_the_anomaly_send_fails() {
    let (logs, _guard) = capture_logs();
    let alerts = reporting(
        FakeCache::default(),
        FakeSender::failing_on(AlertType::UserSpendAnomalies),
        spender(),
    );
    alerts.send_user_spend_alerts(true).await;
    assert!(logs.text().contains("Error sending user spend alerts"));
    let written: Vec<String> = alerts
        .cache
        .written()
        .iter()
        .map(|write| write.key.clone())
        .collect();
    assert_eq!(
        written,
        vec![
            "user_spend_alert_daily_user-1_2026-01-15".to_owned(),
            "user_spend_alert_monthly_user-1_2026-01".to_owned(),
        ]
    );
}

#[tokio::test]
async fn send_logs_a_source_error_and_sends_nothing() {
    let (logs, _guard) = capture_logs();
    let alerts = reporting(
        FakeCache::default(),
        FakeSender::default(),
        FakeSource::failing(),
    );
    alerts.send_user_spend_alerts(true).await;
    assert!(logs.text().contains("Error sending user spend alerts"));
    assert!(alerts.sender.alerts().is_empty());
    assert!(alerts.cache.written().is_empty());
}
