use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;

use crate::generic_guardrail_contracts::{GuardrailInputType, GuardrailMessage};
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::hiddenlayer::evaluation::error::{EvaluationError, blocked};
use crate::hiddenlayer::evaluation::{
    ACTION_BLOCK, ACTION_REDACT, BLOCK_MESSAGE, VIOLATED_POLICY, as_guardrail_message, text_part,
};

const CONTENT: &str = "content";

// LiteLLM's _HiddenlayerResponse, the typed read of the v1 interaction body
#[derive(Debug, Default, Deserialize)]
pub(crate) struct InteractionBody {
    pub(crate) evaluation: Option<Evaluation>,
    #[serde(default)]
    pub(crate) analysis: Vec<AnalysisEntry>,
    pub(crate) modified_data: Option<ModifiedData>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Evaluation {
    pub(crate) action: Option<String>,
    pub(crate) threat_level: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct AnalysisEntry {
    pub(crate) name: Option<String>,
    pub(crate) detected: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ModifiedData {
    pub(crate) input: Option<ModifiedSide>,
    pub(crate) output: Option<ModifiedSide>,
}

// LiteLLM's _HiddenlayerModifiedSide, whose messages key is required
#[derive(Debug, Deserialize)]
pub(crate) struct ModifiedSide {
    pub(crate) messages: Vec<GuardrailValue>,
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
    let selected = match input_type {
        GuardrailInputType::Request => modified.input.as_ref(),
        GuardrailInputType::Response => modified.output.as_ref(),
    };
    let Some(side) = selected else {
        return Ok(inputs);
    };
    let messages = &side.messages;
    let Some(last) = messages.last() else {
        return Err(EvaluationError::Shape {
            field: "modified_data.messages",
        });
    };
    inputs.texts = Some(redacted_texts(last)?);
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

// LiteLLM reads the last message's content, and fails when the message has none
fn redacted_texts(message: &GuardrailValue) -> Result<Vec<String>, EvaluationError> {
    let GuardrailValue::Object(fields) = message else {
        return Err(EvaluationError::Shape {
            field: "modified_data.messages.content",
        });
    };
    match fields.get(CONTENT) {
        Some(GuardrailValue::Array(parts)) => {
            let texts: Vec<String> = parts.iter().filter_map(text_part).collect();
            if texts.is_empty() {
                Ok(vec![String::new()])
            } else {
                Ok(texts)
            }
        }
        Some(GuardrailValue::Text(text)) => Ok(vec![text.clone()]),
        _ => Err(EvaluationError::Shape {
            field: "modified_data.messages.content",
        }),
    }
}

fn block_failure(result: &InteractionBody) -> EvaluationError {
    let threat_level = result
        .evaluation
        .as_ref()
        .and_then(|evaluation| evaluation.threat_level.clone())
        .map_or(GuardrailValue::Null, GuardrailValue::Text);
    let reasons = result
        .analysis
        .iter()
        .filter(|entry| entry.detected == Some(true))
        .map(|entry| {
            GuardrailValue::Text(entry.name.clone().unwrap_or_else(|| "unknown".to_owned()))
        })
        .collect();
    blocked(vec![
        ("error", GuardrailValue::Text(VIOLATED_POLICY.to_owned())),
        (
            "hiddenlayer_guardrail_response",
            GuardrailValue::Text(BLOCK_MESSAGE.to_owned()),
        ),
        ("block_reasons", GuardrailValue::Array(reasons)),
        ("threat_level", threat_level),
    ])
}
