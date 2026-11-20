use std::collections::BTreeMap;

use betterlite_logging::prometheus::budget_gauges::gauge::{EntityBudgetGauges, GaugeField};
use betterlite_oracle_helper_api::error::HelperError;

use super::probe::{ProbeGauge, ProbeLog};
use super::support::field;

pub(super) fn entity_gauges(
    remaining: &'static str,
    max: &'static str,
    hours: &'static str,
    labels: &BTreeMap<String, Vec<String>>,
    log: &ProbeLog,
) -> Result<EntityBudgetGauges<ProbeGauge>, Box<HelperError>> {
    Ok(EntityBudgetGauges {
        remaining: field(remaining, labels, log)?,
        max: field(max, labels, log)?,
        remaining_hours: field(hours, labels, log)?,
    })
}
