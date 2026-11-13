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
        let api_key = SecretString::from(required_value(
            parameters
                .api_key
                .map(|secret| secret.expose_secret().to_owned()),
            API_KEY_VARIABLE,
            &format!("{prefix}.api_key"),
            lookup,
        )?);
        let api_base = optional_value(
            parameters.api_base,
            API_BASE_VARIABLE,
            &format!("{prefix}.api_base"),
            lookup,
        )?;
        let policy_name = optional_name(
            parameters.policy_name,
            &format!("{prefix}.policy_name"),
            lookup,
        )?;
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

fn required_value<L>(
    configured: Option<String>,
    variable: &str,
    path: &str,
    lookup: &L,
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let missing = || ConfigError::MissingEnvironment {
        variable: variable.to_owned(),
        path: path.to_owned(),
    };
    let candidate = configured
        .filter(|value| !value.is_empty())
        .or_else(|| lookup.lookup(variable))
        .ok_or_else(missing)?;
    let resolved = resolve_string(candidate, path, lookup)?;
    if resolved.is_empty() {
        return Err(missing());
    }
    Ok(resolved)
}

fn optional_value<L>(
    configured: Option<String>,
    variable: &str,
    path: &str,
    lookup: &L,
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = configured
        .map(|value| resolve_string(value, path, lookup))
        .transpose()?
        .filter(|value| !value.is_empty())
        .or_else(|| lookup.lookup(variable).filter(|value| !value.is_empty()));
    Ok(resolved.unwrap_or_else(|| DEFAULT_API_BASE.to_owned()))
}

fn optional_name<L>(
    configured: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = configured
        .map(|value| resolve_string(value, path, lookup))
        .transpose()?;
    Ok(resolved.filter(|value| !value.is_empty()))
}
