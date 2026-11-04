use crate::resolve::SecretText;
use std::mem;

use betterlite_secret_managers::secret_types::SecretAccessMode;

use super::{Boolean, ConfigError, EnvironmentLookup, Settings, resolve_string};

pub(super) fn text<T>(
    value: Option<T>,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<T>, ConfigError>
where
    T: SecretText + From<String>,
{
    let Some(raw) = value.map(SecretText::secret_text) else {
        return Ok(None);
    };
    let from_environment = raw.starts_with("os.environ/");
    let resolved = resolve_string(raw, path, lookup)?;
    Ok((!from_environment || !resolved.is_empty()).then(|| T::from(resolved)))
}

pub(super) fn collections(
    settings: &mut Settings,
    lookup: &dyn EnvironmentLookup,
) -> Result<(), ConfigError> {
    let entries = settings
        .hosted_keys
        .iter_mut()
        .flatten()
        .map(|value| ("hosted_keys".to_owned(), value))
        .chain(
            settings
                .replica_regions
                .iter_mut()
                .flatten()
                .map(|value| ("replica_regions".to_owned(), value)),
        )
        .chain(
            settings
                .tags
                .iter_mut()
                .flatten()
                .map(|(key, value)| (format!("tags.{key}"), value)),
        );
    for (field, value) in entries {
        *value = resolve_string(
            mem::take(value),
            &format!("general_settings.key_management_settings.{field}"),
            lookup,
        )?;
    }
    Ok(())
}

pub(super) fn defaults(
    settings: &mut Settings,
    store_virtual_keys: Option<Boolean>,
    access_mode: Option<String>,
    prefix: Option<String>,
    lookup: &dyn EnvironmentLookup,
) -> Result<(), ConfigError> {
    settings.store_virtual_keys = match store_virtual_keys {
        None => false,
        Some(Boolean::Literal(flag)) => flag,
        Some(Boolean::Environment(reference)) => {
            let path = "general_settings.key_management_settings.store_virtual_keys";
            let variable = reference.strip_prefix("os.environ/").ok_or_else(|| {
                ConfigError::InvalidScalar {
                    path: path.to_owned(),
                    value: false,
                }
            })?;
            let resolved = resolve_string(reference.clone(), path, lookup)?;
            betterlite_env::flag(variable, &resolved)?.unwrap_or(false)
        }
    };
    let mode = text(
        access_mode,
        "general_settings.key_management_settings.access_mode",
        lookup,
    )?;
    settings.access_mode = access(mode.as_deref())?;
    if let Some(resolved) = text(
        prefix,
        "general_settings.key_management_settings.prefix_for_stored_virtual_keys",
        lookup,
    )? {
        settings.prefix_for_stored_virtual_keys = resolved;
    }
    Ok(())
}

fn access(value: Option<&str>) -> Result<SecretAccessMode, ConfigError> {
    match value {
        None | Some("read_only") => Ok(SecretAccessMode::ReadOnly),
        Some("read_and_write") => Ok(SecretAccessMode::ReadAndWrite),
        Some("write_only") => Ok(SecretAccessMode::WriteOnly),
        Some(_) => Err(ConfigError::InvalidScalar {
            path: "general_settings.key_management_settings.access_mode".to_owned(),
            value: false,
        }),
    }
}
