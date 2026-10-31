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
    Percentage,
    FixedAmount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscountEntry {
    Percent(Decimal),
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginFieldEntry {
    Absent,
    Number(Decimal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginEntry {
    Percent(Decimal),
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
            cost_discount_config: resolve_adjustment_entries(
                "litellm_settings.cost_discount_config",
                self.litellm_settings.cost_discount_config,
                discount_entry,
            )?,
            cost_margin_config: resolve_adjustment_entries(
                "litellm_settings.cost_margin_config",
                self.litellm_settings.cost_margin_config,
                margin_entry,
            )?,
        })
    }
}

pub fn resolve_adjustment_entries<T>(
    prefix: &str,
    config: CostAdjustmentMap,
    mut entry_for: impl FnMut(&str, &CostAdjustmentValue, &mut Vec<String>) -> Option<T>,
) -> Result<BTreeMap<String, T>, ConfigError> {
    let mut resolved = BTreeMap::new();
    let mut paths = Vec::new();
    for (provider, value) in config {
        let path = format!("{prefix}.{provider}");
        if let Some(entry) = entry_for(&path, &value, &mut paths) {
            resolved.insert(provider, entry);
        }
    }
    if paths.is_empty() {
        Ok(resolved)
    } else {
        Err(ConfigError::Unsupported { paths })
    }
}

fn parse_percent_lexeme<T>(
    lexeme: &str,
    error_path: impl FnOnce() -> String,
    paths: &mut Vec<String>,
    build: impl FnOnce(Decimal) -> T,
) -> Option<T> {
    match parse_decimal_lexeme(lexeme) {
        Ok(percent) => Some(build(percent)),
        Err(_) => {
            paths.push(error_path());
            None
        }
    }
}

pub fn discount_entry(
    path: &str,
    value: &CostAdjustmentValue,
    paths: &mut Vec<String>,
) -> Option<DiscountEntry> {
    let CostAdjustmentValue::Number(number) = value else {
        return Some(DiscountEntry::Unsupported);
    };
    parse_percent_lexeme(
        &number.to_string(),
        || path.to_owned(),
        paths,
        DiscountEntry::Percent,
    )
}

pub fn margin_entry(
    path: &str,
    value: &CostAdjustmentValue,
    paths: &mut Vec<String>,
) -> Option<MarginEntry> {
    match value {
        CostAdjustmentValue::Number(number) => parse_percent_lexeme(
            &number.to_string(),
            || path.to_owned(),
            paths,
            MarginEntry::Percent,
        ),
        CostAdjustmentValue::Object(entries) => Some(MarginEntry::Split {
            percentage: margin_field(path, entries, AdjustmentField::Percentage, paths),
            fixed_amount: margin_field(path, entries, AdjustmentField::FixedAmount, paths),
        }),
        CostAdjustmentValue::Null
        | CostAdjustmentValue::Boolean(_)
        | CostAdjustmentValue::Text(_)
        | CostAdjustmentValue::Array(_) => Some(MarginEntry::Unsupported),
    }
}

fn margin_field(
    path: &str,
    entries: &BTreeMap<String, CostAdjustmentValue>,
    field: AdjustmentField,
    paths: &mut Vec<String>,
) -> MarginFieldEntry {
    let field_path = || format!("{path}.{field}");
    match entries.get(field.to_string().as_str()) {
        None | Some(CostAdjustmentValue::Boolean(_)) => MarginFieldEntry::Absent,
        Some(CostAdjustmentValue::Number(number)) => parse_percent_lexeme(
            &number.to_string(),
            field_path,
            paths,
            MarginFieldEntry::Number,
        )
        .unwrap_or(MarginFieldEntry::Absent),
        Some(CostAdjustmentValue::Text(text)) => {
            parse_percent_lexeme(text.trim(), field_path, paths, MarginFieldEntry::Number)
                .unwrap_or(MarginFieldEntry::Absent)
        }
        Some(
            CostAdjustmentValue::Null
            | CostAdjustmentValue::Object(_)
            | CostAdjustmentValue::Array(_),
        ) => {
            paths.push(field_path());
            MarginFieldEntry::Absent
        }
    }
}

mod tests;
