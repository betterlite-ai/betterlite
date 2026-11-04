use crate::resolve::resolve_secret;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::BTreeMap;

use betterlite_secret_managers::secret_types::SecretAccessMode;

use super::{Boolean, ConfigError, EnvironmentLookup, resolve_string};

pub(super) fn text<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = value else { return Ok(None) };
    let from_environment = raw.starts_with("os.environ/");
    let resolved = resolve_string(raw, path, lookup)?;
    Ok((!from_environment || !resolved.is_empty()).then_some(resolved))
}

pub(super) fn names<L>(
    values: Option<Vec<String>>,
    field: &str,
    lookup: &L,
) -> Result<Option<Vec<String>>, ConfigError>
where
    L: EnvironmentLookup,
{
    values
        .map(|names| {
            names
                .into_iter()
                .map(|name| {
                    resolve_string(
                        name,
                        &format!("general_settings.key_management_settings.{field}"),
                        lookup,
                    )
                })
                .collect()
        })
        .transpose()
}

pub(super) fn tags<L>(
    values: Option<BTreeMap<String, String>>,
    lookup: &L,
) -> Result<Option<BTreeMap<String, String>>, ConfigError>
where
    L: EnvironmentLookup,
{
    values
        .map(|tags| {
            tags.into_iter()
                .map(|(key, value)| {
                    let resolved = resolve_string(
                        value,
                        &format!("general_settings.key_management_settings.tags.{key}"),
                        lookup,
                    )?;
                    Ok((key, resolved))
                })
                .collect()
        })
        .transpose()
}

pub(super) fn boolean<L>(value: Option<Boolean>, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None => Ok(false),
        Some(Boolean::Literal(flag)) => Ok(flag),
        Some(Boolean::Environment(reference)) => {
            let path = "general_settings.key_management_settings.store_virtual_keys";
            let variable = reference.strip_prefix("os.environ/").ok_or_else(|| {
                ConfigError::InvalidScalar {
                    path: path.to_owned(),
                    value: false,
                }
            })?;
            let resolved = resolve_string(reference.clone(), path, lookup)?;
            Ok(betterlite_env::flag(variable, &resolved)?.unwrap_or(false))
        }
    }
}

pub(super) fn access<L>(value: Option<String>, lookup: &L) -> Result<SecretAccessMode, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = "general_settings.key_management_settings.access_mode";
    let resolved = text(value, path, lookup)?;
    match resolved.as_deref() {
        None | Some("read_only") => Ok(SecretAccessMode::ReadOnly),
        Some("read_and_write") => Ok(SecretAccessMode::ReadAndWrite),
        Some("write_only") => Ok(SecretAccessMode::WriteOnly),
        Some(_) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
    }
}

pub(super) fn token<L>(
    value: Option<SecretString>,
    lookup: &L,
) -> Result<Option<SecretString>, ConfigError>
where
    L: EnvironmentLookup,
{
    let from_environment = value
        .as_ref()
        .is_some_and(|secret| secret.expose_secret().starts_with("os.environ/"));
    let resolved = resolve_secret(
        value,
        "general_settings.key_management_settings.aws_web_identity_token",
        lookup,
    )?;
    Ok(resolved.filter(|secret| !from_environment || !secret.expose_secret().is_empty()))
}
