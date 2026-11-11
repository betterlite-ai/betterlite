use betterlite_db::migration_recovery::{
    DB_PUSH_ENV_VAR, REPLICA_IDENTITY_ENV_VAR, V2_RESOLVER_ENV_VAR, extras_boolean,
    resolve_v2_migration_resolver,
};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// Every input this section reads is an environment variable, so no YAML key belongs to it.
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub modes: Modes,
    pub set_replica_identity_full: bool,
}

#[derive(Debug, Clone)]
pub struct Modes {
    pub use_v2_migration_resolver: bool,
    pub use_prisma_db_push: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            use_v2_migration_resolver: true,
            use_prisma_db_push: false,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let resolver = lookup.lookup(V2_RESOLVER_ENV_VAR);
        Ok(Settings {
            modes: Modes {
                use_v2_migration_resolver: resolve_v2_migration_resolver(
                    false,
                    resolver.as_deref(),
                ),
                use_prisma_db_push: extras_boolean(lookup.lookup(DB_PUSH_ENV_VAR).as_deref()),
            },
            set_replica_identity_full: extras_boolean(
                lookup.lookup(REPLICA_IDENTITY_ENV_VAR).as_deref(),
            ),
        })
    }
}
