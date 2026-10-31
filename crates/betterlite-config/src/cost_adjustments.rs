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
    Invalid,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginFieldEntry {
    Absent,
    Number(Decimal),
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginEntry {
    Percent(Decimal),
    Invalid,
    Split {
        percentage: MarginFieldEntry,
        fixed_amount: MarginFieldEntry,
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
            ),
            cost_margin_config: resolve_margin_config(self.litellm_settings.cost_margin_config),
        })
    }
}

#[must_use]
pub fn resolve_discount_config(config: CostAdjustmentMap) -> BTreeMap<String, DiscountEntry> {
    let mut resolved = BTreeMap::new();
    for (provider, value) in config {
        resolved.insert(provider, discount_entry(&value));
    }
    resolved
}

#[must_use]
pub fn resolve_margin_config(config: CostAdjustmentMap) -> BTreeMap<String, MarginEntry> {
    let mut resolved = BTreeMap::new();
    for (provider, value) in config {
        resolved.insert(provider, margin_entry(&value));
    }
    resolved
}

fn margin_entry(value: &CostAdjustmentValue) -> MarginEntry {
    match value {
        CostAdjustmentValue::Number(number) => parse_decimal_lexeme(&number.to_string())
            .map_or(MarginEntry::Invalid, MarginEntry::Percent),
        CostAdjustmentValue::Object(entries) => MarginEntry::Split {
            percentage: margin_field(entries, AdjustmentField::Percentage),
            fixed_amount: margin_field(entries, AdjustmentField::FixedAmount),
        },
        CostAdjustmentValue::Null
        | CostAdjustmentValue::Boolean(_)
        | CostAdjustmentValue::Text(_)
        | CostAdjustmentValue::Array(_) => MarginEntry::Unsupported,
    }
}

// a discount entry reads the same scalar shape as a margin entry, so delegate and collapse margin's split shape into unsupported
fn discount_entry(value: &CostAdjustmentValue) -> DiscountEntry {
    match margin_entry(value) {
        MarginEntry::Percent(percent) => DiscountEntry::Percent(percent),
        MarginEntry::Invalid => DiscountEntry::Invalid,
        MarginEntry::Split { .. } | MarginEntry::Unsupported => DiscountEntry::Unsupported,
    }
}

fn margin_field(
    entries: &BTreeMap<String, CostAdjustmentValue>,
    field: AdjustmentField,
) -> MarginFieldEntry {
    match entries.get(field.to_string().as_str()) {
        None | Some(CostAdjustmentValue::Boolean(_)) => MarginFieldEntry::Absent,
        Some(CostAdjustmentValue::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map_or(MarginFieldEntry::Invalid, MarginFieldEntry::Number),
        Some(CostAdjustmentValue::Text(text)) => parse_decimal_lexeme(text.trim())
            .map_or(MarginFieldEntry::Invalid, MarginFieldEntry::Number),
        Some(
            CostAdjustmentValue::Null
            | CostAdjustmentValue::Object(_)
            | CostAdjustmentValue::Array(_),
        ) => MarginFieldEntry::Invalid,
    }
}

mod tests;
