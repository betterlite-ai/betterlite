use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;
use crate::sections::adaptive_classification::Weights;

mod tests;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";
const ADAPTIVE_KEY: &str = "adaptive";
const WEIGHTS_KEY: &str = "adaptive_weights";
const PENALTY_KEY: &str = "tier_distance_penalty";
const ELIGIBLE_KEY: &str = "adaptive_eligible";
const ENV_MARKER: &str = "os.environ/";
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

#[derive(Debug, Default, Deserialize)]
struct RawWeights {
    #[serde(default)]
    quality: Option<RawNumber>,
    #[serde(default)]
    cost: Option<RawNumber>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Literal(bool),
    Text(String),
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
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(
                deployment
                    .litellm_params
                    .complexity_router_config
                    .map(|raw| raw.resolve(index, lookup))
                    .transpose()?,
            );
        }
        Ok(Settings { deployments })
    }
}

impl RawRouterConfig {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Config, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("model_list.{index}.{PATH_ROOT}");
        Ok(Config {
            adaptive: flag(self.adaptive, &format!("{prefix}.{ADAPTIVE_KEY}"), lookup)?
                .unwrap_or(DEFAULT_ADAPTIVE),
            weights: self
                .adaptive_weights
                .map(|weights| weights.resolve(&format!("{prefix}.{WEIGHTS_KEY}"), lookup))
                .transpose()?,
            tier_distance_penalty: penalty(
                self.tier_distance_penalty,
                &format!("{prefix}.{PENALTY_KEY}"),
                lookup,
            )?
            .unwrap_or(DEFAULT_TIER_DISTANCE_PENALTY),
            eligible: eligible(
                self.adaptive_eligible,
                &format!("{prefix}.{ELIGIBLE_KEY}"),
                lookup,
            )?
            .unwrap_or(AdaptiveEligible::All),
        })
    }
}

impl RawWeights {
    fn resolve<L>(self, prefix: &str, lookup: &L) -> Result<Weights, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let quality = resolve_number(
            self.quality,
            &format!("{prefix}.quality"),
            lookup,
            parse_float,
        )?;
        let economy = resolve_number(self.cost, &format!("{prefix}.cost"), lookup, parse_float)?;
        Ok(Weights { quality, economy })
    }
}

// LiteLLM resolves os.environ/ strings before pydantic reads the field as a bool
fn flag<L>(raw: Option<RawFlag>, path: &str, lookup: &L) -> Result<Option<bool>, ConfigError>
where
    L: EnvironmentLookup,
{
    match raw {
        None => Ok(None),
        Some(RawFlag::Literal(flag)) => Ok(Some(flag)),
        Some(RawFlag::Text(text)) => {
            let name = text.strip_prefix(ENV_MARKER).unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?)
        }
    }
}

// the declared field carries ge=0.0; a non-finite penalty has no score to compute
fn penalty<L>(raw: Option<RawNumber>, path: &str, lookup: &L) -> Result<Option<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = resolve_number(raw, path, lookup, parse_float)?;
    match resolved {
        Some(value) if !(value.is_finite() && value >= 0.0) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
        Some(value) => Ok(Some(value)),
        None => Ok(None),
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
    match resolve_string(text, path, lookup)?.as_str() {
        ELIGIBLE_ALL => Ok(Some(AdaptiveEligible::All)),
        ELIGIBLE_CLASSIFIED_TIER => Ok(Some(AdaptiveEligible::ClassifiedTier)),
        _other => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
    }
}
