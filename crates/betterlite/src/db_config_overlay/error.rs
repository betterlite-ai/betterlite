use std::{io, path::PathBuf};

use betterlite_config::loader::ConfigError;
use betterlite_config::sections::{includes::IncludeError, settings::ConfigOwnedKeyError};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::table_ops::rows::RowError;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use betterlite_server::bucket_config::error::BucketConfigError;
use betterlite_server::plugin_routes::error::PluginError;

use crate::cleanup_schedule::error::CleanupScheduleError;
use crate::config_boot::raw_secret_manager::RawSecretManagerError;

// LiteLLM's caller only logs a failed route registration, so the installer logs its own cause
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the pass-through endpoints could not be installed")]
pub struct EndpointInstallError;

#[derive(Debug, thiserror::Error)]
pub enum OverlayError {
    #[error(transparent)]
    Bucket(#[from] BucketConfigError),
    #[error("no reader is configured for the config bucket")]
    NoBucketReader,
    #[error("Unable to load config from given source.")]
    NothingLoaded,
    #[error("Config file not found: {}", .path.display())]
    ConfigFileMissing { path: PathBuf },
    #[error("the config file {} could not be read", .path.display())]
    ReadConfigFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the config file {} is not valid YAML", .path.display())]
    ConfigYaml {
        path: PathBuf,
        #[source]
        source: Box<serde_saphyr::Error>,
    },
    #[error("Config cannot be None or Empty.")]
    ConfigEmpty,
    #[error("the config file {} does not hold a mapping", .path.display())]
    ConfigNotMapping { path: PathBuf },
    #[error(transparent)]
    Includes(#[from] IncludeError),
    #[error("the config rows could not be read")]
    Database {
        #[source]
        source: DbError<RowError>,
    },
    #[error(transparent)]
    ConfigOwned(#[from] ConfigOwnedKeyError),
    #[error("the key management settings could not be read")]
    SecretSettings {
        #[source]
        source: serde_json::Error,
    },
    #[error("the key management settings could not be resolved")]
    SecretProjection {
        #[source]
        source: ConfigError,
    },
    #[error(transparent)]
    SecretManager(#[from] RawSecretManagerError),
    #[error(transparent)]
    SecretLookup(#[from] SecretLookupError),
    #[error("the off-runtime config worker could not be started")]
    WorkerStart {
        #[source]
        source: io::Error,
    },
    #[error("the off-runtime config worker is unavailable")]
    WorkerUnavailable,
    #[error(transparent)]
    CleanupSchedule(#[from] CleanupScheduleError),
    #[error(transparent)]
    Plugins(#[from] PluginError),
    #[error("pass_through_endpoints entry {index} is not an object")]
    PassThroughEntry { index: usize },
    #[error("the pass-through endpoints could not be installed")]
    PassThroughInstall {
        #[source]
        source: EndpointInstallError,
    },
}
