use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use crate::content_filter::detection_metrics::get_patterns_checked_count;
use crate::content_filter::detection_metrics::get_policy_templates;
use crate::content_filter::guardrail_tracing::GuardrailTraceInput;
use crate::content_filter::guardrail_tracing::GuardrailTracingDetail;
use crate::content_filter::guardrail_tracing::log_guardrail_information;
use crate::content_filter::risk_scoring::GuardrailStatus;
use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_records::error::GuardrailRecordError;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::GuardrailInvocationRecord;
use crate::guardrail_records::record::GuardrailRecordContext;
use crate::guardrail_records::record::GuardrailRecordInput;
use crate::guardrail_records::record::GuardrailSpanSink;
use crate::guardrail_records::record::GuardrailStatus as RecordStatus;
use crate::guardrail_records::record::GuardrailTracingDetail as RecordTracingDetail;

use super::ContentFilterGuardrail;

pub(super) struct RecordTrace<'trace> {
    pub(super) detections: &'trace [ContentFilterDetection],
    pub(super) masked_entity_count: &'trace BTreeMap<String, usize>,
    pub(super) status: GuardrailStatus,
    pub(super) exception: Option<&'trace str>,
    pub(super) started: SystemTime,
}

struct RecordTiming {
    start_time: f64,
    end_time: f64,
    duration: f64,
}

impl RecordTiming {
    fn ending_at(started: SystemTime, finished: SystemTime) -> Self {
        Self {
            start_time: epoch_seconds(started),
            end_time: epoch_seconds(finished),
            duration: finished
                .duration_since(started)
                .unwrap_or_default()
                .as_secs_f64(),
        }
    }
}

struct NoSpanSink;

impl GuardrailSpanSink for NoSpanSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

const NO_SPAN_SINK: NoSpanSink = NoSpanSink;

impl<R> ContentFilterGuardrail<R> {
    pub(super) fn write_record(
        &self,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        log_target: Option<&mut GuardrailLogTarget>,
        trace: &RecordTrace<'_>,
    ) -> Result<(), GuardrailRecordError> {
        let timing = RecordTiming::ending_at(trace.started, SystemTime::now());
        let detail = self.log_record(trace, &timing);
        let input = GuardrailRecordInput {
            guardrail_response: record_response(trace)?,
            guardrail_status: record_status(trace.status),
            start_time: Some(timing.start_time),
            end_time: Some(timing.end_time),
            duration: Some(timing.duration),
            masked_entity_count: Some(masked_counts(trace.masked_entity_count)),
            guardrail_provider: Some(self.guardrail_provider.clone()),
            event_type: None,
            tracing_detail: Some(record_tracing_detail(detail)?),
        };
        let mut context = GuardrailRecordContext {
            base: &self.base,
            request_data,
            log_target,
            sink: &NO_SPAN_SINK,
        };
        add_guardrail_record(&mut context, input)
    }

    fn log_record(&self, trace: &RecordTrace<'_>, timing: &RecordTiming) -> GuardrailTracingDetail {
        let mut records = Vec::new();
        log_guardrail_information(
            &mut records,
            &GuardrailTraceInput {
                guardrail_provider: &self.guardrail_provider,
                guardrail_name: self
                    .base
                    .settings
                    .guardrail_name
                    .as_deref()
                    .unwrap_or_default(),
                guardrail_id: self.config_guardrail_id.as_deref(),
                policy_template: self.policy_template(),
                detections: trace.detections,
                status: trace.status,
                start_time: timing.start_time,
                end_time: timing.end_time,
                duration: Some(timing.duration),
                masked_entity_count: trace.masked_entity_count,
                patterns_checked: self.patterns_checked(),
                exception_str: trace.exception,
            },
        )
    }

    // LiteLLM's config_policy_template or the descriptions of the loaded categories
    pub(super) fn policy_template(&self) -> Option<String> {
        self.config_policy_template
            .clone()
            .or_else(|| category_policy_template(&self.loaded_categories))
    }

    fn patterns_checked(&self) -> usize {
        get_patterns_checked_count(
            self.compiled_patterns.len(),
            self.blocked_words.len(),
            self.loaded_categories.category_keywords.len(),
            self.loaded_categories.always_block_category_keywords.len(),
        )
    }
}

// LiteLLM's _log_guardrail_information puts the exception text in place of the detections once the run failed
fn record_response(trace: &RecordTrace<'_>) -> Result<GuardrailValue, GuardrailRecordError> {
    let Some(text) = trace.exception else {
        return to_guardrail_value(trace.detections);
    };
    Ok(GuardrailValue::Text(text.to_owned()))
}

fn masked_counts(counts: &BTreeMap<String, usize>) -> BTreeMap<String, i64> {
    counts
        .iter()
        .map(|(name, count)| (name.clone(), i64::try_from(*count).unwrap_or(i64::MAX)))
        .collect()
}

const fn record_status(status: GuardrailStatus) -> RecordStatus {
    match status {
        GuardrailStatus::Success => RecordStatus::Success,
        GuardrailStatus::GuardrailIntervened => RecordStatus::GuardrailIntervened,
        GuardrailStatus::GuardrailFailedToRespond => RecordStatus::GuardrailFailedToRespond,
    }
}

fn record_tracing_detail(
    detail: GuardrailTracingDetail,
) -> Result<RecordTracingDetail, GuardrailRecordError> {
    Ok(RecordTracingDetail {
        guardrail_id: Some(detail.guardrail_id),
        policy_template: detail.policy_template,
        detection_method: detail.detection_method,
        confidence_score: detail.confidence_score,
        classification: detail
            .classification
            .as_ref()
            .map(to_guardrail_value)
            .transpose()?,
        match_details: detail
            .match_details
            .as_ref()
            .map(to_guardrail_value)
            .transpose()?,
        patterns_checked: Some(i64::try_from(detail.patterns_checked).unwrap_or(i64::MAX)),
        alert_recipients: None,
        risk_score: Some(detail.risk_score),
        violation_categories: None,
        guardrail_action: None,
        guardrail_usage: None,
        guardrail_cost: None,
        guardrail_cost_by_unit: None,
        guardrail_cost_in_spend: None,
    })
}

fn to_guardrail_value<T>(value: &T) -> Result<GuardrailValue, GuardrailRecordError>
where
    T: Serialize + ?Sized,
{
    let json =
        serde_json::to_value(value).map_err(|source| GuardrailRecordError::Json { source })?;
    serde_json::from_value(json).map_err(|source| GuardrailRecordError::Json { source })
}

fn category_policy_template(categories: &CategoryLoadResult) -> Option<String> {
    let templates = categories
        .loaded_categories
        .values()
        .map(|category| {
            (
                Some(category.description.clone()),
                category.category_name.clone(),
            )
        })
        .collect::<Vec<_>>();
    get_policy_templates(&templates)
}

fn epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
