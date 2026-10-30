use std::fmt;

use serde::Deserialize;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde_json::Number;

#[derive(Debug, Clone, Default)]
pub struct OffPeakBlock {
    pub hours_utc: Option<HoursField>,
    pub windows: Option<WindowsField>,
    pub weekday_timezone: Option<TimezoneField>,
    pub input_cost_per_token: Option<RateField>,
    pub output_cost_per_token: Option<RateField>,
    pub reasoning_token_cost: Option<RateField>,
    pub cache_read_input_token_cost: Option<RateField>,
    pub cache_creation_cost: Option<RateField>,
}

// A plain derive also accepts a JSON array through visit_seq, so OffPeakBlockEntry needs this map-only visitor to fall through to Other instead.
macro_rules! deserialize_off_peak_block {
    () => {
        struct OffPeakBlockVisitor;

        impl<'de> Visitor<'de> for OffPeakBlockVisitor {
            type Value = OffPeakBlock;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut block = OffPeakBlock::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "hours_utc" => block.hours_utc = Some(map.next_value()?),
                        "windows" => block.windows = Some(map.next_value()?),
                        "weekday_timezone" => block.weekday_timezone = Some(map.next_value()?),
                        "input_cost_per_token" => {
                            block.input_cost_per_token = Some(map.next_value()?);
                        }
                        "output_cost_per_token" => {
                            block.output_cost_per_token = Some(map.next_value()?);
                        }
                        "output_cost_per_reasoning_token" => {
                            block.reasoning_token_cost = Some(map.next_value()?);
                        }
                        "cache_read_input_token_cost" => {
                            block.cache_read_input_token_cost = Some(map.next_value()?);
                        }
                        "cache_creation_input_token_cost" => {
                            block.cache_creation_cost = Some(map.next_value()?);
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(block)
            }
        }

        impl<'de> Deserialize<'de> for OffPeakBlock {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                deserializer.deserialize_map(OffPeakBlockVisitor)
            }
        }
    };
}

deserialize_off_peak_block!();

// LiteLLM's _open_off_peak_block requires a Mapping, so a list-shaped value is no block at all.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OffPeakBlockEntry {
    Block(Box<OffPeakBlock>),
    Other(IgnoredAny),
}

impl OffPeakBlockEntry {
    #[must_use]
    pub const fn as_block(&self) -> Option<&OffPeakBlock> {
        match self {
            Self::Block(block) => Some(block),
            Self::Other(_) => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum HoursField {
    One(String),
    Many(Vec<StringOrIgnored>),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StringOrIgnored {
    Text(String),
    Ignored(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum WindowsField {
    List(Vec<WindowRuleEntry>),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum WindowRuleEntry {
    Rule(WindowRule),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Default)]
pub struct WindowRule {
    pub hours_utc: Option<HoursField>,
    pub weekdays: Option<WeekdaysField>,
}

// Same map-only guard as OffPeakBlock, so a list-shaped windows entry falls through WindowRuleEntry to Other.
macro_rules! deserialize_window_rule {
    () => {
        struct WindowRuleVisitor;

        impl<'de> Visitor<'de> for WindowRuleVisitor {
            type Value = WindowRule;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut rule = WindowRule::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "hours_utc" => rule.hours_utc = Some(map.next_value()?),
                        "weekdays" => rule.weekdays = Some(map.next_value()?),
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(rule)
            }
        }

        impl<'de> Deserialize<'de> for WindowRule {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                deserializer.deserialize_map(WindowRuleVisitor)
            }
        }
    };
}

deserialize_window_rule!();

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum WeekdaysField {
    List(Vec<WeekdayEntry>),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum WeekdayEntry {
    Bool(bool),
    Int(i64),
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TimezoneField {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RateField {
    Bool(bool),
    Number(Number),
    Text(String),
    Other(IgnoredAny),
}
