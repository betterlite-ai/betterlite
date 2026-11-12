use betterlite_db::database_tokens::settings::{TokenToggles, database_url_is_pooled};
use betterlite_db::database_url_policy::flag::{
    FlagValue, TokenFlagError, token_auth_flag_enabled,
};
use betterlite_db::database_urls::environment::EnvironmentSource;
use betterlite_db::database_urls::settings::{
    AZURE_POSTGRESQL_AUTH_ENV_VAR, IAM_TOKEN_DB_AUTH_ENV_VAR,
};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw;

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub toggles: TokenToggles,
    pub pooled_database_url: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment = LoaderEnvironment { variables: lookup };
        Ok(Settings {
            toggles: TokenToggles {
                iam_token_db_auth: flag(
                    environment.lookup(IAM_TOKEN_DB_AUTH_ENV_VAR),
                    IAM_TOKEN_DB_AUTH_ENV_VAR,
                )?,
                azure_postgresql_auth: flag(
                    environment.lookup(AZURE_POSTGRESQL_AUTH_ENV_VAR),
                    AZURE_POSTGRESQL_AUTH_ENV_VAR,
                )?,
            },
            pooled_database_url: database_url_is_pooled(&environment),
        })
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
        // LiteLLM's settings fold the name to one case, so the loader is asked in the declared and folded spelling
        self.variables
            .lookup(name)
            .or_else(|| self.variables.lookup(&name.to_ascii_lowercase()))
    }
}

// B2307's parser rejects a spelling it cannot read instead of silently reading a typo as off
fn flag(value: Option<String>, name: &str) -> Result<bool, ConfigError> {
    let Some(text) = value else {
        return Ok(false);
    };
    let flag = FlagValue::Text(text);
    token_auth_flag_enabled(Some(&flag), name).map_err(|error| match error {
        TokenFlagError::NotRecognized { source } => ConfigError::Environment { source },
    })
}
