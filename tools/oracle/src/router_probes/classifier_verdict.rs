use std::fmt::Display;

use betterlite_config::sections::tier_definitions as section;
use betterlite_models::decisions::Json;
use betterlite_router::classifier_verdict::call::{RequestKwargs, effective_logging_off};
use betterlite_router::classifier_verdict::diagnostics::classifier_reply_is_private;
use betterlite_router::classifier_verdict::error::ClassifierVerdictError;
use betterlite_router::classifier_verdict::prompt::{
    built_in_tier_prompt, classification_system_prompt, custom_tier_prompt,
};
use betterlite_router::classifier_verdict::reply::parse_class_reply;
use betterlite_router::classifier_verdict::response_format::tier_classification_format;
use betterlite_router::classifier_verdict::rubric::calibration_examples_section;
use betterlite_router::complexity_scoring::tier::{ComplexityTier, tier_severity_order};
use betterlite_router::tier_definitions::checks::{ClassifierFallback, TierChecks};
use betterlite_router::tier_definitions::model::{TierDefinition, TierSet};
use serde::{Deserialize, Serialize};

use self::wire::{Fields, Function, ParseOutcome, Payload, RejectReason, Reply};
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod wire;

const MAX_PADDED_BYTES: u64 = 0x0020_0000;
const DEFAULT_LABELS: [&str; 4] = ["SIMPLE", "MEDIUM", "COMPLEX", "REASONING"];

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let mut fields = Fields::default();
    match payload.function {
        Function::SystemPrompt => {
            fields.prompt = Some(invalid(classification_system_prompt(
                *required(payload.window.as_ref(), "window")?,
                payload.prompt.as_deref(),
                &labeled_tiers(payload),
                payload.rubric,
            ))?);
        }
        Function::BuiltInPrompt => fields.prompt = Some(render_built_in_prompt(payload)?),
        Function::CustomPrompt => fields.prompt = Some(render_custom_prompt(payload)?),
        Function::CalibrationExamples => {
            fields.examples = Some(render_calibration_examples(payload)?)
        }
        Function::ResponseFormat => fields.response_format = Some(render_response_format()?),
        Function::ResolveLabel => fields.resolved = resolve_label(payload)?,
        Function::ParseReply => fields.parse = Some(parse_reply_outcome(payload)?),
        Function::LoggingFlag => {
            fields.logging_flag =
                invalid(effective_logging_off(request_kwargs(payload)?.as_ref()))?;
        }
    }
    Ok(ProbeOutput::ClassifierVerdict(Output(Box::new(fields))))
}

fn required<'value, T>(value: Option<&'value T>, name: &str) -> Result<&'value T, OperationError> {
    value.ok_or_else(|| OperationError::InvalidRequest {
        message: format!("{name} is required for this function"),
    })
}

fn invalid<T, E: Display>(result: Result<T, E>) -> Result<T, OperationError> {
    result.map_err(|source| OperationError::InvalidRequest {
        message: source.to_string(),
    })
}

fn labeled_tiers(payload: &Payload) -> Vec<(ComplexityTier, &str)> {
    match &payload.labeled_tiers {
        Some(items) => items
            .iter()
            .map(|item| (item.tier, item.label.as_str()))
            .collect(),
        None => tier_severity_order(false)
            .iter()
            .map(|tier| (*tier, tier.name()))
            .collect(),
    }
}

fn render_built_in_prompt(payload: &Payload) -> Result<String, OperationError> {
    invalid(built_in_tier_prompt(
        payload.prompt.as_deref(),
        *required(payload.window.as_ref(), "window")?,
        &labeled_tiers(payload),
        payload.rubric,
        payload.examples.as_deref(),
    ))
}

fn render_custom_prompt(payload: &Payload) -> Result<String, OperationError> {
    let definitions = required(payload.definitions.as_ref(), "definitions")?;
    let window = *required(payload.window.as_ref(), "window")?;
    let mapped: Vec<TierDefinition> = definitions
        .iter()
        .map(|definition| TierDefinition {
            name: definition.name.clone(),
            description: definition.description.clone(),
        })
        .collect();
    let prompt = custom_tier_prompt(
        &mapped,
        payload.prompt.as_deref(),
        window,
        payload.examples.as_deref(),
    );
    invalid(prompt)
}

fn render_calibration_examples(payload: &Payload) -> Result<String, OperationError> {
    invalid(calibration_examples_section(
        *required(payload.rubric.as_ref(), "rubric")?,
        &labeled_tiers(payload),
    ))
}

