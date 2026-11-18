use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_with::{DefaultOnNull, serde_as};
use subtle::ConstantTimeEq as _;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;
mod values;

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

// betterlite spells the map tier_models and the flag non_reasoning_tier, and takes LiteLLM's names as aliases
#[derive(Debug, Default, Deserialize)]
struct RawRouterConfig {
    #[serde(default)]
    tier_definitions: Option<Vec<TierDefinition>>,
    #[serde(default, alias = "enable_non_reasoning_tier")]
    non_reasoning_tier: Option<RawFlag>,
    #[serde(default)]
    tier_labels: Option<BTreeMap<String, Option<String>>>,
    #[serde(default, alias = "tiers")]
    tier_models: Option<BTreeMap<String, Option<TierPool>>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TierDefinition {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

// A request parameter's text may be a credential, so it is a SecretString from deserialization onward
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged, into = "Json")]
pub enum TierParam {
    Text(SecretString),
    Number(Number),
    List(Vec<Self>),
    Object(IndexMap<String, Self>),
    Value(Json),
}

impl From<TierParam> for Json {
    fn from(value: TierParam) -> Self {
        match value {
            TierParam::Text(text) => Self::Text(text.expose_secret().to_owned()),
            TierParam::Number(number) => Self::Number(number),
            TierParam::List(items) => Self::List(items.into_iter().map(Self::from).collect()),
            TierParam::Object(entries) => Self::Object(
                entries
                    .into_iter()
                    .map(|(key, item)| (key, Self::from(item)))
                    .collect(),
            ),
            TierParam::Value(value) => value,
        }
    }
}

impl PartialEq for TierParam {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text(left), Self::Text(right)) => bool::from(
                left.expose_secret()
                    .as_bytes()
                    .ct_eq(right.expose_secret().as_bytes()),
            ),
            (Self::Number(left), Self::Number(right)) => left == right,
            (Self::Value(left), Self::Value(right)) => left == right,
            (Self::List(left), Self::List(right)) => left == right,
            (Self::Object(left), Self::Object(right)) => left == right,
            _ => false,
        }
    }
}

impl Eq for TierParam {}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TierModelEntry {
    pub model_name: String,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub litellm_params: IndexMap<String, TierParam>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged, expecting = "a model name or a tier model entry")]
pub enum TierEntry {
    Model(String),
    Detailed(TierModelEntry),
}

// LiteLLM accepts one entry or a list of them under a tier key
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(
    untagged,
    expecting = "a model name, a tier model entry, or a list of either"
)]
pub enum TierPool {
    Single(TierEntry),
    Pool(Vec<TierEntry>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub tier_definitions: Option<Vec<TierDefinition>>,
    #[serde(default, alias = "enable_non_reasoning_tier")]
    pub non_reasoning_tier: bool,
    #[serde(default)]
    pub tier_labels: BTreeMap<String, String>,
    // absent keeps LiteLLM's pinned default map, while an explicit empty map is used as given
    #[serde(default, alias = "tiers")]
    pub tier_models: Option<BTreeMap<String, TierPool>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures no tier settings
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
            tier_definitions: values::definitions(self.tier_definitions, &prefix, lookup)?,
            non_reasoning_tier: values::flag(
                self.non_reasoning_tier,
                &format!("{prefix}.non_reasoning_tier"),
                lookup,
            )?,
            tier_labels: values::resolve_map(
                self.tier_labels.unwrap_or_default(),
                &prefix,
                "tier_labels",
                lookup,
                values::reference,
            )?,
            tier_models: values::pools(self.tier_models, &prefix, lookup)?,
        })
    }
}
