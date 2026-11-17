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
    let configured = payload
        .config
        .as_ref()
        .map(|config| {
            serde_json::to_value(config)
                .and_then(serde_json::from_value::<section::Config>)
                .map_err(|source| OperationError::InvalidRequest {
                    message: source.to_string(),
                })
        })
        .transpose()?
        .unwrap_or_default();
    let config = Config::resolve(&configured).map_err(|source| OperationError::InvalidRequest {
        message: source.to_string(),
    })?;
    let mut fields = Fields::default();
    match payload.function {
        Function::Classify => {
            let prompt = PromptInput {
                prompt: &payload.prompt,
                system_prompt: payload.system_prompt.as_deref(),
            };
            let outcome = Scorer::new(config)
                .and_then(|scorer| scorer.score_and_classify(&prompt))
                .map_err(|source| OperationError::InvalidRequest {
                    message: source.to_string(),
                })?;
            fields.tier = Some(outcome.tier.name().to_owned());
            fields.score = Some(format!("{:.6}", outcome.score));
            fields.signals = Some(outcome.signals);
            fields.cause = Some(outcome.cause);
        }
        Function::TierModel => {
            let tier = payload
                .tier
                .as_deref()
                .ok_or_else(|| OperationError::InvalidRequest {
                    message: "tier is required for tier_model".to_owned(),
                })?;
            fields.model = Some(
                config
                    .single_model_for_tier(&Tier::from_name(tier))
                    .map_err(|source| OperationError::InvalidRequest {
                        message: source.to_string(),
                    })?
                    .to_owned(),
            );
        }
    }
    Ok(ProbeOutput::ComplexityScoring(Output(Box::new(fields))))
}
