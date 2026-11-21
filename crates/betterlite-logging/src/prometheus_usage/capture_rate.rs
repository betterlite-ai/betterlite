use betterlite_prometheus::collector_factory::collector::{Collector as _, MetricValue};
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use rust_decimal::Decimal;

use super::counters::{UsageCounters, label_pairs};
use super::error::PrometheusUsageError;
use super::metric::UsageMetric;
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelSource, prometheus_label_factory,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

pub fn set_spend_capture_rate<C>(
    counters: &C,
    api_provider: &str,
    capture_rate: Option<Decimal>,
    settings: &PrometheusLabelSettings,
    end_user_settings: EndUserCostTrackingSettings,
) -> Result<(), PrometheusUsageError<<C::Counter as RemovableMetric>::Error>>
where
    C: UsageCounters,
{
    let values = UserApiKeyLabelValues::from_entries(&[(
        "api_provider".to_owned(),
        LabelDumpValue::String(api_provider.to_owned()),
    )]);
    let labels = prometheus_label_factory(
        counters.supported_labels(UsageMetric::SpendCaptureRate),
        &values,
        LabelSource::Values {
            settings,
            end_user_settings,
        },
    )?;
    let amount = capture_rate.map_or(MetricValue::NotANumber, MetricValue::Finite);
    let gauge = counters.counter(UsageMetric::SpendCaptureRate);
    if labels.is_empty() {
        gauge.set(amount);
        return Ok(());
    }
    let pairs = label_pairs(&labels);
    gauge.labels(&pairs).set(amount);
    Ok(())
}
