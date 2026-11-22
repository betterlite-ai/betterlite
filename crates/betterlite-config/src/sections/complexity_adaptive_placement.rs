use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;
use crate::sections::adaptive_classification::Weights;

use self::flag::RawFlag;
use self::weights::RawWeights;

mod flag;
mod tests;
mod weights;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";
const ADAPTIVE_KEY: &str = "adaptive";
const WEIGHTS_KEY: &str = "adaptive_weights";
const PENALTY_KEY: &str = "tier_distance_penalty";
const ELIGIBLE_KEY: &str = "adaptive_eligible";
const ELIGIBLE_ALL: &str = "all";
const ELIGIBLE_CLASSIFIED_TIER: &str = "classified_tier";

pub const DEFAULT_ADAPTIVE: bool = false;
pub const DEFAULT_QUALITY_WEIGHT: f64 = 0.3;
pub const DEFAULT_COST_WEIGHT: f64 = 0.7;
pub const DEFAULT_TIER_DISTANCE_PENALTY: f64 = 0.5;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    #[serde(default)]
    complexity_router_config: Option<RawRouterConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouterConfig {
    #[serde(default)]
    adaptive: Option<RawFlag>,
    #[serde(default)]
    adaptive_weights: Option<RawWeights>,
    #[serde(default)]
    tier_distance_penalty: Option<RawNumber>,
    #[serde(default)]
    adaptive_eligible: Option<String>,
}

// adaptive_eligible: the two values the declared Literal accepts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptiveEligible {
    All,
    ClassifiedTier,
}

impl AdaptiveEligible {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => ELIGIBLE_ALL,
            Self::ClassifiedTier => ELIGIBLE_CLASSIFIED_TIER,
        }
    }

    fn declared(text: &str) -> Option<Self> {
        match text {
            ELIGIBLE_ALL => Some(Self::All),
            ELIGIBLE_CLASSIFIED_TIER => Some(Self::ClassifiedTier),
            _other => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub adaptive: bool,
    // None keeps LiteLLM's adaptive_weights declaration default in the caller
    pub weights: Option<Weights>,
    pub tier_distance_penalty: f64,
    pub eligible: AdaptiveEligible,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            adaptive: DEFAULT_ADAPTIVE,
            weights: None,
            tier_distance_penalty: DEFAULT_TIER_DISTANCE_PENALTY,
            eligible: AdaptiveEligible::All,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures no placement settings
    pub deployments: Vec<Option<Config>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let deployments = self
            .model_list
            .into_iter()
            .enumerate()
            .map(|(index, deployment)| {
                deployment
                    .litellm_params
                    .complexity_router_config
                    .map(|raw| raw.resolve(index, lookup))
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Settings { deployments })
    }
}

impl RawRouterConfig {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Config, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("model_list.{index}.{PATH_ROOT}");
        let penalty_path = format!("{prefix}.{PENALTY_KEY}");
        let penalty = resolve_number(
            self.tier_distance_penalty,
            &penalty_path,
            lookup,
            parse_float,
        )?;
        // the declared field carries ge=0.0: a negative or non-finite penalty has no score to compute
        if penalty.is_some_and(|value| !(value.is_finite() && value >= 0.0)) {
            return Err(ConfigError::InvalidScalar {
                path: penalty_path,
                value: false,
            });
        }
        Ok(Config {
            adaptive: self
                .adaptive
                .map(|flag| flag.into_bool(&format!("{prefix}.{ADAPTIVE_KEY}"), lookup))
                .transpose()?
                .flatten()
                .unwrap_or(DEFAULT_ADAPTIVE),
            weights: self
                .adaptive_weights
                .map(|weights| weights.resolve(&format!("{prefix}.{WEIGHTS_KEY}"), lookup))
                .transpose()?,
            tier_distance_penalty: penalty.unwrap_or(DEFAULT_TIER_DISTANCE_PENALTY),
            eligible: eligible(
                self.adaptive_eligible,
                &format!("{prefix}.{ELIGIBLE_KEY}"),
                lookup,
            )?
            .unwrap_or(AdaptiveEligible::All),
        })
    }
}

fn eligible<L>(
    raw: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<AdaptiveEligible>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = raw else {
        return Ok(None);
    };
    let resolved = resolve_string(text, path, lookup)?;
    AdaptiveEligible::declared(&resolved)
        .map(Some)
        .ok_or_else(|| ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        })
}
