use secrecy::SecretString;
use serde::Deserialize;
use serde::de::Error as DeserializeError;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_secret, resolve_string};

mod tests;

const GUARDRAIL: &str = "headroom";
const FAIL_CLOSED: &str = "fail_closed";
const FAIL_OPEN: &str = "fail_open";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    guardrails: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
struct Entry {
    guardrail_name: String,
    litellm_params: Parameters,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Parameters {
    guardrail: String,
    api_base: Option<String>,
    api_key: Option<SecretString>,
    model: Option<String>,
    unreachable_fallback: Option<String>,
    timeout: Option<RawNumber>,
    ccr_retrieval: Option<RawFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
}

impl RawFlag {
    fn resolve<L: EnvironmentLookup>(
        self,
        path: &str,
        lookup: &L,
    ) -> Result<Option<bool>, ConfigError> {
        match self {
            Self::Boolean(flag) => Ok(Some(flag)),
            Self::Text(text) => {
                let name = text.strip_prefix("os.environ/").unwrap_or(path).to_owned();
                let resolved = resolve_string(text, path, lookup)?;
                Ok(betterlite_env::flag(&name, &resolved)?)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnreachableFallback {
    FailClosed,
    FailOpen,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Configured>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub model: Option<String>,
    pub unreachable_fallback: Option<UnreachableFallback>,
    pub timeout: Option<f64>,
    pub ccr_retrieval: Option<bool>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == GUARDRAIL {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

impl Entry {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let parameters = self.litellm_params;
        let api_base = optional_value(parameters.api_base, &format!("{prefix}.api_base"), lookup)?;
        let api_key = resolve_secret(parameters.api_key, &format!("{prefix}.api_key"), lookup)?;
        let model = optional_value(parameters.model, &format!("{prefix}.model"), lookup)?;
        let unreachable_fallback = optional_fallback(
            parameters.unreachable_fallback,
            &format!("{prefix}.unreachable_fallback"),
            lookup,
        )?;
        let timeout = resolve_number(
            parameters.timeout,
            &format!("{prefix}.timeout"),
            lookup,
            parse_float,
        )?;
        let ccr_path = format!("{prefix}.ccr_retrieval");
        let ccr_retrieval = parameters
            .ccr_retrieval
            .map(|raw| raw.resolve(&ccr_path, lookup))
            .transpose()?
            .flatten();
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_base,
            api_key,
            model,
            unreachable_fallback,
            timeout,
            ccr_retrieval,
        })
    }
}

fn optional_value<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value else {
        return Ok(None);
    };
    let is_reference = text.starts_with("os.environ/");
    let resolved = resolve_string(text, path, lookup)?;
    if resolved.is_empty() && is_reference {
        return Ok(None);
    }
    Ok(Some(resolved))
}

fn optional_fallback<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<UnreachableFallback>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(resolved) = optional_value(value, path, lookup)? else {
        return Ok(None);
    };
    match resolved.as_str() {
        FAIL_CLOSED => Ok(Some(UnreachableFallback::FailClosed)),
        FAIL_OPEN => Ok(Some(UnreachableFallback::FailOpen)),
        other => {
            let source = <serde_saphyr::Error as DeserializeError>::custom(format!(
                "{path}: invalid unreachable_fallback `{other}`; expected `fail_closed` or `fail_open`"
            ));
            Err(ConfigError::Yaml { source })
        }
    }
}
