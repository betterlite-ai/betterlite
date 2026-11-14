use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::content_filter_records::{
    BlockCodeExecGuardrailConfig, CodeBlockActionTaken, CodeBlockDetection,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use thiserror::Error;

use super::error_handling::block_failure;
use super::lifecycle::BlockCodeExecutionGuardrail;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_records::error::GuardrailRecordError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus, GuardrailTracingDetail,
};

pub type BlockCodeExecutionConfigModel = BlockCodeExecGuardrailConfig;

pub const GUARDRAIL_PROVIDER: &str = "block_code_execution";
pub const DETECTION_METHOD: &str = "fenced_code_block";
pub const UNKNOWN_LANGUAGE: &str = "unknown";

#[derive(Debug, Error)]
pub enum ApplyGuardrailError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error(transparent)]
    Record(#[from] GuardrailRecordError),
}

struct Invocation<'invocation> {
    guardrail_name: Option<String>,
    detections: &'invocation [CodeBlockDetection],
    exception_text: &'invocation str,
    status: GuardrailStatus,
    input_type: GuardrailInputType,
    started: SystemTime,
    finished: SystemTime,
}

pub fn apply_guardrail(
    guardrail: &BlockCodeExecutionGuardrail,
    inputs: GuardrailApiInputs,
    context: &mut GuardrailRecordContext<'_>,
    input_type: GuardrailInputType,
) -> Result<GuardrailApiInputs, ApplyGuardrailError> {
    let started = (guardrail.clock)();
    let mut detections: Vec<CodeBlockDetection> = Vec::new();
    let outcome = scan_texts(
        guardrail,
        inputs,
        &*context.request_data,
        input_type,
        &mut detections,
    );
    let finished = (guardrail.clock)();
    let status = match &outcome {
        Ok(_inputs) => GuardrailStatus::Success,
        Err(failure) => status_for_failure(failure),
    };
    let exception_text = match &outcome {
        Ok(_inputs) => String::new(),
        Err(failure) => failure.to_string(),
    };
    let invocation = Invocation {
        guardrail_name: context.base.settings.guardrail_name.clone(),
        detections: &detections,
        exception_text: &exception_text,
        status,
        input_type,
        started,
        finished,
    };
    let recorded = add_guardrail_record(context, invocation_record(&invocation));
    match (outcome, recorded) {
        (Ok(processed), Ok(())) => Ok(processed),
        (Err(failure), Ok(())) => Err(ApplyGuardrailError::Guardrail(failure)),
        (_result, Err(error)) => Err(ApplyGuardrailError::Record(error)),
    }
}

fn scan_texts(
    guardrail: &BlockCodeExecutionGuardrail,
    mut inputs: GuardrailApiInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    detections: &mut Vec<CodeBlockDetection>,
) -> Result<GuardrailApiInputs, GuardrailFailure> {
    if inputs.texts.as_ref().is_none_or(Vec::is_empty) {
        return Ok(inputs);
    }
    let texts = inputs.texts.take().unwrap_or_default();
    let mut processed = Vec::with_capacity(texts.len());
    for text in texts {
        let (new_text, should_raise) = guardrail.scanner.scan_text(&text, detections, input_type);
        processed.push(new_text);
        if should_raise {
            let language = detections
                .iter()
                .find(|detection| detection.action_taken == CodeBlockActionTaken::Block)
                .map_or(UNKNOWN_LANGUAGE, |detection| detection.language.as_str());
            return Err(block_failure(guardrail, language, input_type, request_data));
        }
    }
    inputs.texts = Some(processed);
    Ok(inputs)
}

#[must_use]
pub const fn status_for_failure(failure: &GuardrailFailure) -> GuardrailStatus {
    if failure.is_intervention() {
        GuardrailStatus::GuardrailIntervened
    } else {
        GuardrailStatus::GuardrailFailedToRespond
    }
}

fn invocation_record(invocation: &Invocation<'_>) -> GuardrailRecordInput {
    let response = guardrail_response(invocation);
    let confidence_score = invocation
        .detections
        .iter()
        .map(|detection| detection.confidence)
        .reduce(f64::max);
    GuardrailRecordInput {
        guardrail_response: response.clone(),
        guardrail_status: invocation.status,
        start_time: Some(
            invocation
                .started
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
        ),
        end_time: Some(
            invocation
                .finished
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
        ),
        duration: Some(
            invocation
                .finished
                .duration_since(invocation.started)
                .unwrap_or_default()
                .as_secs_f64(),
        ),
        masked_entity_count: None,
        guardrail_provider: Some(GUARDRAIL_PROVIDER.to_owned()),
        event_type: Some(event_type(invocation.input_type)),
        tracing_detail: Some(tracing_detail(invocation, response, confidence_score)),
    }
}

fn tracing_detail(
    invocation: &Invocation<'_>,
    response: GuardrailValue,
    confidence_score: Option<f64>,
) -> GuardrailTracingDetail {
    GuardrailTracingDetail {
        guardrail_id: invocation.guardrail_name.clone(),
        policy_template: None,
        detection_method: Some(DETECTION_METHOD.to_owned()),
        confidence_score,
        classification: None,
        match_details: Some(response),
        patterns_checked: None,
        alert_recipients: None,
        risk_score: None,
        violation_categories: None,
        guardrail_action: None,
        guardrail_usage: None,
        guardrail_cost: None,
        guardrail_cost_by_unit: None,
        guardrail_cost_in_spend: None,
    }
}

fn guardrail_response(invocation: &Invocation<'_>) -> GuardrailValue {
    if invocation.status != GuardrailStatus::Success && invocation.detections.is_empty() {
        return GuardrailValue::Text(invocation.exception_text.to_owned());
    }
    GuardrailValue::Array(invocation.detections.iter().map(detection_value).collect())
}

fn detection_value(detection: &CodeBlockDetection) -> GuardrailValue {
    let mut fields = BTreeMap::new();
    fields.insert(
        "type".to_owned(),
        GuardrailValue::Text(detection.r#type.clone()),
    );
    fields.insert(
        "language".to_owned(),
        GuardrailValue::Text(detection.language.clone()),
    );
    fields.insert(
        "confidence".to_owned(),
        GuardrailValue::Decimal(detection.confidence),
    );
    fields.insert(
        "action_taken".to_owned(),
        GuardrailValue::Text(action_name(detection.action_taken).to_owned()),
    );
    if let Some(evidence) = &detection.evidence {
        fields.insert(
            "evidence".to_owned(),
            GuardrailValue::Text(evidence.clone()),
        );
    }
    if let Some(snippet) = &detection.snippet {
        fields.insert("snippet".to_owned(), GuardrailValue::Text(snippet.clone()));
    }
    GuardrailValue::Object(fields)
}

const fn action_name(action: CodeBlockActionTaken) -> &'static str {
    match action {
        CodeBlockActionTaken::Block => "block",
        CodeBlockActionTaken::Allow => "allow",
        CodeBlockActionTaken::LogOnly => "log_only",
    }
}

const fn event_type(input_type: GuardrailInputType) -> GuardrailEventHooks {
    match input_type {
        GuardrailInputType::Request => GuardrailEventHooks::PreCall,
        GuardrailInputType::Response => GuardrailEventHooks::PostCall,
    }
}
