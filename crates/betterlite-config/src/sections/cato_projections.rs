use secrecy::SecretString;
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::tls::VerifySetting;

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
    inspect_embeddings: Option<RawFlag>,
    ssl_verify: Option<VerifySetting>,
}

// guardrail_params resolves os.environ/ references in litellm_params before reading a bool key
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub inspect_embeddings: bool,
    pub ssl_verify: Option<VerifySetting>,
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
        let api_key = resolve_secret(parameters.api_key, &format!("{prefix}.api_key"), lookup)?;
        let api_base = parameters
            .api_base
            .map(|base| resolve_string(base, &format!("{prefix}.api_base"), lookup))
            .transpose()?;
        let path = format!("{prefix}.inspect_embeddings");
        let inspect_embeddings = match parameters.inspect_embeddings {
            Some(RawFlag::Boolean(flag)) => flag,
            Some(RawFlag::Text(text)) => {
                let name = text.strip_prefix("os.environ/").unwrap_or(&path).to_owned();
                let resolved = resolve_string(text, &path, lookup)?;
                betterlite_env::flag(&name, &resolved)?.unwrap_or(false)
            }
            None => false,
        };
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_key,
            api_base,
            inspect_embeddings,
            ssl_verify: parameters.ssl_verify,
        })
    }
}

#[path = "cato_projections/tests.rs"]
mod tests;
