use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::tls::VerifySetting;

pub const DEFAULT_API_BASE: &str = "https://api.aisec.catonetworks.com";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    guardrails: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Entry {
    guardrail_name: String,
    litellm_params: Parameters,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Parameters {
    guardrail: String,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    inspect_embeddings: Option<bool>,
    ssl_verify: Option<VerifySetting>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub inspect_embeddings: bool,
    pub ssl_verify: Option<VerifySetting>,
}

impl Section for Raw {
    type Settings = Vec<Configured>;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == "cato_networks" {
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
        let api_key = match parameters.api_key {
            Some(key) => Some(SecretString::from(resolve_string(
                key.expose_secret().to_owned(),
                &format!("{prefix}.api_key"),
                lookup,
            )?)),
            None => None,
        };
        let api_base = match parameters.api_base {
            Some(base) => Some(resolve_string(base, &format!("{prefix}.api_base"), lookup)?),
            None => None,
        };
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_key,
            api_base,
            inspect_embeddings: parameters.inspect_embeddings.unwrap_or(false),
            ssl_verify: parameters.ssl_verify,
        })
    }
}

#[path = "cato_projections/tests.rs"]
mod tests;
