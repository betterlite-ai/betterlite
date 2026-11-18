use std::collections::BTreeMap;
use std::fmt;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::{Deserializer, EnumAccess, VariantAccess as _, Visitor};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_string;
use crate::sections::Section;

mod resolve;
mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    model_name: String,
    #[serde(default)]
    litellm_params: RawParams,
    #[serde(default)]
    model_info: RawInfo,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    quality_router_config: Option<RawRouterConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct RawInfo {
    litellm_routing_preferences: Option<RawPreferences>,
    // every price scalar keeps the text the file writes, so Decimal never sees an f64 rounding
    input_cost_per_token: Option<RawPrice>,
}

// a price as the scalar's written text, with quotes and tags already resolved
#[derive(Debug)]
struct RawPrice(String);

// ast-grep-ignore: no-custom-serde
impl<'de> Deserialize<'de> for RawPrice {
    // ast-grep-ignore: no-custom-serde
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // a scalar reaches an enum as its effective text, so a plain or tagged number keeps every digit
        deserializer.deserialize_enum("RawPrice", &[], PriceVisitor)
    }
}

struct PriceVisitor;

// ast-grep-ignore: no-custom-serde
impl<'de> Visitor<'de> for PriceVisitor {
    type Value = RawPrice;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a YAML number or an os.environ reference")
    }

    fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
    where
        A: EnumAccess<'de>,
    {
        let (text, variant) = data.variant::<String>()?;
        variant.unit_variant()?;
        Ok(RawPrice(text))
    }
}

#[derive(Debug, Deserialize)]
struct RawRouterConfig {
    // an absent or null list is the declared default, an explicit list is used as given
    available_models: Option<Vec<String>>,
    default_model: Option<String>,
    complexity_to_quality: Option<BTreeMap<String, RawNumber>>,
    // LiteLLM's config model allows extra fields, so an unknown key is kept and reported, not refused
    #[serde(flatten)]
    extras: BTreeMap<String, Json>,
}

#[derive(Debug, Deserialize)]
struct RawPreferences {
    quality_tier: RawNumber,
    keywords: Option<Vec<String>>,
    order: Option<RawNumber>,
    #[serde(flatten)]
    extras: BTreeMap<String, Json>,
}

// DEFAULT_COMPLEXITY_TO_QUALITY: the pinned mapping a file replaces by declaring its own table
#[must_use]
pub fn default_complexity_to_quality() -> BTreeMap<String, i64> {
    BTreeMap::from([
        ("SIMPLE".to_owned(), 1),
        ("MEDIUM".to_owned(), 2),
        ("COMPLEX".to_owned(), 3),
        ("REASONING".to_owned(), 4),
    ])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterConfig {
    pub available_models: Vec<String>,
    pub default_model: Option<String>,
    // resolved at load time: an absent or null table is the pinned default, an explicit one is used as given
    pub complexity_to_quality: BTreeMap<String, i64>,
    pub extras: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    pub quality_tier: i64,
    pub keywords: Vec<String>,
    pub order: Option<i64>,
    pub extras: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployment {
    pub model_name: String,
    pub config: Option<RouterConfig>,
    pub preferences: Option<Preferences>,
    pub input_cost_per_token: Option<Decimal>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // index-aligned with model_list; every entry is kept, since available_models may name any of them
    pub deployments: Vec<Deployment>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(deployment.resolve(index, lookup)?);
        }
        Ok(Settings { deployments })
    }
}

impl RawDeployment {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Deployment, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("model_list.{index}");
        let model_name = resolve_string(self.model_name, &format!("{prefix}.model_name"), lookup)?;
        let config = self
            .litellm_params
            .quality_router_config
            .map(|config| {
                config.resolve(
                    &format!("{prefix}.litellm_params.quality_router_config"),
                    lookup,
                )
            })
            .transpose()?;
        let preferences = self
            .model_info
            .litellm_routing_preferences
            .map(|preferences| {
                preferences.resolve(
                    &format!("{prefix}.model_info.litellm_routing_preferences"),
                    lookup,
                )
            })
            .transpose()?;
        let cost_path = format!("{prefix}.model_info.input_cost_per_token");
        let input_cost_per_token =
            resolve::resolve_price(self.model_info.input_cost_per_token, &cost_path, lookup)?;
        Ok(Deployment {
            model_name,
            config,
            preferences,
            input_cost_per_token,
        })
    }
}
