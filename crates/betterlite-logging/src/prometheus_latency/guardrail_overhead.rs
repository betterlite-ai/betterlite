use std::slice::from_ref;

use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::Collector;
use rust_decimal::Decimal;

use crate::fields::payload::records::GuardrailInformation;
use crate::prometheus_label_projection::prometheus_label_factory::prometheus_label_factory;

use super::error::OverheadRecordingError;
use super::labels::LabelInputs;
use super::latency_histograms::MetricHandle;

const ADDITIVE_GUARDRAIL_MODES: [&str; 2] = ["pre_call", "post_call"];
const MILLIS_PER_SECOND: Decimal = Decimal::ONE_THOUSAND;

// guardrail_information is typed as a list, but some guardrails assign a single record directly
#[derive(Clone, Copy, Debug)]
pub enum GuardrailRecords<'records> {
    Absent,
    Single(&'records GuardrailInformation),
    Multiple(&'records [GuardrailInformation]),
}

#[derive(Clone, Copy, Debug)]
pub struct OverheadObservation<'observation> {
    pub overhead_ms: Option<Decimal>,
    pub guardrail_records: GuardrailRecords<'observation>,
}

#[must_use]
pub fn guardrail_is_additive(record: &GuardrailInformation) -> bool {
    let names = mode_name_list(record.guardrail_mode.as_ref().and_then(Option::as_ref));
    !names.is_empty()
        && names
            .iter()
            .all(|name| ADDITIVE_GUARDRAIL_MODES.contains(name))
}

#[must_use]
pub fn guardrail_overhead_seconds(records: GuardrailRecords<'_>) -> Option<Decimal> {
    let entries: &[GuardrailInformation] = match records {
        GuardrailRecords::Absent => &[],
        GuardrailRecords::Single(record) => from_ref(record),
        GuardrailRecords::Multiple(list) => list,
    };
    let mut total = Decimal::ZERO;
    for record in entries
        .iter()
        .filter(|record| guardrail_is_additive(record))
    {
        let duration = record
            .duration
            .as_ref()
            .and_then(Option::as_ref)
            .map_or(Decimal::ZERO, |value| value.0);
        total = total.checked_add(duration)?;
    }
    Some(total)
}

pub fn set_guardrail_overhead_metric<Metric>(
    histogram: &MetricHandle<'_, Metric>,
    observation: &OverheadObservation<'_>,
    labels: &LabelInputs<'_>,
) -> Result<(), OverheadRecordingError>
where
    Metric: Collector,
{
    let guardrail_seconds = guardrail_overhead_seconds(observation.guardrail_records)
        .ok_or(OverheadRecordingError::Overflow)?;
    if observation.overhead_ms.is_none() && guardrail_seconds <= Decimal::ZERO {
        return Ok(());
    }
    let overhead_seconds = match observation.overhead_ms {
        Some(milliseconds) => milliseconds
            .checked_div(MILLIS_PER_SECOND)
            .ok_or(OverheadRecordingError::Overflow)?,
        None => Decimal::ZERO,
    };
    let sample = overhead_seconds
        .checked_add(guardrail_seconds)
        .ok_or(OverheadRecordingError::Overflow)?;
    let projected =
        prometheus_label_factory(histogram.supported_labels, labels.values, labels.source())?;
    histogram.observe(&projected, sample);
    Ok(())
}

fn mode_name_list(mode: Option<&Json>) -> Vec<&str> {
    match mode {
        Some(Json::Text(name)) => vec![name.as_str()],
        Some(Json::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                Json::Text(name) => Some(name.as_str()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) | None => Vec::new(),
    }
}

#[path = "tests/guardrail_overhead.rs"]
mod tests;
