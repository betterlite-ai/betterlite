use betterlite_config::sections::complexity_scoring as section;
use betterlite_models::decisions::Json;
use betterlite_router::complexity_scoring::classify::{PromptInput, Scorer};
use betterlite_router::complexity_scoring::config::Config;
use betterlite_router::complexity_scoring::decision::RoutingDecisionCause;
use betterlite_router::complexity_scoring::tier::Tier;
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
    // the section's typed view, so the probe configures the scorer exactly as a config file does
    #[serde(default)]
    config: Option<Json>,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    tier: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Classify,
    TierModel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Fields {
    tier: Option<String>,
    // six decimals, because the reference formats the same way and a raw float would differ in representation
    score: Option<String>,
    signals: Option<Vec<String>>,
    cause: Option<RoutingDecisionCause>,
    model: Option<String>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let config = resolved_config(payload.config.as_ref())?;
    let fields = match payload.function {
        Function::Classify => classify_fields(config, payload)?,
        Function::TierModel => tier_model_fields(&config, payload.tier.as_deref())?,
    };
    Ok(ProbeOutput::ComplexityScoring(Output(Box::new(fields))))
}

// the section's typed view, so the probe configures the scorer exactly as a config file does
fn resolved_config(configured: Option<&Json>) -> Result<Config, OperationError> {
    let configured = configured
        .map(|config| {
            serde_json::to_value(config)
                .and_then(serde_json::from_value::<section::Config>)
                .map_err(|source| OperationError::InvalidRequest {
                    message: source.to_string(),
                })
        })
        .transpose()?
        .unwrap_or_default();
    Config::resolve(&configured).map_err(|source| OperationError::InvalidRequest {
        message: source.to_string(),
    })
}

fn classify_fields(config: Config, payload: &Payload) -> Result<Fields, OperationError> {
    let prompt = PromptInput {
        prompt: &payload.prompt,
        system_prompt: payload.system_prompt.as_deref(),
    };
    let outcome = Scorer::new(config)
        .and_then(|scorer| scorer.score_and_classify(&prompt))
        .map_err(|source| OperationError::InvalidRequest {
            message: source.to_string(),
        })?;
    Ok(Fields {
        tier: Some(outcome.tier.name().to_owned()),
        score: Some(format!("{:.6}", outcome.score)),
        signals: Some(outcome.signals),
        cause: Some(outcome.cause),
        model: None,
    })
}

fn tier_model_fields(config: &Config, tier: Option<&str>) -> Result<Fields, OperationError> {
    let tier = tier.ok_or_else(|| OperationError::InvalidRequest {
        message: "tier is required for tier_model".to_owned(),
    })?;
    let model = config
        .single_model_for_tier(&Tier::from_name(tier))
        .map_err(|source| OperationError::InvalidRequest {
            message: source.to_string(),
        })?;
    Ok(Fields {
        tier: None,
        score: None,
        signals: None,
        cause: None,
        model: Some(model.to_owned()),
    })
}
