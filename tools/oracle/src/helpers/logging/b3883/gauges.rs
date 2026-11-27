use std::collections::BTreeMap;

use betterlite_logging::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prometheus::collector_factory::collector::MetricLike;

use super::probe::{ProbeLog, RecordingGauge};
use super::support::field;

pub(super) fn entity_gauges(
    metrics: [&'static str; 3],
    labels: &BTreeMap<String, Vec<String>>,
    noop: bool,
    log: &ProbeLog,
) -> Result<EntityBudgetGauges<MetricLike<RecordingGauge>>, Box<HelperError>> {
    let [remaining, max, hours] = metrics;
    Ok(EntityBudgetGauges {
        remaining: field(remaining, labels, noop, log)?,
        max: field(max, labels, noop, log)?,
        remaining_hours: field(hours, labels, noop, log)?,
    })
}
