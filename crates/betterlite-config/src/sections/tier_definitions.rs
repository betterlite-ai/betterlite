use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TierDefinition {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

// LiteLLM's ComplexityTierModel: a pool entry names its model and may override request params for it
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TierModelEntry {
    pub model_name: String,
    #[serde(default)]
    pub litellm_params: Option<IndexMap<String, Json>>,
}

// a bare model name or a parameterized entry, accepted anywhere a pool entry is
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged, expecting = "a model name or a tier model entry")]
pub enum TierEntry {
    Model(String),
    Detailed(TierModelEntry),
}

// LiteLLM accepts one entry or a list of them under a tier key
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(
    untagged,
    expecting = "a model name, a tier model entry, or a list of either"
)]
pub enum TierPool {
    Single(TierEntry),
    Pool(Vec<TierEntry>),
}

// the keys' environment references resolved, in the shape the configuration file spells them
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
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
            tier_definitions: definitions(self.tier_definitions, &prefix, lookup)?,
            non_reasoning_tier: flag(
                self.non_reasoning_tier,
                &format!("{prefix}.non_reasoning_tier"),
                lookup,
            )?,
            tier_labels: labels(self.tier_labels, &prefix, lookup)?,
            tier_models: pools(self.tier_models, &prefix, lookup)?,
        })
    }
}

fn flag<L>(value: Option<RawFlag>, path: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None => Ok(false),
        Some(RawFlag::Boolean(enabled)) => Ok(enabled),
        Some(RawFlag::Text(text)) => {
            let name = text.strip_prefix("os.environ/").unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?.unwrap_or(false))
        }
    }
}

// the same rule every scalar setting follows: an os.environ/ reference resolves once here, and an
// empty variable counts as unset
fn reference<L>(text: String, path: &str, lookup: &L) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(name) = text.strip_prefix("os.environ/").map(str::to_owned) else {
        return Ok(Some(text));
    };
    let Some(value) = lookup.lookup(&name) else {
        return Err(ConfigError::MissingEnvironment {
            variable: name,
            path: path.to_owned(),
        });
    };
    Ok((!value.is_empty()).then_some(value))
}

fn definitions<L>(
    raw: Option<Vec<TierDefinition>>,
    prefix: &str,
    lookup: &L,
) -> Result<Option<Vec<TierDefinition>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut resolved = Vec::with_capacity(raw.len());
    for (position, definition) in raw.into_iter().enumerate() {
        let path = format!("{prefix}.tier_definitions.{position}");
        let name = reference(definition.name, &format!("{path}.name"), lookup)?.unwrap_or_default();
        let description = definition
            .description
            .map(|text| reference(text, &format!("{path}.description"), lookup))
            .transpose()?
            .flatten();
        resolved.push(TierDefinition { name, description });
    }
    Ok(Some(resolved))
}

fn labels<L>(
    raw: Option<BTreeMap<String, Option<String>>>,
    prefix: &str,
    lookup: &L,
) -> Result<BTreeMap<String, String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let mut resolved = BTreeMap::new();
    for (tier, label) in raw {
        let Some(label) = label else {
            continue;
        };
        let path = format!("{prefix}.tier_labels.{tier}");
        if let Some(label) = reference(label, &path, lookup)? {
            resolved.insert(tier, label);
        }
    }
    Ok(resolved)
}

fn pools<L>(
    raw: Option<BTreeMap<String, Option<TierPool>>>,
    prefix: &str,
    lookup: &L,
) -> Result<Option<BTreeMap<String, TierPool>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut resolved = BTreeMap::new();
    for (tier, pool) in raw {
        let Some(pool) = pool else {
            continue;
        };
        let path = format!("{prefix}.tier_models.{tier}");
        if let Some(pool) = pool_value(pool, &path, lookup)? {
            resolved.insert(tier, pool);
        }
    }
    Ok(Some(resolved))
}

fn pool_value<L>(pool: TierPool, path: &str, lookup: &L) -> Result<Option<TierPool>, ConfigError>
where
    L: EnvironmentLookup,
{
    match pool {
        TierPool::Single(entry) => Ok(single_entry(entry, path, lookup)?.map(TierPool::Single)),
        TierPool::Pool(entries) => {
            let mut resolved = Vec::with_capacity(entries.len());
            for (position, entry) in entries.into_iter().enumerate() {
                let path = format!("{path}.{position}");
                if let Some(entry) = listed_entry(entry, &path, lookup)? {
                    resolved.push(entry);
                }
            }
            Ok(Some(TierPool::Pool(resolved)))
        }
    }
}

fn single_entry<L>(
    entry: TierEntry,
    path: &str,
    lookup: &L,
) -> Result<Option<TierEntry>, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = match entry {
        TierEntry::Model(model) => reference(model, path, lookup)?.map(TierEntry::Model),
        TierEntry::Detailed(entry) => detailed(entry, path, lookup)?.map(TierEntry::Detailed),
    };
    Ok(resolved)
}

// the proxy substitutes mapping values only, so a model name inside a list stays literal
fn listed_entry<L>(
    entry: TierEntry,
    path: &str,
    lookup: &L,
) -> Result<Option<TierEntry>, ConfigError>
where
    L: EnvironmentLookup,
{
    match entry {
        TierEntry::Model(model) => Ok(Some(TierEntry::Model(model))),
        TierEntry::Detailed(entry) => detailed(entry, path, lookup),
    }
}

fn detailed<L>(
    entry: TierModelEntry,
    path: &str,
    lookup: &L,
) -> Result<Option<TierEntry>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(model_name) = reference(entry.model_name, &format!("{path}.model_name"), lookup)?
    else {
        return Ok(None);
    };
    let values = entry.litellm_params.unwrap_or_default();
    let values = params(values, &format!("{path}.litellm_params"), lookup)?;
    Ok(Some(TierEntry::Detailed(TierModelEntry {
        model_name,
        litellm_params: Some(values),
    })))
}

fn params<L>(
    values: IndexMap<String, Json>,
    path: &str,
    lookup: &L,
) -> Result<IndexMap<String, Json>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = IndexMap::with_capacity(values.len());
    for (key, value) in values {
        let path = format!("{path}.{key}");
        if let Some(value) = param_value(value, &path, lookup)? {
            resolved.insert(key, value);
        }
    }
    Ok(resolved)
}

fn param_value<L>(value: Json, path: &str, lookup: &L) -> Result<Option<Json>, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        Json::Text(text) => Ok(reference(text, path, lookup)?.map(Json::Text)),
        Json::Object(inner) => Ok(Some(Json::Object(params(inner, path, lookup)?))),
        Json::List(items) => Ok(Some(Json::List(params_list(items, path, lookup)?))),
        other => Ok(Some(other)),
    }
}

// a list value recurses into its mapping items only, matching the proxy's walk
fn params_list<L>(items: Vec<Json>, path: &str, lookup: &L) -> Result<Vec<Json>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = Vec::with_capacity(items.len());
    for (position, item) in items.into_iter().enumerate() {
        let path = format!("{path}.{position}");
        let item = match item {
            Json::Object(inner) => Json::Object(params(inner, &path, lookup)?),
            other => other,
        };
        resolved.push(item);
    }
    Ok(resolved)
}
