use std::collections::BTreeMap;
use std::str::FromStr;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";

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
pub struct RawRouterConfig {
    // LiteLLM spells these two keys tiers and token_thresholds
    #[serde(default, alias = "tiers")]
    tier_models: Option<BTreeMap<String, TierModelValue>>,
    #[serde(default, alias = "token_thresholds")]
    complexity_thresholds: Option<BTreeMap<String, RawNumber>>,
    #[serde(default)]
    dimension_weights: Option<BTreeMap<String, RawNumber>>,
    #[serde(default)]
    tier_boundaries: Option<BTreeMap<String, RawNumber>>,
    #[serde(default)]
    reasoning_override_min_score: Option<RawNumber>,
    #[serde(default)]
    code_keywords: Option<Vec<String>>,
    #[serde(default)]
    reasoning_keywords: Option<Vec<String>>,
    #[serde(default)]
    technical_keywords: Option<Vec<String>>,
    #[serde(default)]
    custom_technical_keywords: Option<Vec<String>>,
    #[serde(default)]
    simple_keywords: Option<Vec<String>>,
    #[serde(default)]
    default_model: Option<String>,
}

// B374 owns the parameterized tier entry schema, so this baseline view reads only the bare shapes
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged, expecting = "a model name or a list of model names")]
pub enum TierModelValue {
    Single(String),
    Pool(Vec<String>),
}

// resolved scalars, with the keys still in the shape the configuration file spells them
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Config {
    pub tier_models: Option<BTreeMap<String, TierModelValue>>,
    pub complexity_thresholds: Option<BTreeMap<String, i64>>,
    pub dimension_weights: Option<BTreeMap<String, f64>>,
    pub tier_boundaries: Option<BTreeMap<String, f64>>,
    pub reasoning_override_min_score: Option<f64>,
    pub code_keywords: Option<Vec<String>>,
    pub reasoning_keywords: Option<Vec<String>>,
    pub technical_keywords: Option<Vec<String>>,
    pub custom_technical_keywords: Option<Vec<String>>,
    pub simple_keywords: Option<Vec<String>>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures no complexity router
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
            let configured = deployment.litellm_params.complexity_router_config;
            deployments.push(
                configured
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
        let config = Config {
            tier_models: tier_map(self.tier_models, "tier_models", &prefix, lookup)?,
            complexity_thresholds: None,
            dimension_weights: None,
            tier_boundaries: None,
            reasoning_override_min_score: None,
            code_keywords: self.code_keywords,
            reasoning_keywords: self.reasoning_keywords,
            technical_keywords: self.technical_keywords,
            custom_technical_keywords: self.custom_technical_keywords,
            simple_keywords: self.simple_keywords,
            default_model: self
                .default_model
                .map(|model| resolve_string(model, &format!("{prefix}.default_model"), lookup))
                .transpose()?,
        };
        let raw = RawNumbers {
            complexity_thresholds: self.complexity_thresholds,
            dimension_weights: self.dimension_weights,
            tier_boundaries: self.tier_boundaries,
            reasoning_override_min_score: self.reasoning_override_min_score,
        };
        with_numbers(config, raw, &prefix, lookup)
    }
}

struct RawNumbers {
    complexity_thresholds: Option<BTreeMap<String, RawNumber>>,
    dimension_weights: Option<BTreeMap<String, RawNumber>>,
    tier_boundaries: Option<BTreeMap<String, RawNumber>>,
    reasoning_override_min_score: Option<RawNumber>,
}

fn with_numbers<L>(
    mut config: Config,
    raw: RawNumbers,
    prefix: &str,
    lookup: &L,
) -> Result<Config, ConfigError>
where
    L: EnvironmentLookup,
{
    config.complexity_thresholds = numbers(
        raw.complexity_thresholds,
        "complexity_thresholds",
        prefix,
        lookup,
        parse_threshold,
    )?;
    config.dimension_weights = numbers(
        raw.dimension_weights,
        "dimension_weights",
        prefix,
        lookup,
        parse_float,
    )?;
    config.tier_boundaries = numbers(
        raw.tier_boundaries,
        "tier_boundaries",
        prefix,
        lookup,
        parse_float,
    )?;
    config.reasoning_override_min_score = optional_number(
        raw.reasoning_override_min_score,
        &format!("{prefix}.reasoning_override_min_score"),
        lookup,
    )?;
    Ok(config)
}

// a tier's single model is a mapping value, so an os.environ reference in it is resolved; pool items stay literal
fn tier_map<L>(
    values: Option<BTreeMap<String, TierModelValue>>,
    field: &str,
    prefix: &str,
    lookup: &L,
) -> Result<Option<BTreeMap<String, TierModelValue>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(configured) = values else {
        return Ok(None);
    };
    let mut resolved = BTreeMap::new();
    for (tier, value) in configured {
        let path = format!("{prefix}.{field}.{tier}");
        let entry = match value {
            TierModelValue::Single(model) => {
                TierModelValue::Single(resolve_string(model, &path, lookup)?)
            }
            pool @ TierModelValue::Pool(_) => pool,
        };
        resolved.insert(tier, entry);
    }
    Ok(Some(resolved))
}

fn numbers<L, T>(
    values: Option<BTreeMap<String, RawNumber>>,
    field: &str,
    prefix: &str,
    lookup: &L,
    parse: fn(&str, &str) -> Result<T, ConfigError>,
) -> Result<Option<BTreeMap<String, T>>, ConfigError>
where
    L: EnvironmentLookup,
    T: FromStr,
{
    let Some(configured) = values else {
        return Ok(None);
    };
    let mut resolved = BTreeMap::new();
    for (name, value) in configured {
        let path = format!("{prefix}.{field}.{name}");
        reject_quoted(&value, &path)?;
        // an empty environment value is unset, so the entry stays absent
        let Some(parsed) = resolve_number(Some(value), &path, lookup, parse)? else {
            continue;
        };
        resolved.insert(name, parsed);
    }
    Ok(Some(resolved))
}

fn optional_number<L>(
    value: Option<RawNumber>,
    path: &str,
    lookup: &L,
) -> Result<Option<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(declared) = &value {
        reject_quoted(declared, path)?;
    }
    resolve_number(value, path, lookup, parse_float)
}

// a declared number is a number; only an os.environ reference may be text
fn reject_quoted(value: &RawNumber, path: &str) -> Result<(), ConfigError> {
    if let RawNumber::Text(text) = value
        && !text.starts_with("os.environ/")
    {
        return Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        });
    }
    Ok(())
}

// a threshold is a declared int; the text form is read only for an os.environ reference
fn parse_threshold(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
