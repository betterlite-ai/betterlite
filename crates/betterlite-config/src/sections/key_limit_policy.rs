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
    // ast-grep-ignore: no-long-names
    pub upperbound_key_generate_params: Option<UpperboundKeyGenerateParams>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let upperbound = self
            .litellm_settings
            .upperbound
            .map(|raw| params(raw, lookup))
            .transpose()?;
        Ok(Settings {
            // ast-grep-ignore: no-long-names
            upperbound_key_generate_params: upperbound,
        })
    }
}

fn params<L>(raw: RawUpperbound, lookup: &L) -> Result<UpperboundKeyGenerateParams, ConfigError>
where
    L: EnvironmentLookup,
{
    Ok(UpperboundKeyGenerateParams {
        max_budget: resolve_number(
            raw.max_budget,
            &format!("{KEY}.max_budget"),
            lookup,
            parse_money,
        )?,
        budget_duration: text(raw.budget_duration, "budget_duration", lookup)?,
        duration: text(raw.duration, "duration", lookup)?,
        max_parallel_requests: count(raw.max_parallel_requests, "max_parallel_requests", lookup)?,
        tpm_limit: count(raw.tpm_limit, "tpm_limit", lookup)?,
        rpm_limit: count(raw.rpm_limit, "rpm_limit", lookup)?,
    })
}

fn count<L>(value: Option<RawNumber>, field: &str, lookup: &L) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    resolve_number(value, &format!("{KEY}.{field}"), lookup, |number, path| {
        number
            .parse::<i64>()
            .map_err(|source| ConfigError::InvalidInteger {
                path: path.to_owned(),
                source,
            })
    })
}

fn text<L>(value: Option<RawText>, field: &str, lookup: &L) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("{KEY}.{field}");
    match value {
        None => Ok(None),
        Some(RawText::Text(found)) => resolve_string(found, &path, lookup).map(Some),
        Some(RawText::Other(_)) => Err(ConfigError::InvalidScalar { path, value: false }),
    }
}
