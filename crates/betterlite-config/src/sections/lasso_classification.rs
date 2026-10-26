use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

mod tests;

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
    api_key: Option<SecretString>,
    pub api_base: Option<String>,
    #[serde(rename = "lasso_user_id")]
    pub user_id: Option<String>,
    #[serde(rename = "lasso_conversation_id")]
    pub conversation_id: Option<String>,
    pub mask: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Configured>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: String,
    pub api_key: SecretString,
    pub api_base: String,
    pub user_id: Option<String>,
    pub conversation_id: Option<String>,
    pub mask: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == "lasso" {
                guardrails.push(entry.resolve(index, lookup)?);
            } else {
                return Err(ConfigError::Unsupported {
                    paths: vec![format!("guardrails.{index}.litellm_params.guardrail")],
                });
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
        let supplied_key = parameters
            .api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or_else(|| lookup.lookup("LASSO_API_KEY").map(SecretString::from))
            .ok_or_else(|| ConfigError::MissingEnvironment {
                variable: "LASSO_API_KEY".to_owned(),
                path: format!("{prefix}.api_key"),
            })?;
        let api_key = SecretString::from(resolve_string(
            supplied_key.expose_secret().to_owned(),
            &format!("{prefix}.api_key"),
            lookup,
        )?);
        let api_base = optional(
            parameters.api_base,
            "LASSO_API_BASE",
            &format!("{prefix}.api_base"),
            lookup,
        )?
        .unwrap_or_else(|| "https://server.lasso.security/gateway/v3".to_owned());
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_key,
            api_base,
            user_id: optional(
                parameters.user_id,
                "LASSO_USER_ID",
                &format!("{prefix}.lasso_user_id"),
                lookup,
            )?,
            conversation_id: optional(
                parameters.conversation_id,
                "LASSO_CONVERSATION_ID",
                &format!("{prefix}.lasso_conversation_id"),
                lookup,
            )?,
            mask: parameters.mask,
        })
    }
}

fn optional<L>(
    value: Option<String>,
    variable: &str,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let configured = value
        .filter(|text| !text.is_empty())
        .or_else(|| lookup.lookup(variable));
    configured
        .map(|text| resolve_string(text, path, lookup))
        .transpose()
}
