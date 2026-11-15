use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;

use crate::generic_guardrail_contracts::{GuardrailInputType, GuardrailMessage};
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::hiddenlayer::evaluation::error::{EvaluationError, as_guardrail_message, blocked};
use crate::hiddenlayer::evaluation::text_part;
use crate::hiddenlayer::evaluation::{ACTION_BLOCK, ACTION_REDACT, BLOCK_MESSAGE, VIOLATED_POLICY};

// LiteLLM's _HiddenlayerResponse, the typed read of the v1 interaction body
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct InteractionBody {
    pub(crate) evaluation: Option<Evaluation>,
    pub(crate) analysis: Option<Vec<AnalysisEntry>>,
    pub(crate) modified_data: Option<ModifiedData>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Evaluation {
    pub(crate) action: Option<String>,
    pub(crate) threat_level: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct AnalysisEntry {
    pub(crate) name: Option<String>,
    pub(crate) detected: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ModifiedData {
    pub(crate) input: Option<ModifiedSide>,
    pub(crate) output: Option<ModifiedSide>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ModifiedSide {
    pub(crate) messages: Option<Vec<GuardrailValue>>,
}

// LiteLLM's _interaction_body, the typed decode of the v1 interaction reply
pub(crate) fn interaction_body(body: &[u8]) -> Result<InteractionBody, EvaluationError> {
    serde_json::from_slice(body).map_err(|source| EvaluationError::Decode { source })
}

pub(crate) fn apply_result(
    inputs: GuardrailApiInputs,
    result: &InteractionBody,
    input_type: GuardrailInputType,
) -> Result<GuardrailApiInputs, EvaluationError> {
    let action = result
        .evaluation
        .as_ref()
        .and_then(|evaluation| evaluation.action.as_deref());
    match action {
        Some(ACTION_BLOCK) => Err(block_failure(result)),
        Some(ACTION_REDACT) => redacted(inputs, result, input_type),
        _ => Ok(inputs),
    }
}

fn redacted(
    mut inputs: GuardrailApiInputs,
    result: &InteractionBody,
    input_type: GuardrailInputType,
) -> Result<GuardrailApiInputs, EvaluationError> {
    let Some(modified) = result.modified_data.as_ref() else {
        return Ok(inputs);
    };
    let side = match input_type {
        GuardrailInputType::Request => modified.input.as_ref(),
        GuardrailInputType::Response => modified.output.as_ref(),
    };
    let Some(messages) = side
        .and_then(|side| side.messages.as_deref())
        .filter(|messages| !messages.is_empty())
    else {
        return Ok(inputs);
    };
    let Some(last) = messages.last() else {
        return Ok(inputs);
    };
    inputs.texts = Some(redacted_texts(last));
    if input_type == GuardrailInputType::Request {
        inputs.structured_messages = Some(
            messages
                .iter()
                .map(as_guardrail_message)
                .collect::<Result<Vec<GuardrailMessage>, EvaluationError>>()?,
        );
    }
    Ok(inputs)
}

fn redacted_texts(message: &GuardrailValue) -> Vec<String> {
    let GuardrailValue::Object(fields) = message else {
        return vec![String::new()];
    };
    match fields.get("content") {
        Some(GuardrailValue::Array(parts)) => {
            let texts: Vec<String> = parts.iter().filter_map(text_part).collect();
            if texts.is_empty() {
                vec![String::new()]
            } else {
                texts
            }
        }
        Some(GuardrailValue::Text(text)) => vec![text.clone()],
        _ => vec![String::new()],
    }
}

fn block_failure(result: &InteractionBody) -> EvaluationError {
    blocked(vec![
        ("error", GuardrailValue::Text(VIOLATED_POLICY.to_owned())),
        (
            "hiddenlayer_guardrail_response",
            GuardrailValue::Text(BLOCK_MESSAGE.to_owned()),
        ),
        (
            "block_reasons",
            GuardrailValue::Array(block_reasons(result)),
        ),
        ("threat_level", threat_level(result)),
    ])
}

fn block_reasons(result: &InteractionBody) -> Vec<GuardrailValue> {
    result
        .analysis
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|entry| entry.detected == Some(true))
        .map(|entry| {
            GuardrailValue::Text(entry.name.clone().unwrap_or_else(|| "unknown".to_owned()))
        })
        .collect()
}

fn threat_level(result: &InteractionBody) -> GuardrailValue {
    match result
        .evaluation
        .as_ref()
        .and_then(|evaluation| evaluation.threat_level.clone())
    {
        Some(level) => GuardrailValue::Text(level),
        None => GuardrailValue::Null,
    }
}
