use std::time::Duration;

use betterlite_db::migration_ledger::timeouts::{
    DEFAULT_COMMAND_TIMEOUT, DEFAULT_DEPLOY_TIMEOUT, DEFAULT_LOCK_TIMEOUT,
    MIGRATION_LOCK_TIMEOUT_ENV_VAR, PRISMA_COMMAND_TIMEOUT_ENV_VAR, PRISMA_DEPLOY_TIMEOUT_ENV_VAR,
    prisma_migrate_deploy_timeout, timeout_from_env,
};
use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// Every input this component reads is an environment variable, so no YAML key belongs to this section.
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

// Boxed: Sections holds each Settings by value, and config snapshots live inside futures
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub inner: Box<Inner>,
}

#[derive(Debug, Clone)]
pub struct Inner {
    pub database_url: Option<SecretString>,
    pub direct_url: Option<SecretString>,
    pub migration_lock_timeout: Duration,
    pub prisma_command_timeout: Duration,
    pub prisma_migrate_deploy_timeout: Duration,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            database_url: None,
            direct_url: None,
            migration_lock_timeout: DEFAULT_LOCK_TIMEOUT,
            prisma_command_timeout: DEFAULT_COMMAND_TIMEOUT,
            prisma_migrate_deploy_timeout: DEFAULT_DEPLOY_TIMEOUT,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let command = lookup.lookup(PRISMA_COMMAND_TIMEOUT_ENV_VAR);
        let deploy = lookup.lookup(PRISMA_DEPLOY_TIMEOUT_ENV_VAR);
        Ok(Settings {
            inner: Box::new(Inner {
                database_url: lookup
                    .lookup("DATABASE_URL")
                    .filter(|text| !text.is_empty())
                    .map(SecretString::from),
                direct_url: lookup
                    .lookup("DIRECT_URL")
                    .filter(|text| !text.is_empty())
                    .map(SecretString::from),
                migration_lock_timeout: timeout_from_env(
                    MIGRATION_LOCK_TIMEOUT_ENV_VAR,
                    lookup.lookup(MIGRATION_LOCK_TIMEOUT_ENV_VAR).as_deref(),
                    DEFAULT_LOCK_TIMEOUT,
                ),
                prisma_command_timeout: timeout_from_env(
                    PRISMA_COMMAND_TIMEOUT_ENV_VAR,
                    command.as_deref(),
                    DEFAULT_COMMAND_TIMEOUT,
                ),
                prisma_migrate_deploy_timeout: prisma_migrate_deploy_timeout(
                    deploy.as_deref(),
                    command.as_deref(),
                ),
            }),
        })
    }
}
