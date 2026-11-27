#![cfg(test)]

use chrono::{DateTime, Utc};

use super::{KeyBudgetRequest, set_key_budget_metrics_after_api_request};
use crate::prometheus::budget_lookups::records::KeyRecord;
use crate::prometheus::budget_lookups::snapshots::KeyBudgetSnapshot;
use crate::prometheus::budget_lookups::test_support::{
    Answer, KEY_GAUGES, LookupCall, RecordingLookups, all_samples, capture_logs, decimal,
    default_tracking, entity_gauges, hours_row, infinity_row, row, source, time,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const LABELS: [&str; 2] = ["hashed_api_key", "api_key_alias"];

fn request<'request>(
    key: Option<&'request str>,
    alias: Option<&'request str>,
    budgets: Option<(&str, &str)>,
    cost: &str,
    carried: Option<&'request KeyBudgetSnapshot>,
) -> KeyBudgetRequest<'request> {
    KeyBudgetRequest {
        user_api_key: key,
        user_api_key_alias: alias,
        response_cost: decimal(cost),
        key_max_budget: budgets.map(|(_, max_budget)| decimal(max_budget)),
        key_spend: budgets.map(|(spend, _)| decimal(spend)),
        carried,
    }
}

fn snapshot(reset_at: Option<DateTime<Utc>>) -> KeyBudgetSnapshot {
    KeyBudgetSnapshot {
        budget_reset_at: reset_at,
    }
}

#[tokio::test]
async fn a_carried_snapshot_uses_its_reset() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(KEY_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let reset_at = time("2026-01-06T00:00:00Z");
    let carried = snapshot(Some(reset_at));
    let key_request = request(
        Some("key-1"),
        Some("Key Alpha"),
        Some(("0.5", "10")),
        "0.25",
        Some(&carried),
    );
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &key_request,
    )
    .await
    .unwrap();
    let labels = [("hashed_api_key", "key-1"), ("api_key_alias", "Key Alpha")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(KEY_GAUGES[0], &labels, "9.25"),
            row(KEY_GAUGES[1], &labels, "10"),
            hours_row(KEY_GAUGES[2], &labels, reset_at, now),
        ]
    );
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn the_cache_only_fetch_reads_the_reset() {
    let settings = PrometheusLabelSettings::with_defaults();
    let reset_at = time("2026-01-06T00:00:00Z");
    let lookups = RecordingLookups::default().with_key(Answer::Found(KeyRecord {
        budget_reset_at: Some(reset_at),
    }));
    let gauges = entity_gauges(KEY_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let key_request = request(Some("key-1"), Some("Key Alpha"), None, "0.5", None);
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &key_request,
    )
    .await
    .unwrap();
    let labels = [("hashed_api_key", "key-1"), ("api_key_alias", "Key Alpha")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            infinity_row(KEY_GAUGES[0], &labels),
            hours_row(KEY_GAUGES[2], &labels, reset_at, now),
        ]
    );
    assert_eq!(
        lookups.calls(),
        vec![LookupCall::KeyInfo {
            hashed_token: "key-1".to_owned(),
            check_cache_only: true,
        }]
    );
}

#[tokio::test]
async fn a_fetched_key_without_a_reset_emits_no_hours() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default().with_key(Answer::Found(KeyRecord {
        budget_reset_at: None,
    }));
    let gauges = entity_gauges(KEY_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let key_request = request(Some("key-1"), None, Some(("0", "10")), "0.5", None);
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &key_request,
    )
    .await
    .unwrap();
    let labels = [("hashed_api_key", "key-1"), ("api_key_alias", "")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(KEY_GAUGES[0], &labels, "9.5"),
            row(KEY_GAUGES[1], &labels, "10"),
        ]
    );
}

#[tokio::test]
async fn a_cache_miss_keeps_the_partial_key_and_logs_it() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (logs, _guard) = capture_logs();
    let lookups = RecordingLookups::default().with_key(Answer::Failed("the key cache read failed"));
    let gauges = entity_gauges(KEY_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let key_request = request(
        Some("key-1"),
        Some("Key Alpha"),
        Some(("1", "10")),
        "0.25",
        None,
    );
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &key_request,
    )
    .await
    .unwrap();
    let labels = [("hashed_api_key", "key-1"), ("api_key_alias", "Key Alpha")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(KEY_GAUGES[0], &labels, "8.75"),
            row(KEY_GAUGES[1], &labels, "10"),
        ]
    );
    assert!(
        logs.text()
            .contains("Error getting key info: the key cache read failed")
    );
}

#[tokio::test]
async fn no_key_skips_before_the_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(KEY_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let key_request = request(None, Some("Key Alpha"), Some(("1", "10")), "0.25", None);
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &key_request,
    )
    .await
    .unwrap();
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_noop_remaining_gauge_skips_before_the_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(KEY_GAUGES, &LABELS, true);
    let now = time("2026-01-01T00:00:00Z");
    let key_request = request(
        Some("key-1"),
        Some("Key Alpha"),
        Some(("1", "10")),
        "0.25",
        None,
    );
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &key_request,
    )
    .await
    .unwrap();
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}
