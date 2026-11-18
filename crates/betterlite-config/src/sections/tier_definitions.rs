use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

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
#[serde(from = "Json", into = "Json")]
pub enum TierParam {
    Text(SecretString),
    Value(Json),
    List(Vec<Self>),
    Object(IndexMap<String, Self>),
}

impl TierParam {
    #[must_use]
    pub fn exposed(&self) -> Json {
        match self {
            Self::Text(text) => Json::Text(text.expose_secret().to_owned()),
            Self::Value(value) => value.clone(),
            Self::List(items) => Json::List(items.iter().map(Self::exposed).collect()),
            Self::Object(entries) => Json::Object(
                entries
                    .iter()
                    .map(|(key, item)| (key.clone(), item.exposed()))
                    .collect(),
            ),
        }
    }
}

impl From<Json> for TierParam {
    fn from(value: Json) -> Self {
        match value {
            Json::Text(text) => Self::Text(SecretString::from(text)),
            Json::List(items) => Self::List(items.into_iter().map(Self::from).collect()),
            Json::Object(entries) => Self::Object(
                entries
                    .into_iter()
                    .map(|(key, item)| (key, Self::from(item)))
                    .collect(),
            ),
            other => Self::Value(other),
        }
    }
}

impl From<TierParam> for Json {
    fn from(value: TierParam) -> Self {
        value.exposed()
    }
}

impl PartialEq for TierParam {
    fn eq(&self, other: &Self) -> bool {
        match self {
            Self::Text(left) => matches!(other, Self::Text(right) if same_secret(left, right)),
            Self::Value(left) => matches!(other, Self::Value(right) if left == right),
            Self::List(left) => matches!(other, Self::List(right) if left == right),
            Self::Object(left) => matches!(other, Self::Object(right) if left == right),
        }
    }
}

impl Eq for TierParam {}

// secret bytes compare without an early exit, so a wrong prefix costs the same as a wrong last byte
fn same_secret(left: &SecretString, right: &SecretString) -> bool {
    let left = left.expose_secret().as_bytes();
    let right = right.expose_secret().as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(
    from = "Option<IndexMap<String, TierParam>>",
    into = "IndexMap<String, TierParam>"
)]
pub struct TierParams(IndexMap<String, TierParam>);

impl TierParams {
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&TierParam> {
        self.0.get(key)
    }
}

impl From<Option<IndexMap<String, TierParam>>> for TierParams {
    fn from(params: Option<IndexMap<String, TierParam>>) -> Self {
        Self(params.unwrap_or_default())
    }
}

impl From<TierParams> for IndexMap<String, TierParam> {
    fn from(params: TierParams) -> Self {
        params.0
    }
}

impl FromIterator<(String, Json)> for TierParams {
    fn from_iter<T: IntoIterator<Item = (String, Json)>>(iter: T) -> Self {
        Self(
            iter.into_iter()
                .map(|(key, value)| (key, TierParam::from(value)))
                .collect(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TierModelEntry {
    pub model_name: String,
    #[serde(default)]
    pub litellm_params: TierParams,
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

// the keys' environment references resolved, in the shape the configuration file spells them
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
