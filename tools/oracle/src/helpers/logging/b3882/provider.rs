use betterlite_logging::prometheus::budget_gauges::gauge::GaugeField;
use betterlite_logging::prometheus::budget_gauges::small_gauges::track_provider_budget;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName;

use super::input::Input;
use super::output::samples_value;
use super::probe::{ProbeGauge, ProbeLog};
use super::support::{PROVIDER_REMAINING, SHAPE_ERROR_MESSAGE, helper_error};

pub(super) fn provider_value(input: &Input) -> HelperResult {
    let Input::Provider {
        provider,
        spend,
        budget_limit,
    } = input
    else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let log = ProbeLog::default();
    let gauge = GaugeField {
        metric_name: PROVIDER_REMAINING,
        supported_labels: vec![UserApiKeyLabelName::ApiProvider.as_str().to_owned()],
        collector: ProbeGauge {
            metric: PROVIDER_REMAINING,
            labels: Vec::new(),
            log: log.clone(),
        },
    };
    track_provider_budget(&gauge, provider, *spend, *budget_limit);
    Ok(samples_value(&log.drain()))
}
