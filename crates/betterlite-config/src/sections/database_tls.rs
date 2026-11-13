use betterlite_db::database_tls::pool_policy::{PgBouncerSettings, PoolSettingsError, from_env};
use betterlite_db::database_urls::environment::EnvironmentSource;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// the section reads only LITELLM_PGBOUNCER_* environment variables, so every YAML key stays unknown
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub pgbouncer: PgBouncerSettings,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment = LoaderEnvironment { variables: lookup };
        let pool = from_env(&environment).map_err(pool_error)?;
        if !pool.ignored.is_empty() {
            tracing::warn!(
                ignored = %pool.ignored.join(", "),
                "the native gateway does not run an embedded PgBouncer, so these settings are ignored"
            );
        }
        Ok(Settings {
            pgbouncer: pool.settings,
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
        // LiteLLM's PgBouncerSettings matches env names case-insensitively; ask for the folded spelling too
        self.variables
            .lookup(name)
            .or_else(|| self.variables.lookup(&name.to_ascii_lowercase()))
    }
}

fn pool_error(error: PoolSettingsError) -> ConfigError {
    match error {
        PoolSettingsError::Environment(source) => ConfigError::Environment { source },
        PoolSettingsError::TooSmall { name, .. } | PoolSettingsError::TooLarge { name, .. } => {
            ConfigError::InvalidScalar {
                path: name,
                value: false,
            }
        }
    }
}
