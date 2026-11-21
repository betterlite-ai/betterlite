use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::config::{ConfigError, ConfigRepository};
use thiserror::Error;

pub const GENERAL_SETTINGS: &str = "general_settings";

pub const COORDINATION_REDIS: &str = "coordination_redis";

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("the general_settings row must be a JSON object")]
    GeneralSettingsNotAnObject,
    #[error("the proxy config state must be a JSON object")]
    ConfigStateNotAnObject,
}

// The persisted general_settings row; no store stands for a deployment without a database.
#[async_trait]
pub trait GeneralSettingsStore: Send + Sync {
    async fn general_settings(&self) -> Result<Option<JsonData>, ConfigError>;
}

pub struct DatabaseGeneralSettingsStore {
    pub store: Arc<PostgresStore>,
    pub availability: Availability,
}

impl DatabaseGeneralSettingsStore {
    // litellm builds the repository with use_writer=True, so the row is read from the writer too.
    fn repository(&self) -> ConfigRepository<'_> {
        let client = PrismaClient {
            writer: self.store.as_ref(),
            reader: self.store.as_ref(),
            availability: &self.availability,
        };
        ConfigRepository::new(Some(client), true)
    }
}

#[async_trait]
impl GeneralSettingsStore for DatabaseGeneralSettingsStore {
    async fn general_settings(&self) -> Result<Option<JsonData>, ConfigError> {
        let parameter = self.repository().get_param(GENERAL_SETTINGS).await?;
        Ok(parameter.map(|param| param.param_value))
    }
}

// litellm's get_persisted_coordination_redis_settings: the database block, read at startup and before a save.
pub async fn persisted_settings(
    store: Option<&dyn GeneralSettingsStore>,
) -> Result<Option<JsonValue>, SettingsError> {
    let general = general_settings(store).await?;
    Ok(general.as_ref().and_then(coordination_block))
}

// litellm's _current_coordination_redis_settings: the persisted row wins over the yaml config state.
pub async fn current_settings(
    store: Option<&dyn GeneralSettingsStore>,
    config_state: &JsonValue,
) -> Result<Option<JsonValue>, SettingsError> {
    if let Some(persisted) = persisted_settings(store).await? {
        return Ok(Some(persisted));
    }
    let JsonValue::Object(members) = config_state else {
        return Err(SettingsError::ConfigStateNotAnObject);
    };
    Ok(members.get(GENERAL_SETTINGS).and_then(coordination_block))
}

// No database, a missing row and a null value all mean the same empty section.
async fn general_settings(
    store: Option<&dyn GeneralSettingsStore>,
) -> Result<Option<JsonValue>, SettingsError> {
    let Some(reader) = store else {
        return Ok(None);
    };
    let Some(row) = reader.general_settings().await? else {
        return Ok(None);
    };
    match row {
        JsonData::Null => Ok(None),
        JsonData::Object(members) => Ok(Some(JsonValue::Object(
            members
                .into_iter()
                .map(|(key, value)| (key, json_value(value)))
                .collect(),
        ))),
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Array(_) => {
            Err(SettingsError::GeneralSettingsNotAnObject)
        }
    }
}

// A dictionary-valued coordination_redis is a block even when empty; every other value is no block at all.
pub(crate) fn coordination_block(general: &JsonValue) -> Option<JsonValue> {
    let JsonValue::Object(members) = general else {
        return None;
    };
    members
        .get(COORDINATION_REDIS)
        .filter(|value| value.is_object())
        .cloned()
}

fn json_value(data: JsonData) -> JsonValue {
    match data {
        JsonData::Null => JsonValue::Null,
        JsonData::Bool(flag) => JsonValue::Bool(flag),
        JsonData::Number(number) => JsonValue::Number(number),
        JsonData::String(text) => JsonValue::String(text),
        JsonData::Array(items) => JsonValue::Array(items.into_iter().map(json_value).collect()),
        JsonData::Object(members) => JsonValue::Object(
            members
                .into_iter()
                .map(|(key, value)| (key, json_value(value)))
                .collect(),
        ),
    }
}
