use serde::Deserialize;

use super::Section;
use super::guardrail_params::mode::LitellmParamsMode;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};

const GUARDRAIL_KIND: &str = "cato_networks";

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
    mode: Option<LitellmParamsMode>,
    default_on: Option<bool>,
    timeout: Option<RawNumber>,
}

// the kwargs Cato's initialize_guardrail reads besides the projections: event_hook (mode), default_on and timeout
#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: String,
    pub mode: Option<LitellmParamsMode>,
    pub default_on: bool,
    pub timeout: Option<f64>,
}

pub type Settings = Vec<Configured>;

impl Section for Raw {
    type Settings = Vec<Configured>;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == GUARDRAIL_KIND {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(guardrails)
    }
}

impl Entry {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let parameters = self.litellm_params;
        // guardrail_params treats an empty timeout as unset, so this section reads it the same way
        let timeout = match parameters.timeout {
            Some(RawNumber::Text(text)) if text.is_empty() => None,
            other => resolve_number(other, &format!("{prefix}.timeout"), lookup, parse_float)?,
        };
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            mode: parameters.mode.map(lowercase_mode),
            default_on: parameters.default_on.unwrap_or(false),
            timeout,
        })
    }
}

// LitellmParams' mode validator only lowercases the plain string and list shapes
fn lowercase_mode(mode: LitellmParamsMode) -> LitellmParamsMode {
    match mode {
        LitellmParamsMode::One(value) => LitellmParamsMode::One(value.to_lowercase()),
        LitellmParamsMode::Many(values) => LitellmParamsMode::Many(
            values
                .into_iter()
                .map(|value| value.to_lowercase())
                .collect(),
        ),
        LitellmParamsMode::Tagged(tags) => LitellmParamsMode::Tagged(tags),
    }
}

#[path = "cato_lifecycle/tests.rs"]
mod tests;
