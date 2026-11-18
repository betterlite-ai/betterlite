use betterlite_config::sections::complexity_scoring as scoring_section;
use betterlite_config::sections::custom_dimensions as section;
use betterlite_models::decisions::Json;
use betterlite_router::complexity_scoring::classify::{PromptInput, Scorer};
use betterlite_router::complexity_scoring::config::Config;
use betterlite_router::complexity_scoring::decision::RoutingDecisionCause;
use betterlite_router::custom_dimensions::validate;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    function: Function,
    // the complexity router config in the key names betterlite resolves, so a case configures the service as a file does
    #[serde(default, skip_serializing_if = "Option::is_none")]
    config: Option<Json>,
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

#[derive(Debug, Deserialize)]
struct Dimensions {
    #[serde(default)]
    custom_dimensions: Vec<section::Config>,
    #[serde(default = "default_classifier")]
    classifier_type: String,
}

impl Default for Dimensions {
    fn default() -> Self {
        Self {
            custom_dimensions: Vec::new(),
            classifier_type: default_classifier(),
        }
    }
}

fn default_classifier() -> String {
    "heuristic".to_owned()
}

struct Resolved {
    config: Config,
    work: u64,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let fields = match payload.function {
        Function::PatternWork => pattern_work_fields(&payload.patterns),
        Function::Validate => validate_fields(payload.config.as_ref()),
        Function::Classify => classify_fields(payload)?,
    };
    Ok(ProbeOutput::CustomDimensions(Output(Box::new(fields))))
}

fn pattern_work_fields(patterns: &[String]) -> Fields {
    let results = patterns.iter().map(|pattern| entry(pattern)).collect();
    Fields {
        results: Some(results),
        ..Fields::default()
    }
}

fn entry(pattern: &str) -> Entry {
    match section::pattern_work(pattern) {
        Ok(work) => Entry {
            work: Some(work),
            message: None,
        },
        Err(problem) => Entry {
            work: None,
            message: Some(problem.message().to_owned()),
        },
    }
}

// a refused config is an answer, not a probe failure: the reference reports it as a validation error
fn validate_fields(config: Option<&Json>) -> Fields {
    match validated_work(config) {
        Some(work) => Fields {
            work: Some(work),
            accepted: Some(true),
            ..Fields::default()
        },
        None => Fields {
            accepted: Some(false),
            ..Fields::default()
        },
    }
}

fn validated_work(config: Option<&Json>) -> Option<u64> {
    let (scoring, dimensions) = configured(config).ok()?;
    let resolved = Config::resolve(&scoring).ok()?;
    validate(
        &dimensions.custom_dimensions,
        &dimensions.classifier_type,
        &resolved.dimension_weights,
    )
    .ok()
    .map(|validated| validated.work)
}

fn classify_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let resolved = resolved(payload.config.as_ref())?;
    let prompt = PromptInput {
        prompt: &payload.prompt,
        system_prompt: payload.system_prompt.as_deref(),
    };
    let outcome = Scorer::new(resolved.config)
        .and_then(|scorer| scorer.score_and_classify(&prompt))
        .map_err(|source| invalid(source.to_string()))?;
    Ok(Fields {
        tier: Some(outcome.tier.name().to_owned()),
        score: Some(format!("{:.6}", outcome.score)),
        signals: Some(outcome.signals),
        cause: Some(outcome.cause),
        ..Fields::default()
    })
}

// the section's typed view of the router config, then the service's own validation of its dimensions
fn resolved(config: Option<&Json>) -> Result<Resolved, OperationError> {
    let (scoring, dimensions) = configured(config)?;
    let resolved = Config::resolve(&scoring).map_err(|source| invalid(source.to_string()))?;
    let validated = validate(
        &dimensions.custom_dimensions,
        &dimensions.classifier_type,
        &resolved.dimension_weights,
    )
    .map_err(|source| invalid(source.to_string()))?;
    let config = Config {
        custom_dimensions: validated.dimensions,
        ..resolved
    };
    Ok(Resolved {
        config,
        work: validated.work,
    })
}

fn configured(
    config: Option<&Json>,
) -> Result<(scoring_section::Config, Dimensions), OperationError> {
    let Some(config) = config else {
        return Ok((scoring_section::Config::default(), Dimensions::default()));
    };
    let value = serde_json::to_value(config).map_err(|source| invalid(source.to_string()))?;
    let scoring =
        serde_json::from_value(value.clone()).map_err(|source| invalid(source.to_string()))?;
    let dimensions = serde_json::from_value(value).map_err(|source| invalid(source.to_string()))?;
    Ok((scoring, dimensions))
}

fn invalid(message: impl Into<String>) -> OperationError {
    OperationError::InvalidRequest {
        message: message.into(),
    }
}
