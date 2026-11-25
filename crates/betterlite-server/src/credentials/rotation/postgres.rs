use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::model_secrets::PreparedModel;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::config::{
    ConfigParam, ConfigRepository, ConfigStoredRow, ConfigUpdate,
};
use betterlite_db::resource_repository::credential::{
    CredentialError as RepositoryError, CredentialsRepository,
};
use betterlite_db::resource_repository::table::{KeyFilter, RowWrite};
use betterlite_db::resource_rows::credential::CredentialItem as CredentialRow;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_redis::config_sync::table_actions::TableActions as _;
use thiserror::Error;
use tokio::time::timeout;

use super::ENVIRONMENT_VARIABLES;
use super::store::RotationStore;
use crate::db_row_decrypt::rows::read_model_rows;

// prisma_client.db.tx(timeout=timedelta(minutes=2)): the rotation holds one transaction for the whole model table.
pub const MODEL_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(120);

// LiteLLM_ProxyModelTable(**_data) parses the prepared JSON text back into a dict before prisma.Json stores it, and @updatedAt stamps the row.
const UPDATE_MODEL_PARAMS_SQL: &str = "UPDATE \"LiteLLM_ProxyModelTable\" \
     SET \"litellm_params\" = ($2::text)::jsonb, \"updated_at\" = CURRENT_TIMESTAMP \
     WHERE \"model_id\" = $1";

#[derive(Debug, Error)]
pub enum StoreError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Credentials(#[from] RepositoryError),
    #[error("the stored model rows could not be read")]
    ModelRead,
    #[error("the model parameter update did not finish within its limit")]
    ModelTimeout,
    #[error("the environment_variables row is gone")]
    EnvironmentRowMissing,
}

pub struct PostgresRotationStore {
    pub store: PostgresStore,
    pub availability: Availability,
    pub model_timeout: Duration,
}

impl PostgresRotationStore {
    #[must_use]
    pub fn new(store: PostgresStore, availability: Availability) -> Self {
        Self {
            store,
            availability,
            model_timeout: MODEL_TRANSACTION_TIMEOUT,
        }
    }

    // read_model_rows pins to the writer while it is healthy: a lagging replica would hand back ciphertext from before the last rotation.
    const fn prisma(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }
}

impl RotationStore for PostgresRotationStore {
    type Error = StoreError;

    async fn model_rows(&self) -> Result<Vec<ProxyModelTable>, StoreError> {
        read_model_rows(Some(self.prisma()))
            .await
            .ok_or(StoreError::ModelRead)
    }

    async fn save_model_params(&self, models: &[PreparedModel]) -> Result<(), StoreError> {
        let mut client = self.store.session().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| StoreError::Database(backend(source)))?;
        let write = async {
            for model in models {
                transaction
                    .execute(
                        UPDATE_MODEL_PARAMS_SQL,
                        &[&model.model_id, &model.litellm_params],
                    )
                    .await
                    .map_err(|source| StoreError::Database(backend(source)))?;
            }
            transaction
                .commit()
                .await
                .map_err(|source| StoreError::Database(backend(source)))
        };
        timeout(self.model_timeout, write)
            .await
            .map_err(|_| StoreError::ModelTimeout)?
    }

    async fn config_rows(&self) -> Result<Vec<ConfigParam>, StoreError> {
        let repository = ConfigRepository::new(Some(self.prisma()), true);
        let rows = repository
            .table()?
            .as_ref()
            .find::<ConfigStoredRow>(KeyFilter::All)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| ConfigParam {
                param_name: row.param_name,
                param_value: row.param_value,
            })
            .collect())
    }

    async fn save_environment_variables(&self, values: JsonData) -> Result<(), StoreError> {
        let repository = ConfigRepository::new(Some(self.prisma()), true);
        let update = ConfigUpdate {
            param_value: Some(values),
        };
        let updated = repository
            .table()?
            .update(update, ENVIRONMENT_VARIABLES.to_owned())
            .await?;
        if updated.is_none() {
            return Err(StoreError::EnvironmentRowMissing);
        }
        Ok(())
    }

    async fn credential_rows(&self) -> Result<Vec<JsonData>, StoreError> {
        let repository =
            CredentialsRepository::new(Some(self.prisma()), redis_config_change_publisher(None));
        Ok(repository.find_all().await?)
    }

    async fn update_credential(
        &self,
        name: &str,
        row: &CredentialRow,
        actor: Option<&str>,
    ) -> Result<(), StoreError> {
        let mut record: BTreeMap<String, JsonData> = BTreeMap::new();
        record.insert(
            "credential_name".to_owned(),
            JsonData::String(row.fields.base.credential_name.clone()),
        );
        record.insert(
            "credential_info".to_owned(),
            JsonData::Object(row.fields.base.credential_info.0.clone()),
        );
        record.insert(
            "credential_values".to_owned(),
            JsonData::Object(row.fields.credential_values.0.clone()),
        );
        record.insert(
            "updated_by".to_owned(),
            actor.map_or(JsonData::Null, |id| JsonData::String(id.to_owned())),
        );
        let repository =
            CredentialsRepository::new(Some(self.prisma()), redis_config_change_publisher(None));
        // the row matched or not is not the rotation's business; the per-row warning already covers a failed write
        repository
            .update_by_name(name, RowWrite(record))
            .await
            .map_err(StoreError::Credentials)?;
        Ok(())
    }
}
