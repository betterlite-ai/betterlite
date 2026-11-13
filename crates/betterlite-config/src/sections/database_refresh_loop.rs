use betterlite_db::database_refresh_loop::endpoint::WriterEndpoint;
use betterlite_db::database_tokens::endpoint::{
    EndpointError, IAMEndpoint, parse_iam_endpoint_from_url,
};
use betterlite_db::database_url_policy::flag::TokenFlagError;
use betterlite_db::database_urls::environment::EnvironmentSource;
use betterlite_db::database_urls::settings::{DatabaseUrlSettings, SettingsError, from_env};
use secrecy::ExposeSecret as _;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // every value is read from the environment, so this section owns no YAML key
    #[serde(skip)]
    _no_yaml_keys: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub endpoints: DatabaseUrlSettings,
}

impl Settings {
    #[must_use]
    pub fn writer_endpoint(&self) -> WriterEndpoint {
        WriterEndpoint {
            host: self.endpoints.database_host.clone(),
            port: self.endpoints.database_port.clone(),
            user: self.endpoints.database_user.clone(),
            name: self.endpoints.database_name.clone(),
            schema: self.endpoints.database_schema.clone(),
        }
    }

    pub fn explicit_reader_endpoint(&self) -> Result<Option<IAMEndpoint>, EndpointError> {
        let Some(url) = self
            .endpoints
            .database_url_read_replica
            .as_ref()
            .filter(|url| !url.expose_secret().is_empty())
        else {
            return Ok(None);
        };
        parse_iam_endpoint_from_url(url.expose_secret()).map(Some)
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment = LoaderEnvironment { variables: lookup };
        let endpoints = from_env(&environment).map_err(settings_error)?;
        Ok(Settings { endpoints })
    }
}

struct LoaderEnvironment<'lookup, L> {
    variables: &'lookup L,
}

impl<L> EnvironmentSource for LoaderEnvironment<'_, L>
where
    L: EnvironmentLookup,
{
    fn lookup(&self, name: &str) -> Option<String> {
        // B710's model reads names case-insensitively, so the folded spelling names the same variable
        self.variables
            .lookup(name)
            .or_else(|| self.variables.lookup(&name.to_ascii_lowercase()))
    }
}

fn settings_error(error: SettingsError) -> ConfigError {
    match error {
        SettingsError::Flag {
            source: flag_error, ..
        } => match flag_error {
            TokenFlagError::NotRecognized { source } => ConfigError::Environment { source },
        },
        SettingsError::Lifetime { name, source } => {
            ConfigError::InvalidInteger { path: name, source }
        }
    }
}
