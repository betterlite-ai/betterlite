use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSpecificPricing {
    #[serde(deserialize_with = "crate::price_fields::deserialize::optional_price")]
    pub us: Option<Decimal>,
    #[serde(deserialize_with = "crate::price_fields::deserialize::optional_price")]
    pub fast: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataPricing {
    #[serde(
        rename = "original_pricing_per_hour",
        deserialize_with = "crate::price_fields::deserialize::optional_price"
    )]
    pub per_hour: Option<Decimal>,
    #[serde(
        rename = "original_pricing_per_minute",
        deserialize_with = "crate::price_fields::deserialize::optional_price"
    )]
    pub per_minute: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OffPeakPricing {
    #[serde(deserialize_with = "crate::price_fields::deserialize::optional_price")]
    pub input_cost_per_token: Option<Decimal>,
    #[serde(deserialize_with = "crate::price_fields::deserialize::optional_price")]
    pub output_cost_per_token: Option<Decimal>,
    #[serde(deserialize_with = "crate::price_fields::deserialize::optional_price")]
    pub cache_read_input_token_cost: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::deserialize::optional_price"
    )]
    pub cache_create_token: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_reasoning_token",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::deserialize::optional_price"
    )]
    pub output_reasoning_token: Option<Decimal>,
    #[serde(default)]
    pub windows: Vec<PriceWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours_utc: Option<HoursUtc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekday_timezone: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceWindow {
    pub hours_utc: HoursUtc,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekdays: Option<Vec<Weekday>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HoursUtc {
    One(String),
    Many(Vec<String>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde_repr::Serialize_repr)]
#[serde(try_from = "WeekdayWire")]
#[repr(u8)]
pub enum Weekday {
    Monday = 1,
    Tuesday = 2,
    Wednesday = 3,
    Thursday = 4,
    Friday = 5,
    Saturday = 6,
    Sunday = 7,
}

impl Weekday {
    fn named(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "mon" | "monday" => Some(Self::Monday),
            "tue" | "tues" | "tuesday" => Some(Self::Tuesday),
            "wed" | "wednesday" => Some(Self::Wednesday),
            "thu" | "thur" | "thurs" | "thursday" => Some(Self::Thursday),
            "fri" | "friday" => Some(Self::Friday),
            "sat" | "saturday" => Some(Self::Saturday),
            "sun" | "sunday" => Some(Self::Sunday),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WeekdayWire {
    Number(u8),
    Name(String),
}

#[derive(Debug, thiserror::Error)]
pub enum WeekdayError {
    #[error("weekday {0} is not between 1 and 7")]
    OutOfRange(u8),
    #[error("{0:?} is not a recognized weekday name")]
    UnknownName(String),
}

impl TryFrom<WeekdayWire> for Weekday {
    type Error = WeekdayError;

    fn try_from(wire: WeekdayWire) -> Result<Self, Self::Error> {
        match wire {
            WeekdayWire::Number(1) => Ok(Self::Monday),
            WeekdayWire::Number(2) => Ok(Self::Tuesday),
            WeekdayWire::Number(3) => Ok(Self::Wednesday),
            WeekdayWire::Number(4) => Ok(Self::Thursday),
            WeekdayWire::Number(5) => Ok(Self::Friday),
            WeekdayWire::Number(6) => Ok(Self::Saturday),
            WeekdayWire::Number(7) => Ok(Self::Sunday),
            WeekdayWire::Number(number) => Err(WeekdayError::OutOfRange(number)),
            WeekdayWire::Name(name) => {
                let day = Self::named(&name);
                day.ok_or(WeekdayError::UnknownName(name))
            }
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchContextPricing {
    #[serde(deserialize_with = "crate::price_fields::deserialize::optional_price")]
    pub search_context_size_low: Option<Decimal>,
    #[serde(deserialize_with = "crate::price_fields::deserialize::optional_price")]
    pub search_context_size_medium: Option<Decimal>,
    #[serde(deserialize_with = "crate::price_fields::deserialize::optional_price")]
    pub search_context_size_high: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TierPricing {
    #[serde(
        rename = "input_cost_per_token",
        deserialize_with = "crate::price_fields::deserialize::optional_price"
    )]
    pub input_token: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token",
        deserialize_with = "crate::price_fields::deserialize::optional_price"
    )]
    pub output_token: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_reasoning_token",
        deserialize_with = "crate::price_fields::deserialize::optional_price"
    )]
    pub output_reasoning_token: Option<Decimal>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::deserialize::optional_price"
    )]
    pub cache_read_input_token_cost: Option<Decimal>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::deserialize::optional_price"
    )]
    pub input_cost_per_query: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::deserialize::optional_price"
    )]
    pub cache_create_token: Option<Decimal>,
    #[serde(
        deserialize_with = "serde_with::As::<Option<Vec<serde_with::FromInto<super::deserialize::ExactDecimal>>>>::deserialize"
    )]
    pub range: Option<Vec<Decimal>>,
    #[serde(
        deserialize_with = "serde_with::As::<Option<Vec<serde_with::FromInto<super::deserialize::ExactDecimal>>>>::deserialize"
    )]
    pub max_results_range: Option<Vec<Decimal>>,
}
