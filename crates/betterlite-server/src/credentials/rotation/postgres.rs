use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::model_secrets::PreparedModel;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::resource_repository::config::{
    ConfigParam, ConfigRepository, ConfigStoredRow, ConfigUpdate,
};
use betterlite_db::resource_repository::credential::{
    CredentialError as RepositoryError, CredentialsRepository,
};
use betterlite_db::resource_repository::table::{KeyFilter, RowWrite};
use betterlite_db::resource_rows::credential::CredentialItem as CredentialRow;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_redis::config_sync::publish::redis_config_change_publisher as publish_change;
use betterlite_redis::config_sync::table_actions::TableActions as _;
use deadpool_postgres::Client as PooledClient;
use thiserror::Error;
use tokio::time::timeout;
use tokio_postgres::Error as PgError;
use tokio_postgres::error::SqlState;

use super::ENVIRONMENT_VARIABLES;
use super::store::RotationStore;
use crate::db_row_decrypt::rows::read_model_rows;

// prisma_client.db.tx(timeout=timedelta(minutes=2)): the rotation holds one transaction for the whole model table.
pub const MODEL_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(120);

// LiteLLM_ProxyModelTable(**_data) parses the prepared JSON text back into a dict before prisma.Json stores it, and @updatedAt stamps the row.
const UPDATE_MODEL_PARAMS_SQL: &str = "UPDATE \"LiteLLM_ProxyModelTable\" \
     SET \"litellm_params\" = ($2::text)::jsonb, \"updated_at\" = CURRENT_TIMESTAMP \
     WHERE \"model_id\" = $1";

// true is is_local: the setting ends with the transaction, so the pool's next borrower keeps its own deadline.
const STATEMENT_TIMEOUT_SQL: &str = "SELECT set_config('statement_timeout', $1, true)";

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
    #[error("the {0} did not finish within {1:?}")]
    Timeout(&'static str, Duration),
}

// The caller builds the client from the store it reads: a rotation never owns a pool of its own.
pub struct PostgresRotationStore<'db> {
    pub client: PrismaClient<'db>,
    pub model_timeout: Duration,
    pub acquire_timeout: Duration,
    pub query_timeout: Duration,
}

impl RotationStore for PostgresRotationStore<'_> {
    type Error = StoreError;

    async fn model_rows(&self) -> Result<Vec<ProxyModelTable>, StoreError> {
        let read = read_model_rows(Some(self.client));
        match timeout(self.query_timeout, read).await {
            Ok(found) => found.ok_or(StoreError::ModelRead),
            Err(_elapsed) => Err(StoreError::Timeout("model scan", self.query_timeout)),
        }
    }

    async fn save_model_params(&self, models: &[PreparedModel]) -> Result<(), StoreError> {
        let session = self.client.writer.session();
        let mut client = timeout(self.acquire_timeout, session)
            .await
            .map_err(|_elapsed| {
                StoreError::Timeout("connection acquisition", self.acquire_timeout)
            })?
            .map_err(StoreError::Database)?;
        let outcome = model_transaction(&mut client, models, self.model_timeout).await;
        if matches!(
            &outcome,
            Err(StoreError::ModelTimeout | StoreError::Timeout(..))
        ) {
            // a timed-out statement may still be running on the server, so the connection never goes back to the pool
            drop(PooledClient::take(client));
        }
        outcome
    }

    async fn config_rows(&self) -> Result<Vec<ConfigParam>, StoreError> {
        let repository = ConfigRepository::new(Some(self.client), true);
        let read = async {
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
        };
        timeout(self.query_timeout, read)
            .await
            .map_err(|_elapsed| StoreError::Timeout("config scan", self.query_timeout))?
    }

    async fn save_environment_variables(&self, values: JsonData) -> Result<(), StoreError> {
        let repository = ConfigRepository::new(Some(self.client), true);
        let update = ConfigUpdate {
            param_value: Some(values),
        };
        let sealed = async {
            let updated = repository
                .table()?
                .update(update, ENVIRONMENT_VARIABLES.to_owned())
                .await?;
            if updated.is_none() {
                return Err(StoreError::EnvironmentRowMissing);
            }
            Ok(())
        };
        timeout(self.query_timeout, sealed)
            .await
            .map_err(|_elapsed| StoreError::Timeout("environment write", self.query_timeout))?
    }

    async fn credential_rows(&self) -> Result<Vec<JsonData>, StoreError> {
        let repository = CredentialsRepository::new(Some(self.client), publish_change(None));
        match timeout(self.query_timeout, repository.find_all()).await {
            Ok(rows) => rows.map_err(StoreError::from),
            Err(_elapsed) => Err(StoreError::Timeout("credential scan", self.query_timeout)),
        }
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
        let repository = CredentialsRepository::new(Some(self.client), publish_change(None));
        // the row matched or not is not the rotation's business; the per-row warning already covers a failed write
        timeout(
            self.query_timeout,
            repository.update_by_name(name, RowWrite(record)),
        )
        .await
        .map_err(|_elapsed| StoreError::Timeout("credential write", self.query_timeout))?
        .map_err(StoreError::Credentials)?;
        Ok(())
    }
}

// The whole model write between BEGIN and COMMIT, with the server deadline on each statement and a client bound on the batch.
async fn model_transaction(
    client: &mut PooledClient,
    models: &[PreparedModel],
    deadline: Duration,
) -> Result<(), StoreError> {
    let started = timeout(deadline, client.transaction())
        .await
        .map_err(|_elapsed| StoreError::Timeout("transaction start", deadline))?;
    let transaction = started.map_err(|source| StoreError::Database(backend(source)))?;
    let milliseconds = statement_millis(deadline);
    let setting = format!("{milliseconds}ms");
    let write = async {
        transaction
            .execute(STATEMENT_TIMEOUT_SQL, &[&setting])
            .await
            .map_err(model_statement_error)?;
        for model in models {
            transaction
                .execute(
                    UPDATE_MODEL_PARAMS_SQL,
                    &[&model.model_id, &model.litellm_params],
                )
                .await
                .map_err(model_statement_error)?;
        }
        transaction.commit().await.map_err(model_statement_error)
    };
    // the client waits twice the server deadline, so the server's own cancellation normally answers first
    match timeout(deadline.saturating_add(deadline), write).await {
        Ok(result) => result,
        Err(_elapsed) => Err(StoreError::ModelTimeout),
    }
}

// PostgreSQL treats zero as unlimited and refuses a statement_timeout above INT_MAX milliseconds.
pub(super) fn statement_millis(deadline: Duration) -> u128 {
    deadline
        .as_millis()
        .clamp(1, u128::from(i32::MAX.unsigned_abs()))
}

// set_config makes the transaction deadline a server-side statement_timeout, so a cancelled statement reports the deadline.
fn model_statement_error(source: PgError) -> StoreError {
    if source.code() == Some(&SqlState::QUERY_CANCELED) {
        return StoreError::ModelTimeout;
    }
    StoreError::Database(backend(source))
}
