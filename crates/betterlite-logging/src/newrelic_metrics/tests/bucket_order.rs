use super::support::{attribute_at, envelope, float_count, metric_names, record};
use crate::newrelic_metrics::records::NewRelicMetricRecord;
use crate::newrelic_metrics::wire::{
    METRIC_REQUESTS, METRIC_TEAM_MAX_BUDGET, METRIC_TEAM_REMAINING_BUDGET,
};

const WINDOW_START_MS: u64 = 1_700_000_000_000;
const WINDOW_END_MS: u64 = 1_700_000_001_500;

fn record_for_model(model: &str) -> NewRelicMetricRecord {
    let mut record = record();
    record.model = model.to_owned();
    record.team_max_budget = None;
    record
}

#[test]
fn buckets_keep_the_order_they_first_appeared_in() {
    let envelope = envelope(
        &[
            record_for_model("gpt-4o"),
            record_for_model("gpt-4o-mini"),
            record_for_model("gpt-4o"),
        ],
        WINDOW_START_MS,
        WINDOW_END_MS,
    );
    assert_eq!(metric_names(&envelope).len(), 12);
    assert_eq!(attribute_at(&envelope, 0, "model"), "gpt-4o");
    assert_eq!(attribute_at(&envelope, 6, "model"), "gpt-4o-mini");
    assert_eq!(
        float_count(&envelope, METRIC_REQUESTS).to_bits(),
        2.0_f64.to_bits()
    );
}

#[test]
fn team_gauges_keep_the_order_each_team_first_appeared_in() {
    let mut first = record();
    first.team_id = "team-b".to_owned();
    let mut second = record();
    second.team_id = "team-a".to_owned();
    second.model = "gpt-4o-mini".to_owned();
    let envelope = envelope(&[first, second], WINDOW_START_MS, WINDOW_END_MS);
    assert_eq!(metric_names(&envelope).len(), 16);
    assert_eq!(attribute_at(&envelope, 12, "team_id"), "team-b");
    assert_eq!(attribute_at(&envelope, 14, "team_id"), "team-a");
    assert_eq!(metric_names(&envelope)[12], METRIC_TEAM_MAX_BUDGET);
    assert_eq!(metric_names(&envelope)[13], METRIC_TEAM_REMAINING_BUDGET);
}
