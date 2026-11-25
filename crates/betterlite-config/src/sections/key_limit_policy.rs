use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_money, resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const KEY: &str = "litellm_settings.upperbound_key_generate_params";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    #[serde(rename = "upperbound_key_generate_params")]
    upperbound: Option<RawUpperbound>,
}

#[derive(Debug, Default, Deserialize)]
struct RawUpperbound {
    max_budget: Option<RawNumber>,
    budget_duration: Option<RawText>,
    duration: Option<RawText>,
    max_parallel_requests: Option<RawNumber>,
    tpm_limit: Option<RawNumber>,
    rpm_limit: Option<RawNumber>,
}

// the two duration fields are declared str, so a number is a wrong type rather than a coerced value
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawText {
    Text(String),
    Other(IgnoredAny),
}

impl RawText {
    fn resolve<L>(self, field: &str, lookup: &L) -> Result<String, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let path = format!("{KEY}.{field}");
        match self {
            Self::Text(found) => resolve_string(found, &path, lookup),
            Self::Other(_) => Err(ConfigError::InvalidScalar { path, value: false }),
        }
    }
}

// proxy/_types.py LiteLLM_UpperboundKeyGenerateParams, in its declaration order
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct UpperboundKeyGenerateParams {
    #[serde(default)]
    pub max_budget: Option<Decimal>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub max_parallel_requests: Option<i64>,
    #[serde(default)]
    pub tpm_limit: Option<i64>,
    #[serde(default)]
    pub rpm_limit: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub upperbound_key_generate_params: Option<UpperboundKeyGenerateParams>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(raw) = self.litellm_settings.upperbound else {
            return Ok(Settings::default());
        };
        let params = UpperboundKeyGenerateParams {
            max_budget: resolve_number(
                raw.max_budget,
                &format!("{KEY}.max_budget"),
                lookup,
                parse_money,
            )?,
            budget_duration: raw
                .budget_duration
                .map(|text| text.resolve("budget_duration", lookup))
                .transpose()?,
            duration: raw
                .duration
                .map(|text| text.resolve("duration", lookup))
                .transpose()?,
            max_parallel_requests: resolve_number(
                raw.max_parallel_requests,
                &format!("{KEY}.max_parallel_requests"),
                lookup,
                parse_i64,
            )?,
            tpm_limit: resolve_number(
                raw.tpm_limit,
                &format!("{KEY}.tpm_limit"),
                lookup,
                parse_i64,
            )?,
            rpm_limit: resolve_number(
                raw.rpm_limit,
                &format!("{KEY}.rpm_limit"),
                lookup,
                parse_i64,
            )?,
        };
        Ok(Settings {
            upperbound_key_generate_params: Some(params),
        })
    }
}

fn parse_i64(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
