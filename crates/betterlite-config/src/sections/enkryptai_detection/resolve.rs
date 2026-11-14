use secrecy::{ExposeSecret as _, SecretString};

use super::{Configured, DEFAULT_API_BASE, Entry};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

pub(super) const API_KEY_VARIABLE: &str = "ENKRYPTAI_API_KEY";
pub(super) const API_BASE_VARIABLE: &str = "ENKRYPTAI_API_BASE";

impl Entry {
    pub(super) fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let parameters = self.litellm_params;
        let api_key = resolve_api_key(parameters.api_key, &prefix, lookup)?;
        let api_base = resolve_value(
            parameters.api_base,
            API_BASE_VARIABLE,
            &format!("{prefix}.api_base"),
            lookup,
        )?
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let policy_name = parameters
            .policy_name
            .map(|value| resolve_string(value, &format!("{prefix}.policy_name"), lookup))
            .transpose()?
            .filter(|value| !value.is_empty());
        let timeout =
            super::timeout::resolve(parameters.timeout, &format!("{prefix}.timeout"), lookup)?;
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_key,
            api_base,
            policy_name,
            deployment_name: parameters.deployment_name,
            detectors: parameters.detectors,
            block_on_violation: parameters.block_on_violation.unwrap_or(true),
            mode: parameters.mode,
            timeout,
        })
    }
}

fn resolve_api_key<L>(
    configured: Option<SecretString>,
    prefix: &str,
    lookup: &L,
) -> Result<SecretString, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("{prefix}.api_key");
    let key = resolve_value(
        configured.map(|secret| secret.expose_secret().to_owned()),
        API_KEY_VARIABLE,
        &path,
        lookup,
    )?;
    key.map(SecretString::from)
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: API_KEY_VARIABLE.to_owned(),
            path,
        })
}

fn resolve_value<L>(
    configured: Option<String>,
    variable: &str,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(candidate) = configured
        .filter(|value| !value.is_empty())
        .or_else(|| lookup.lookup(variable))
    else {
        return Ok(None);
    };
    let resolved = resolve_string(candidate, path, lookup)?;
    Ok((!resolved.is_empty()).then_some(resolved))
}
