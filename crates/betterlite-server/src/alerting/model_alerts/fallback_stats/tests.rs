#![cfg(test)]

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::AlertLevel;
use rstest::rstest;

use super::{FallbackMetricSource, FallbackStatsAlerts};
use crate::alerting::model_alerts::tests::support::{FakeSender, Journal, LogCapture};

#[derive(Clone)]
struct FakeSource {
    metric: Result<String, ()>,
    calls: Journal<()>,
}

impl FakeSource {
    fn returning(metric: &str) -> Self {
        Self {
            metric: Ok(metric.to_owned()),
            calls: Journal::default(),
        }
    }

    fn failing() -> Self {
        Self {
            metric: Err(()),
            calls: Journal::default(),
        }
    }
}

#[async_trait]
impl FallbackMetricSource for FakeSource {
    type Error = FakeSourceError;

    async fn fallback_metric(&self) -> Result<String, Self::Error> {
        self.calls.record(());
        self.metric.clone().map_err(|()| FakeSourceError)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("prometheus is down")]
struct FakeSourceError;

fn alerts(source: FakeSource, sender: FakeSender) -> FallbackStatsAlerts<FakeSource, FakeSender> {
    FallbackStatsAlerts { source, sender }
}

#[rstest]
#[case::text("Failed fallbacks: 3\nSuccessful fallback requests: 10")]
#[case::empty("")]
#[tokio::test]
async fn the_metric_text_is_sent_low_as_a_fallback_report(#[case] metric: &str) {
    let sender = FakeSender::default();
    let service = alerts(FakeSource::returning(metric), sender.clone());

    service.send_fallback_stats().await;

    let sent = sender.alerts.entries();
    let alert = sent.first().expect("one fallback report");
    assert_eq!(alert.message, format!("*Fallback Statistics:*\n{metric}"));
    assert_eq!(alert.level, AlertLevel::Low);
    assert_eq!(alert.alert_type, AlertType::FallbackReports);
    assert!(alert.user_info.is_none());
    assert!(alert.metadata.is_empty());
    assert!(alert.kwargs.is_empty());
}

#[tokio::test]
async fn a_prometheus_error_is_logged_and_swallowed() {
    let log = LogCapture::new();
    let source = FakeSource::failing();
    let sender = FakeSender::default();
    let service = alerts(source.clone(), sender.clone());

    service.send_fallback_stats().await;

    assert_eq!(source.calls.entries().len(), 1);
    assert!(sender.alerts.entries().is_empty());
    let text = log.text();
    assert!(text.contains("Error sending weekly spend report"));
    assert!(text.contains("prometheus is down"));
}

#[tokio::test]
async fn a_sender_error_is_logged_and_swallowed() {
    let log = LogCapture::new();
    let service = alerts(FakeSource::returning("fallbacks: 1"), FakeSender::failing());

    service.send_fallback_stats().await;

    let text = log.text();
    assert!(text.contains("Error sending weekly spend report"));
    assert!(text.contains("the fake sender failed"));
}
