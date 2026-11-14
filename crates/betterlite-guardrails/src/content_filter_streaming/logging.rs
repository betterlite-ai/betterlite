use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use crate::content_filter::detection_metrics::{
    count_masked_entities, get_patterns_checked_count, get_policy_templates,
};
use crate::content_filter::guardrail_tracing::{
    GuardrailTraceInput, GuardrailTracingDetail, log_guardrail_information,
};
use crate::content_filter::risk_scoring::GuardrailStatus;
use crate::content_filter_actions::ContentFilterSource;
use crate::content_filter_streaming::error::StreamFilterError;
use crate::content_filter_streaming::hook::{StreamFilterCall, StreamFilterSettings, StreamScan};
use crate::content_filter_streaming::merge::merge_detections;
use crate::content_filter_streaming::state::{StreamedChoiceState, StreamedDetection};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_records::error::{GuardrailRecordError, GuardrailSpanError};
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
    GuardrailStatus as RecordStatus, GuardrailTracingDetail as RecordTracingDetail,
};

const PROVIDER: &str = "litellm_content_filter";

struct NoSpanSink;

impl GuardrailSpanSink for NoSpanSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

const NO_SPAN_SINK: NoSpanSink = NoSpanSink;

struct RecordParts {
    detections: Vec<ContentFilterDetection>,
    masked_entity_count: BTreeMap<String, usize>,
    masked_counts: BTreeMap<String, i64>,
    patterns_checked: usize,
    policy_template: Option<String>,
    started_seconds: f64,
    finished_seconds: f64,
    duration: f64,
}

pub(super) fn log_stream_record(
    settings: &StreamFilterSettings,
    scan: &StreamScan<'_, '_>,
    call: &mut StreamFilterCall<'_>,
    status: GuardrailStatus,
    started: SystemTime,
) -> Result<(), StreamFilterError> {
    let parts = record_parts(settings, scan, started);
    let record = invocation_record(settings, &parts, status)?;
    write_record(settings, call, record)
}

fn record_parts(
    settings: &StreamFilterSettings,
    scan: &StreamScan<'_, '_>,
    started: SystemTime,
) -> RecordParts {
    let mut detections = Vec::new();
    for state in scan.states.values() {
        detections.extend(merge_detections(&ordered_detections(state)));
    }
    let mut masked_entity_count = BTreeMap::new();
    count_masked_entities(&detections, &mut masked_entity_count);
    let mut masked_counts = BTreeMap::new();
    for (name, count) in &masked_entity_count {
        masked_counts.insert(name.clone(), i64::try_from(*count).unwrap_or(i64::MAX));
    }
    let patterns_checked = get_patterns_checked_count(
        scan.source.patterns.len(),
        scan.source.blocked_words.len(),
        scan.source.categories.category_keywords.len(),
        scan.source.categories.always_block_category_keywords.len(),
    );
    let finished = SystemTime::now();
    RecordParts {
        detections,
        masked_entity_count,
        masked_counts,
        patterns_checked,
        policy_template: policy_template(settings, scan.source),
        started_seconds: epoch_seconds(started),
        finished_seconds: epoch_seconds(finished),
        duration: scan.scan_time.as_secs_f64(),
    }
}

// a whole-buffer scan finds category checks first, then patterns and blocked words in config order
fn ordered_detections(state: &StreamedChoiceState) -> Vec<ContentFilterDetection> {
    let mut ordered = state
        .committed_detections
        .iter()
        .chain(&state.latest_detections)
        .cloned()
        .collect::<Vec<StreamedDetection>>();
    ordered.sort_by_key(|entry| (entry.check, entry.position));
    ordered.into_iter().map(|entry| entry.detection).collect()
}

fn policy_template(
    settings: &StreamFilterSettings,
    source: &ContentFilterSource<'_>,
) -> Option<String> {
    let templates = source
        .categories
        .loaded_categories
        .values()
        .map(|category| {
            (
                Some(category.description.clone()),
                category.category_name.clone(),
            )
        })
        .collect::<Vec<_>>();
    settings
        .policy_template
        .clone()
        .or_else(|| get_policy_templates(&templates))
}

fn invocation_record(
    settings: &StreamFilterSettings,
    parts: &RecordParts,
    status: GuardrailStatus,
) -> Result<GuardrailRecordInput, StreamFilterError> {
    let mut records = Vec::new();
    let detail = log_guardrail_information(
        &mut records,
        &GuardrailTraceInput {
            guardrail_provider: PROVIDER,
            guardrail_name: &settings.guardrail_name,
            guardrail_id: settings.guardrail_id.as_deref(),
            policy_template: parts.policy_template.clone(),
            detections: &parts.detections,
            status,
            start_time: parts.started_seconds,
            end_time: parts.finished_seconds,
            duration: Some(parts.duration),
            masked_entity_count: &parts.masked_entity_count,
            patterns_checked: parts.patterns_checked,
            exception_str: None,
        },
    );
    let tracing_detail = record_tracing_detail(detail)?;
    Ok(GuardrailRecordInput {
        guardrail_response: to_guardrail_value(&parts.detections)?,
        guardrail_status: record_status(status),
        start_time: Some(parts.started_seconds),
        end_time: Some(parts.finished_seconds),
        duration: Some(parts.duration),
        masked_entity_count: Some(parts.masked_counts.clone()),
        guardrail_provider: Some(PROVIDER.to_owned()),
        event_type: None,
        tracing_detail: Some(tracing_detail),
    })
}

fn record_tracing_detail(
    detail: GuardrailTracingDetail,
) -> Result<RecordTracingDetail, StreamFilterError> {
    let match_details = detail
        .match_details
        .as_ref()
        .map(to_guardrail_value)
        .transpose()?;
    let classification = detail
        .classification
        .as_ref()
        .map(to_guardrail_value)
        .transpose()?;
    Ok(RecordTracingDetail {
        guardrail_id: Some(detail.guardrail_id),
        policy_template: detail.policy_template,
        detection_method: detail.detection_method,
        confidence_score: detail.confidence_score,
        classification,
        match_details,
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

fn write_record(
    settings: &StreamFilterSettings,
    call: &mut StreamFilterCall<'_>,
    record: GuardrailRecordInput,
) -> Result<(), StreamFilterError> {
    let base = GuardrailBase::new(
        GuardrailSettings {
            guardrail_name: Some(settings.guardrail_name.clone()),
            supported_event_hooks: None,
            event_hook: Some(settings.event_hook.clone()),
            default_on: false,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: GuardrailSwitches::default(),
        },
        false,
    )?;
    let mut context = GuardrailRecordContext {
        base: &base,
        request_data: &mut *call.request_data,
        log_target: call.log_target.as_deref_mut(),
        sink: &NO_SPAN_SINK,
    };
    add_guardrail_record(&mut context, record)?;
    Ok(())
}

const fn record_status(status: GuardrailStatus) -> RecordStatus {
    match status {
        GuardrailStatus::Success => RecordStatus::Success,
        GuardrailStatus::GuardrailIntervened => RecordStatus::GuardrailIntervened,
        GuardrailStatus::GuardrailFailedToRespond => RecordStatus::GuardrailFailedToRespond,
    }
}

fn epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn to_guardrail_value<T>(value: &T) -> Result<GuardrailValue, StreamFilterError>
where
    T: Serialize,
{
    let json = serde_json::to_value(value)
        .map_err(|source| StreamFilterError::Record(GuardrailRecordError::Json { source }))?;
    serde_json::from_value(json)
        .map_err(|source| StreamFilterError::Record(GuardrailRecordError::Json { source }))
}
