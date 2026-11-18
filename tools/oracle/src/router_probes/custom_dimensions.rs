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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    dimension_weights: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    custom_dimensions: Vec<section::Config>,
    #[serde(default = "default_classifier")]
    classifier_type: String,
}

// a payload arrives as JSON, whose numbers are finite, so the derived PartialEq is reflexive
impl Eq for ProbeConfig {}

fn default_classifier() -> String {
    "heuristic".to_owned()
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
        work: None,
        accepted: None,
        tier: None,
        score: None,
        signals: None,
        cause: None,
    }
}

fn entry(pattern: &str) -> Entry {
    match section::pattern_work(pattern) {
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

// a refused config is an answer, not a probe failure: the reference reports it as a validation error
fn validate_fields(config: Option<&ProbeConfig>) -> Fields {
    let resolved = resolved(config).map(|(_config, work)| work);
    let accepted = resolved.is_ok();
    Fields {
        results: None,
        work: resolved.ok(),
        accepted: Some(accepted),
        tier: None,
        score: None,
        signals: None,
        cause: None,
    }
}

fn classify_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let (resolved, _work) = resolved(payload.config.as_ref())?;
    let prompt = PromptInput {
        prompt: &payload.prompt,
        system_prompt: payload.system_prompt.as_deref(),
    };
    let outcome = Scorer::new(resolved)
        .and_then(|scorer| scorer.score_and_classify(&prompt))
        .map_err(|source| invalid(source.to_string()))?;
    Ok(Fields {
        results: None,
        work: None,
        accepted: None,
        tier: Some(outcome.tier.name().to_owned()),
        score: Some(format!("{:.6}", outcome.score)),
        signals: Some(outcome.signals),
        cause: Some(outcome.cause),
    })
}

// the section's typed view of the router config, then the service's own validation of its dimensions
fn resolved(config: Option<&ProbeConfig>) -> Result<(Config, u64), OperationError> {
    let mut scoring = scoring_section::Config::default();
    scoring.dimension_weights = config.and_then(|config| config.dimension_weights.clone());
    let resolved = Config::resolve(&scoring).map_err(|source| invalid(source.to_string()))?;
    let empty = Vec::new();
    let dimensions = config.map_or(&empty, |config| &config.custom_dimensions);
    let classifier = config.map_or("heuristic", |config| config.classifier_type.as_str());
    let validated = validate(dimensions, classifier, &resolved.dimension_weights)
        .map_err(|source| invalid(source.to_string()))?;
    let work = validated.work;
    let config = Config {
        custom_dimensions: validated.dimensions,
        ..resolved
    };
    Ok((config, work))
}

fn invalid(message: String) -> OperationError {
    OperationError::InvalidRequest { message }
}
