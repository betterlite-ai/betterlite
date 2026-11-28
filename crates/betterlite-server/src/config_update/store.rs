use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::settings::{JsonValue, Section};
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_db::table_ops::ConfigParamInvalidator;

use super::SettingsMap;
use crate::proxy_config_state::database::postgres::{READ_CONFIG_SQL, decode_section_values};
use crate::proxy_config_state::database::{ConfigStateDatabase as _, DatabaseConfigState};
use crate::proxy_config_state::error::ConfigStateError;

// The read, upsert and seal half of ProxyConfig that update_config needs, so the route can run against a fake in tests
#[async_trait]
pub trait ConfigUpdateStore: Send + Sync {
    async fn read_section(&self, section: Section) -> Result<SettingsMap, ConfigStateError>;

    async fn upsert_section(
        &self,
        section: Section,
        values: &SettingsMap,
    ) -> Result<(), ConfigStateError>;

    fn seal_environment(&self, values: &SettingsMap) -> Result<SettingsMap, ConfigStateError>;
}

pub struct PostgresConfigUpdateStore {
    pub store: Arc<PostgresStore>,
    pub cipher: Arc<DbCipher>,
    pub invalidator: Arc<dyn ConfigParamInvalidator>,
}

#[async_trait]
impl ConfigUpdateStore for PostgresConfigUpdateStore {
    // _read_section: no row and a null param_value both read as {}
    async fn read_section(&self, section: Section) -> Result<SettingsMap, ConfigStateError> {
        let client = self
            .store
            .session()
            .await
            .map_err(|source| ConfigStateError::Database { source })?;
        let found = client
            .query_opt(READ_CONFIG_SQL, &[&section.name()])
            .await
            .map_err(|source| ConfigStateError::Database {
                source: backend(source),
            })?;
        let Some(row) = found else {
            return Ok(SettingsMap::new());
        };
        let encoded: Option<String> =
            row.try_get(0)
                .map_err(|source| ConfigStateError::Database {
                    source: backend(source),
                })?;
        let Some(stored) = encoded else {
            return Ok(SettingsMap::new());
        };
        decode_section_values(section, &stored)
    }

    // _upsert_section: one row per section, then invalidate_config_param for it
    async fn upsert_section(
        &self,
        section: Section,
        values: &SettingsMap,
    ) -> Result<(), ConfigStateError> {
        let database = DatabaseConfigState {
            store: self.store.as_ref(),
            cipher: self.cipher.as_ref(),
            invalidator: self.invalidator.as_ref(),
        };
        let row = JsonValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        );
        database
            .insert_config(&[(section.name().to_owned(), row)])
            .await
    }

    fn seal_environment(&self, values: &SettingsMap) -> Result<SettingsMap, ConfigStateError> {
        let database = DatabaseConfigState {
            store: self.store.as_ref(),
            cipher: self.cipher.as_ref(),
            invalidator: self.invalidator.as_ref(),
        };
        database.seal_environment(values)
    }
}
