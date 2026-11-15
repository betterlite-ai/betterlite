use std::time::Duration;

use betterlite_db::migration_ledger::timeouts::{
    DEFAULT_COMMAND_TIMEOUT, DEFAULT_DEPLOY_TIMEOUT, DEFAULT_LOCK_TIMEOUT,
    MIGRATION_LOCK_TIMEOUT_ENV_VAR, PRISMA_COMMAND_TIMEOUT_ENV_VAR, PRISMA_DEPLOY_TIMEOUT_ENV_VAR,
    prisma_migrate_deploy_timeout, timeout_from_env,
};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub mod packaged;
pub mod startup;

mod tests;

pub const DISABLE_SCHEMA_UPDATE_ENV_VAR: &str = "DISABLE_SCHEMA_UPDATE";

const ENV_PREFIX: &str = "os.environ/";

const DISABLE_SCHEMA_UPDATE_PATH: &str = "general_settings.disable_prisma_schema_update";

// Prisma's own bootstrap switches. betterlite runs the migrations natively with no Node toolchain, so it
// accepts them and warns once instead of failing a deployment that still carries them.
pub const IGNORED_BOOTSTRAP_INPUTS: [&str; 5] = [
    "PRISMA_OFFLINE_MODE",
    "PRISMA_CLI_PATH",
    "PRISMA_NODEENV_CACHE_DIR",
    "LITELLM_PRISMA_BOOTSTRAP_TIMEOUT",
    "LITELLM_MIGRATION_DIR",
];

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneralSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneralSettings {
    #[serde(default, rename = "disable_prisma_schema_update")]
    disable_prisma_schema_update: Entry<RawDisable>,
}

// An absent key and an explicit null both fall back to the environment, unlike a present false.
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum Entry<T> {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Null(()),
    Value(T),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawDisable {
    Boolean(bool),
    Text(String),
    Other(serde::de::IgnoredAny),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub updates_enabled: bool,
    pub command_timeout: Duration,
    pub lock_timeout: Duration,
    pub deploy_timeout: Duration,
    pub bootstrap_inputs: Vec<&'static str>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            updates_enabled: true,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
            deploy_timeout: DEFAULT_DEPLOY_TIMEOUT,
            bootstrap_inputs: Vec::new(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let disable = disable_updates(self.general_settings.disable_prisma_schema_update, lookup)?;
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        let command = present(PRISMA_COMMAND_TIMEOUT_ENV_VAR);
        let lock = present(MIGRATION_LOCK_TIMEOUT_ENV_VAR);
        let deploy = present(PRISMA_DEPLOY_TIMEOUT_ENV_VAR);
        Ok(Settings {
            updates_enabled: !disable,
            command_timeout: timeout_from_env(
                PRISMA_COMMAND_TIMEOUT_ENV_VAR,
                command.as_deref(),
                DEFAULT_COMMAND_TIMEOUT,
            ),
            lock_timeout: timeout_from_env(
                MIGRATION_LOCK_TIMEOUT_ENV_VAR,
                lock.as_deref(),
                DEFAULT_LOCK_TIMEOUT,
            ),
            deploy_timeout: prisma_migrate_deploy_timeout(deploy.as_deref(), command.as_deref()),
            bootstrap_inputs: IGNORED_BOOTSTRAP_INPUTS
                .into_iter()
                .filter(|name| present(name).is_some())
                .collect(),
        })
    }
}

fn disable_updates<L>(raw: Entry<RawDisable>, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match raw {
        // the pinned default is the environment's "false" when nothing sets the key
        Entry::Absent | Entry::Null(()) => environment_disable(lookup),
        Entry::Value(RawDisable::Boolean(disable)) => Ok(disable),
        // LiteLLM's str_to_bool reads only true and false, and every other literal string enables updates
        Entry::Value(RawDisable::Text(text)) => match text.strip_prefix(ENV_PREFIX) {
            Some(variable) => referenced_disable(variable, lookup),
            None => Ok(text.trim().eq_ignore_ascii_case("true")),
        },
        Entry::Value(RawDisable::Other(_)) => Err(ConfigError::InvalidScalar {
            path: DISABLE_SCHEMA_UPDATE_PATH.to_owned(),
            value: false,
        }),
    }
}

fn environment_disable<L>(lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let text = lookup
        .lookup(DISABLE_SCHEMA_UPDATE_ENV_VAR)
        .unwrap_or_default();
    // an unset or empty variable is the pinned "false" default
    Ok(betterlite_env::flag(DISABLE_SCHEMA_UPDATE_ENV_VAR, &text)?.unwrap_or(false))
}

fn referenced_disable<L>(variable: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let text = lookup.lookup(variable).unwrap_or_default();
    // an unset or empty reference behaves like an absent key and reads DISABLE_SCHEMA_UPDATE
    betterlite_env::flag(variable, &text)?.map_or_else(|| environment_disable(lookup), Ok)
}