fn render_response_format() -> Result<String, OperationError> {
    invalid(serde_json::to_string(&tier_classification_format(
        &DEFAULT_LABELS,
    )))
}

fn resolve_label(payload: &Payload) -> Result<Option<String>, OperationError> {
    let tiers = tier_set(payload)?;
    let label = required(payload.label.as_ref(), "label")?;
    Ok(tiers
        .resolve_classified_tier(label)
        .map(|tier| tier.name().to_owned()))
}

fn tier_set(payload: &Payload) -> Result<TierSet, OperationError> {
    let Some(input) = payload.tier_config.as_ref() else {
        return invalid(TierSet::resolve(
            &section::Config::default(),
            &TierChecks::default(),
        ));
    };
    let config = section::Config {
        tier_definitions: input.tier_definitions.clone(),
        non_reasoning_tier: input.non_reasoning_tier,
        tier_labels: input.tier_labels.clone(),
        tier_models: input.tier_models.clone(),
    };
    let checks = TierChecks {
        classifier_type: input.classifier_type,
        classifier_fallback: ClassifierFallback::Heuristic,
        fallback_tier: input.fallback_tier.as_deref(),
        classification_prompt: None,
        classification_examples: None,
        llm_system_prompt: None,
        classification_rubric: None,
        order_features: &[],
    };
    invalid(TierSet::resolve(&config, &checks))
}

fn request_kwargs(payload: &Payload) -> Result<Option<RequestKwargs>, OperationError> {
    let Some(value) = payload.request_kwargs.as_ref() else {
        return Ok(None);
    };
    let Json::Object(fields) = value else {
        return Err(OperationError::InvalidRequest {
            message: "request_kwargs must be a JSON object".to_owned(),
        });
    };
    Ok(Some(fields.clone()))
}

fn reply_bytes(reply: &Reply) -> Result<Vec<u8>, OperationError> {
    match reply {
        Reply::Text { content } => Ok(content.as_bytes().to_vec()),
        Reply::Padded {
            content,
            pad,
            pad_len,
        } => {
            if pad.is_empty() || !pad.is_ascii() || *pad_len > MAX_PADDED_BYTES {
                return Err(OperationError::InvalidRequest {
                    message: "a padded reply needs an ASCII pad and at most 2097152 pad bytes"
                        .to_owned(),
                });
            }
            let count =
                usize::try_from(*pad_len).map_err(|_error| OperationError::InvalidRequest {
                    message: format!("pad_len {pad_len} does not fit this platform"),
                })?;
            let mut builder = content.clone();
            builder.push_str(&pad.repeat(count));
            Ok(builder.into_bytes())
        }
    }
}

fn parse_reply_outcome(payload: &Payload) -> Result<ParseOutcome, OperationError> {
    let reply = required(payload.reply.as_ref(), "reply")?;
    let tiers = tier_set(payload)?;
    match parse_class_reply(&reply_bytes(reply)?, &tiers) {
        Ok(verdict) => Ok(ParseOutcome::Selected {
            label: verdict.label,
            tier: verdict.tier.name().to_owned(),
        }),
        Err(error) => rejection(payload, &error),
    }
}

fn rejection(
    payload: &Payload,
    error: &ClassifierVerdictError,
) -> Result<ParseOutcome, OperationError> {
    let reason = match error {
        ClassifierVerdictError::UnknownTier { .. } => RejectReason::UnknownTier,
        ClassifierVerdictError::ReplyTooLarge { .. } => RejectReason::TooLarge,
        ClassifierVerdictError::ReplyNotText { .. }
        | ClassifierVerdictError::NoVerdictObject
        | ClassifierVerdictError::VerdictSchema { .. }
        | ClassifierVerdictError::RubricExamples { .. }
        | ClassifierVerdictError::MissingLabel { .. }
        | ClassifierVerdictError::TierCriterion { .. }
        | ClassifierVerdictError::OperatorSectionBlank { .. }
        | ClassifierVerdictError::OperatorSectionTooLong { .. }
        | ClassifierVerdictError::RequestKwargs { .. }
        | ClassifierVerdictError::CallbackMetadata { .. } => RejectReason::Schema,
    };
    let withheld = matches!(reason, RejectReason::Schema)
        && classifier_reply_is_private(request_kwargs(payload)?.as_ref(), false);
    Ok(ParseOutcome::Rejected { reason, withheld })
}
