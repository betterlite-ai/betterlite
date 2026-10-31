use std::collections::BTreeMap;

use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::Decimal;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum AdjustmentField {
    DiscountPercent,
    MarginPercent,
    Percentage,
    FixedAmount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscountEntry {
    Percent(Decimal),
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginEntry {
    Percent(Decimal),
    Split {
        percentage: Option<Decimal>,
        fixed_amount: Option<Decimal>,
    },
    Unsupported,
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
    pub cost_discount_config: BTreeMap<String, DiscountEntry>,
    pub cost_margin_config: BTreeMap<String, MarginEntry>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            cost_discount_config: resolve_discount_config(
                self.litellm_settings.cost_discount_config,
            )?,
            cost_margin_config: resolve_margin_config(self.litellm_settings.cost_margin_config)?,
        })
    }
}

pub fn resolve_discount_config(
    config: CostAdjustmentMap,
) -> Result<BTreeMap<String, DiscountEntry>, ConfigError> {
    let mut resolved = BTreeMap::new();
    for (provider, value) in config {
        resolved.insert(provider, discount_entry(&value)?);
    }
    Ok(resolved)
}

pub fn resolve_margin_config(
    config: CostAdjustmentMap,
) -> Result<BTreeMap<String, MarginEntry>, ConfigError> {
    let mut resolved = BTreeMap::new();
    for (provider, value) in config {
        resolved.insert(provider, margin_entry(&value)?);
    }
    Ok(resolved)
}

fn discount_entry(value: &CostAdjustmentValue) -> Result<DiscountEntry, ConfigError> {
    match value {
        CostAdjustmentValue::Number(number) => {
            let field = AdjustmentField::DiscountPercent;
            parse_decimal_lexeme(&number.to_string())
                .map_err(|source| ConfigError::InvalidCostAdjustmentNumber { field, source })
                .map(DiscountEntry::Percent)
        }
        CostAdjustmentValue::Null
        | CostAdjustmentValue::Boolean(_)
        | CostAdjustmentValue::Text(_)
        | CostAdjustmentValue::Object(_)
        | CostAdjustmentValue::Array(_) => Ok(DiscountEntry::Unsupported),
    }
}

fn margin_entry(value: &CostAdjustmentValue) -> Result<MarginEntry, ConfigError> {
    match value {
        CostAdjustmentValue::Number(number) => {
            let field = AdjustmentField::MarginPercent;
            parse_decimal_lexeme(&number.to_string())
                .map_err(|source| ConfigError::InvalidCostAdjustmentNumber { field, source })
                .map(MarginEntry::Percent)
        }
        CostAdjustmentValue::Object(entries) => Ok(MarginEntry::Split {
            percentage: margin_field(entries, AdjustmentField::Percentage)?,
            fixed_amount: margin_field(entries, AdjustmentField::FixedAmount)?,
        }),
        CostAdjustmentValue::Null
        | CostAdjustmentValue::Boolean(_)
        | CostAdjustmentValue::Text(_)
        | CostAdjustmentValue::Array(_) => Ok(MarginEntry::Unsupported),
    }
}

fn margin_field(
    entries: &BTreeMap<String, CostAdjustmentValue>,
    field: AdjustmentField,
) -> Result<Option<Decimal>, ConfigError> {
    match entries.get(field.to_string().as_str()) {
        None | Some(CostAdjustmentValue::Boolean(_)) => Ok(None),
        Some(CostAdjustmentValue::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map_err(|source| ConfigError::InvalidCostAdjustmentNumber { field, source })
            .map(Some),
        Some(CostAdjustmentValue::Text(text)) => parse_decimal_lexeme(text.trim())
            .map_err(|source| ConfigError::InvalidCostAdjustmentNumber { field, source })
            .map(Some),
        Some(
            CostAdjustmentValue::Null
            | CostAdjustmentValue::Object(_)
            | CostAdjustmentValue::Array(_),
        ) => Err(ConfigError::UnsupportedCostAdjustmentShape { field }),
    }
}

mod tests;
