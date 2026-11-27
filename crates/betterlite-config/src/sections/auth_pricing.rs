use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
    #[serde(default)]
    litellm_settings: RawLitellm,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    enforce_user_param: Option<bool>,
    #[serde(default)]
    reject_clientside_metadata_tags: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellm {
    #[serde(default)]
    max_budget: Option<BudgetNumber>,
}

// Python reads this one with float(), so YAML .inf and .nan reach the check as the floats they are
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(transparent)]
struct BudgetNumber(f64);

// litellm.max_budget's four Python float classifications, kept apart because the budget check branches on finiteness
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxBudget {
    Finite(Decimal),
    Nan,
    PositiveInfinity,
    NegativeInfinity,
}

impl MaxBudget {
    fn from_f64(value: f64) -> Self {
        if value.is_nan() {
            return Self::Nan;
        }
        if value.is_infinite() {
            return Self::infinity(value.is_sign_negative());
        }
        Decimal::from_f64(value)
            .map_or_else(|| Self::infinity(value.is_sign_negative()), Self::Finite)
    }

    const fn infinity(negative: bool) -> Self {
        if negative {
            Self::NegativeInfinity
        } else {
            Self::PositiveInfinity
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub enforce_user_param: bool,
    pub reject_clientside_metadata_tags: bool,
    pub max_budget: MaxBudget,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enforce_user_param: false,
            reject_clientside_metadata_tags: false,
            max_budget: MaxBudget::Finite(Decimal::ZERO),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        Ok(Settings {
            enforce_user_param: general.enforce_user_param.unwrap_or_default(),
            reject_clientside_metadata_tags: general
                .reject_clientside_metadata_tags
                .unwrap_or_default(),
            max_budget: self
                .litellm_settings
                .max_budget
                .map_or(MaxBudget::Finite(Decimal::ZERO), |number| {
                    MaxBudget::from_f64(number.0)
                }),
        })
    }
}
