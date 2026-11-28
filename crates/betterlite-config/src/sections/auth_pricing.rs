use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

mod tests;

const BUDGET_PATH: &str = "litellm_settings.max_budget";

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
    #[serde(default, rename = "reject_clientside_metadata_tags")]
    reject_clientside_tags: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellm {
    max_budget: Option<RawNumber>,
}

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
        match Decimal::from_f64(value) {
            Some(amount) => Self::Finite(amount),
            None if value.is_sign_negative() => Self::NegativeInfinity,
            None => Self::PositiveInfinity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settings {
    pub enforce_user_param: bool,
    pub reject_clientside_tags: bool,
    pub max_budget: MaxBudget,
}

impl Default for MaxBudget {
    fn default() -> Self {
        Self::Finite(Decimal::ZERO)
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        // proxy_server.py reads max_budget with float(value); the shared decoder resolves a reference and refuses a bool
        let max_budget = resolve_number(
            self.litellm_settings.max_budget,
            BUDGET_PATH,
            lookup,
            parse_float,
        )?
        .map_or_else(MaxBudget::default, MaxBudget::from_f64);
        Ok(Settings {
            enforce_user_param: general.enforce_user_param.unwrap_or_default(),
            reject_clientside_tags: general.reject_clientside_tags.unwrap_or_default(),
            max_budget,
        })
    }
}
