use std::collections::BTreeMap;

use betterlite_config::sections::complexity_scoring as scoring_section;
use betterlite_config::sections::custom_dimensions as section;
use betterlite_router::complexity_scoring::classify::{PromptInput, Scorer};
use betterlite_router::complexity_scoring::config::Config;
use betterlite_router::complexity_scoring::decision::RoutingDecisionCause;
use betterlite_router::custom_dimensions::validate;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod tests;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    function: Function,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    config: Option<ProbeConfig>,
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    PatternWork,
    Validate,
    Classify,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Fields {
    results: Option<Vec<Entry>>,
    work: Option<u64>,
    accepted: Option<bool>,
    tier: Option<String>,
    // six decimals, because the reference formats the same way and a raw float would differ in representation
    score: Option<String>,
    signals: Option<Vec<String>>,
    cause: Option<RoutingDecisionCause>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    work: Option<u64>,
    message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    dimension_weights: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    custom_dimensions: Vec<section::Config>,
    #[serde(default = "default_classifier_type")]
    classifier_type: String,
}

// a payload arrives as JSON, whose numbers are finite, so the derived PartialEq is reflexive
impl Eq for ProbeConfig {}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let mut fields = Fields::default();
    match payload.function {
        Function::PatternWork => {
            fields.results = Some(
                payload
                    .patterns
                    .iter()
                    .map(|pattern| entry(pattern))
                    .collect(),
            );
        }
        Function::Validate => {
            let result = resolved(payload.config.as_ref());
            fields.accepted = Some(result.is_ok());
            fields.work = result.ok().map(|(_config, work)| work);
        }
        Function::Classify => classify_fields(payload, &mut fields)?,
    }
    Ok(ProbeOutput::CustomDimensions(Output(Box::new(fields))))
}

fn entry(pattern: &str) -> Entry {
    match section::pattern::pattern_work(pattern) {
        Ok((_matcher, work)) => Entry {
            work: Some(work),
            message: None,
        },
        Err(problem) => Entry {
            work: None,
            message: Some(problem.message().to_owned()),
        },
    }
}

fn classify_fields(payload: &Payload, fields: &mut Fields) -> Result<(), OperationError> {
    let (resolved, _work) = resolved(payload.config.as_ref())?;
    let prompt = PromptInput {
        prompt: &payload.prompt,
        system_prompt: payload.system_prompt.as_deref(),
    };
    let outcome = Scorer::new(resolved)
        .and_then(|scorer| scorer.score_and_classify(&prompt))
        .map_err(|source| OperationError::InvalidRequest {
            message: source.to_string(),
        })?;
    fields.tier = Some(outcome.tier.name().to_owned());
    fields.score = Some(format!("{:.6}", outcome.score));
    fields.signals = Some(outcome.signals);
    fields.cause = Some(outcome.cause);
    Ok(())
}

fn resolved(config: Option<&ProbeConfig>) -> Result<(Config, u64), OperationError> {
    let scoring = scoring_section::Config {
        tier_models: None,
        complexity_thresholds: None,
        dimension_weights: config.and_then(|settings| settings.dimension_weights.clone()),
        tier_boundaries: None,
        reasoning_override_min_score: None,
        code_keywords: None,
        reasoning_keywords: None,
        technical_keywords: None,
        custom_technical_keywords: None,
        simple_keywords: None,
        default_model: None,
    };
    let mut resolved =
        Config::resolve(&scoring).map_err(|source| OperationError::InvalidRequest {
            message: source.to_string(),
        })?;
    let empty = Vec::new();
    let dimensions = config.map_or(&empty, |settings| &settings.custom_dimensions);
    let classifier = config.map_or("heuristic", |settings| &settings.classifier_type);
    let validated =
        validate(dimensions, classifier, &resolved.dimension_weights).map_err(|source| {
            OperationError::InvalidRequest {
                message: source.to_string(),
            }
        })?;
    resolved.custom_dimensions = validated.dimensions;
    Ok((resolved, validated.work))
}

fn default_classifier_type() -> String {
    "heuristic".to_owned()
}
