use std::collections::BTreeMap;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub type CostAdjustmentMap = BTreeMap<String, CostAdjustmentValue>;

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(untagged)]
pub enum CostAdjustmentValue {
    Null,
    Boolean(bool),
    Number(serde_json::Number),
    Text(String),
    Object(BTreeMap<String, Self>),
    Array(Vec<Self>),
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub litellm_settings: Global,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Global {
    #[serde(default)]
    pub cost_discount_config: CostAdjustmentMap,
    #[serde(default)]
    pub cost_margin_config: CostAdjustmentMap,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub cost_discount_config: CostAdjustmentMap,
    pub cost_margin_config: CostAdjustmentMap,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            cost_discount_config: self.litellm_settings.cost_discount_config,
            cost_margin_config: self.litellm_settings.cost_margin_config,
        })
    }
}

mod tests;
