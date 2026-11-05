use std::collections::BTreeMap;

use crate::resolve::SecretText;
use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};

use super::{Boolean, ConfigError, EnvironmentLookup, resolve_string};

pub(super) struct Resolver<'lookup> {
    pub(super) lookup: &'lookup dyn EnvironmentLookup,
}

impl Resolver<'_> {
    pub(super) fn text<T>(&self, value: Option<T>, field: &str) -> Result<Option<T>, ConfigError>
    where
        T: SecretText + From<String>,
    {
        let Some(raw) = value.map(SecretText::secret_text) else {
            return Ok(None);
        };
        let from_environment = raw.starts_with("os.environ/");
        let path = format!("general_settings.key_management_settings.{field}");
        let resolved = resolve_string(raw, &path, self.lookup)?;
        Ok((!from_environment || !resolved.is_empty()).then(|| T::from(resolved)))
    }

    pub(super) fn names(
        &self,
        values: Option<Vec<String>>,
        field: &str,
    ) -> Result<Option<Vec<String>>, ConfigError> {
        let path = format!("general_settings.key_management_settings.{field}");
        values
            .map(|names| {
                names
                    .into_iter()
                    .map(|name| resolve_string(name, &path, self.lookup))
                    .collect()
            })
            .transpose()
    }

    pub(super) fn tags(
        &self,
        values: Option<BTreeMap<String, String>>,
    ) -> Result<Option<BTreeMap<String, String>>, ConfigError> {
        values
            .map(|tags| {
                tags.into_iter()
                    .map(|(key, value)| {
                        let path = format!("general_settings.key_management_settings.tags.{key}");
                        let resolved = resolve_string(value, &path, self.lookup)?;
                        Ok((key, resolved))
                    })
                    .collect()
            })
            .transpose()
    }

    pub(super) fn custom(
        &self,
        value: Option<String>,
        system: Option<KeyManagementSystem>,
    ) -> Result<Option<String>, ConfigError> {
        let resolved = self.text(value, "custom_secret_manager")?;
        if system == Some(KeyManagementSystem::Custom)
            && resolved.as_ref().is_none_or(|path| path.trim().is_empty())
        {
            return Err(ConfigError::InvalidScalar {
                path: "general_settings.key_management_settings.custom_secret_manager".to_owned(),
                value: false,
            });
        }
        Ok(resolved)
    }
}

pub(super) fn access(value: Option<&str>) -> Result<SecretAccessMode, ConfigError> {
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

impl Boolean {
    pub(super) fn resolve(self, lookup: &dyn EnvironmentLookup) -> Result<bool, ConfigError> {
        match self {
            Self::Literal(flag) => Ok(flag),
            Self::Environment(reference) => {
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
}
