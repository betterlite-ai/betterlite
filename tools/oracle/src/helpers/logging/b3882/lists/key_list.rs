use betterlite_logging::prometheus::budget_gauges::inputs::KeyBudgetGaugeInput;
use betterlite_logging::prometheus::budget_gauges::lists::key_list::{
    KeyListEntry, set_key_list_budget_metrics,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::gauges::entity_gauges;
use super::super::input::{Input, KeyListEntryCall};
use super::super::output::samples_value;
use super::super::probe::ProbeLog;
use super::super::support::{
    KEY_HOURS, KEY_MAX, KEY_REMAINING, SHAPE_ERROR_MESSAGE, helper_error, label_source,
};

impl KeyListEntryCall {
    fn entry(&self) -> KeyListEntry {
        match self {
            Self::Record {
                token,
                key_alias,
                spend,
                max_budget,
                budget_reset_at,
            } => KeyListEntry::Record(KeyBudgetGaugeInput {
                token: token.clone(),
                key_alias: key_alias.clone(),
                spend: *spend,
                max_budget: *max_budget,
                budget_reset_at: *budget_reset_at,
            }),
            Self::Token { token } => KeyListEntry::Token(token.clone()),
        }
    }
}

pub(crate) fn key_list_value(input: &Input) -> HelperResult {
    let Input::KeyList {
        entries,
        now,
        labels_by_metric,
    } = input
    else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let log = ProbeLog::default();
    let gauges = entity_gauges(KEY_REMAINING, KEY_MAX, KEY_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let calls: Vec<_> = entries.iter().map(KeyListEntryCall::entry).collect();
    set_key_list_budget_metrics(&gauges, &calls, label_source(&settings, false), *now)
        .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
    Ok(samples_value(&log.drain()))
}
