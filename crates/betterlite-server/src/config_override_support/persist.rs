use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::bindings::ConfigOverrides;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};

use super::models::OverrideKind;
use crate::env_value_codec::{EnvValueCodecError, encrypt_env};

const UPSERT_SQL: &str = "INSERT INTO \"LiteLLM_ConfigOverrides\" (config_type, config_value, updated_at) \
     VALUES ($1, $2::text::jsonb, now()) ON CONFLICT (config_type) \
     DO UPDATE SET config_value = EXCLUDED.config_value, updated_at = now()";

#[derive(Debug, thiserror::Error)]
pub enum ConfigOverrideStoreError {
    #[error(transparent)]
    Table(#[from] TableError),
}

#[derive(Debug, thiserror::Error)]
pub enum PersistOverrideError {
    #[error(transparent)]
    Encrypt(#[from] EnvValueCodecError),
    #[error(transparent)]
    Store(#[from] ConfigOverrideStoreError),
    #[error("the encrypted config override is not serializable: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[async_trait]
pub trait ConfigOverrideStore: Send + Sync {
    async fn upsert_config_override(
        &self,
        config_type: &str,
        config_value: &str,
    ) -> Result<(), ConfigOverrideStoreError>;
}

pub struct DatabaseConfigOverrideStore {
    pub store: Arc<PostgresStore>,
    pub availability: Availability,
}

#[async_trait]
impl ConfigOverrideStore for DatabaseConfigOverrideStore {
    async fn upsert_config_override(
        &self,
        config_type: &str,
        config_value: &str,
    ) -> Result<(), ConfigOverrideStoreError> {
        let client = PrismaClient {
            writer: self.store.as_ref(),
            reader: self.store.as_ref(),
            availability: &self.availability,
        };
        let repository = PrismaTableRepository::new(
            Some(client),
            redis_config_change_publisher(None),
            ConfigOverrides,
        );
        let session = repository
            .access()
            .map_err(TableError::from)?
            .write
            .session()
            .await
            .map_err(TableError::from)?;
        session
            .execute(UPSERT_SQL, &[&config_type, &config_value])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(())
    }
}

pub async fn persist_cyberark_config<S, N, D>(
    store: &S,
    cipher: &StoredCipher<N, D>,
    config: &JsonMap,
) -> Result<JsonMap, PersistOverrideError>
where
    S: ConfigOverrideStore + ?Sized,
    N: NonceSource + Send + Sync,
    D: SecretDiagnostics + Send + Sync,
{
    let encrypted = encrypt_env(config, cipher, None)?;
    let config_value = serde_json::to_string(&encrypted)?;
    store
        .upsert_config_override(OverrideKind::CyberArk.as_str(), &config_value)
        .await?;
    Ok(encrypted)
}
