use secrecy::{ExposeSecret as _, SecretString};

use super::{Configured, Entry, OptionalParams, RawExtraHeaders, UnreachableFallback};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

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
            "DEEPKEEP_API_KEY",
            &prefix,
            "api_key",
            lookup,
        )?);
        let api_base = required_value(
            parameters.api_base,
            "DEEPKEEP_API_BASE",
            &prefix,
            "api_base",
            lookup,
        )?;
        let firewall_id = required_value(
            parameters.deepkeep_firewall_id,
            "DEEPKEEP_FIREWALL_ID",
            &prefix,
            "deepkeep_firewall_id",
            lookup,
        )?;
        validate_extra_headers(parameters.extra_headers, &prefix)?;
        warn_nested_fallback(parameters.optional_params);
        let unreachable_fallback =
            resolve_fallback(parameters.unreachable_fallback, &prefix, lookup)?;
        let timeout =
            super::timeout::resolve(parameters.timeout, &format!("{prefix}.timeout"), lookup)?;
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_base: super::normalize_endpoint(&api_base),
            api_key,
            deepkeep_firewall_id: firewall_id,
            unreachable_fallback,
            timeout,
        })
    }
}

fn required_value<L>(
    configured: Option<String>,
    variable: &str,
    prefix: &str,
    path_name: &str,
    lookup: &L,
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("{prefix}.{path_name}");
    let missing = || ConfigError::MissingEnvironment {
        variable: variable.to_owned(),
        path: path.clone(),
    };
    let candidate = configured
        .filter(|value| !value.is_empty())
        .or_else(|| lookup.lookup(variable))
        .ok_or_else(missing)?;
    let resolved = resolve_string(candidate, &path, lookup)?;
    if resolved.is_empty() {
        return Err(missing());
    }
    Ok(resolved)
}

fn resolve_fallback<L>(
    configured: Option<String>,
    prefix: &str,
    lookup: &L,
) -> Result<UnreachableFallback, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("{prefix}.unreachable_fallback");
    let Some(text) = configured else {
        return Ok(UnreachableFallback::default());
    };
    let is_reference = text.starts_with("os.environ/");
    let resolved = resolve_string(text, &path, lookup)?;
    if resolved.is_empty() && is_reference {
        return Ok(UnreachableFallback::default());
    }
    UnreachableFallback::try_from(resolved).map_err(|error| super::yaml_error(&path, error))
}

fn validate_extra_headers(
    extra_headers: Option<RawExtraHeaders>,
    prefix: &str,
) -> Result<(), ConfigError> {
    match extra_headers {
        None => Ok(()),
        Some(RawExtraHeaders::List(values)) => {
            drop(values);
            tracing::warn!(
                "DeepKeep extra_headers list ignored; DeepKeep does not support configured extra headers"
            );
            Ok(())
        }
        Some(RawExtraHeaders::Other(_)) => Err(super::yaml_error(
            &format!("{prefix}.extra_headers"),
            "extra_headers must be a list of header names",
        )),
    }
}

fn warn_nested_fallback(optional_params: Option<OptionalParams>) {
    let nested = optional_params.and_then(|params| params.unreachable_fallback);
    if nested.is_some() {
        tracing::warn!(
            "DeepKeep optional_params.unreachable_fallback is ignored; set unreachable_fallback at the top level"
        );
    }
}
