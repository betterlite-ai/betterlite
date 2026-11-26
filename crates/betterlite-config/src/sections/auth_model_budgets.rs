mod tests;

use rust_decimal::Decimal;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_number;
use crate::sections::Section;

const KEY: &str = "litellm_settings.max_budget";

// litellm.__init__ pins max_budget to 0.0, and get_global_proxy_spend reads only above it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub max_budget: Decimal,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_budget: Decimal::ZERO,
        }
    }
}

impl Settings {
    #[must_use]
    pub fn global_spend_enabled(&self) -> bool {
        self.max_budget > Decimal::ZERO
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    max_budget: Option<RawNumber>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let max_budget =
            resolve_number(self.litellm_settings.max_budget, KEY, lookup, parse_budget)?
                .unwrap_or(Decimal::ZERO);
        Ok(Settings { max_budget })
    }
}

// LiteLLM tests `max_budget > 0` on the float, so NaN, -inf and every non-positive value disable the read while +inf keeps it on
fn parse_budget(text: &str, path: &str) -> Result<Decimal, ConfigError> {
    let value = match text.trim() {
        "NaN" | "-inf" => return Ok(Decimal::ZERO),
        "inf" => return Ok(Decimal::MAX),
        trimmed => trimmed
            .parse::<Decimal>()
            .map_err(|_source| ConfigError::InvalidScalar {
                path: path.to_owned(),
                value: false,
            })?,
    };
    Ok(if value > Decimal::ZERO {
        value
    } else {
        Decimal::ZERO
    })
}
