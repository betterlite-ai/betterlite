use std::io;
use std::path::PathBuf;

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::postgres_store::errors::DbError;
use serde::Serialize;

use crate::env_value_codec::EnvValueCodecError;

// LiteLLM's HTTPException detail for a write to a key the config file owns: the routes send it as a 400 body
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfigOwnedDetail {
    pub error: String,
    pub keys: Vec<String>,
    pub section: String,
    pub stored_database_values_ignored: Vec<String>,
    pub resolution: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigStateError {
    #[error("{}", .0.error)]
    ConfigOwned(Box<ConfigOwnedDetail>),
    #[error("{field} must be {expected}")]
    InvalidSetting {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the stored {section} config row is not a JSON object")]
    InvalidStoredSection { section: &'static str },
    #[error("team_id missing from team: {team}")]
    MissingTeamId { team: JsonValue },
    #[error("the config file {path} could not be written")]
    WriteConfigFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the config could not be serialized as YAML")]
    SerializeConfig {
        #[source]
        source: serde_saphyr::SerializeError,
    },
    #[error("the config value could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the stored config value could not be decoded")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the config database write failed")]
    Database {
        #[source]
        source: DbError,
    },
    #[error(transparent)]
    EnvironmentValue(#[from] EnvValueCodecError),
}
