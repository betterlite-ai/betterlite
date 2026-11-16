use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use super::AgentIdentityStore;
use super::cache::ManagementObjectCache;
use super::error::AgentStoreError;
use super::records::{
    AgentRecords, AuthenticationFilter, BindingKey, FindResult, IdentityRecord, IdentityRecords,
    InteractiveEnrollment, RetiredAgentRecord, RetiredAgentRecords, RetiredIdentityRecord,
    RetiredIdentityRecords, SubjectKey, VerifiedSubjectRecords, VerifiedSubjectRow,
};
use crate::agent_policy::AgentResponse;

mod sql;
mod tests;

// one database handle serves all five repositories, each lookup through its own statement
struct SqlRecords {
    store: PostgresStore,
}

impl AgentIdentityStore {
    #[must_use]
    pub fn from_client(
        client: &PostgresStore,
        cache: Option<Arc<dyn ManagementObjectCache>>,
    ) -> Self {
        Self {
            agents: Arc::new(SqlRecords {
                store: client.clone(),
            }),
            identities: Arc::new(SqlRecords {
                store: client.clone(),
            }),
            humans: Arc::new(SqlRecords {
                store: client.clone(),
            }),
            retired: Some(Arc::new(SqlRecords {
                store: client.clone(),
            })),
            retired_agents: Some(Arc::new(SqlRecords {
                store: client.clone(),
            })),
            cache,
            clock: Utc::now,
        }
    }
}

async fn look_up(
    store: &PostgresStore,
    statement: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<Option<Row>, AgentStoreError> {
    let client = store.session().await.map_err(AgentStoreError::Database)?;
    client
        .query_opt(statement, parameters)
        .await
        .map_err(|source| AgentStoreError::Database(backend(source)))
}

// each composite key names its columns in the order its statement binds them
trait LookupParameters {
    fn parameters(&self) -> [&(dyn ToSql + Sync); 3];
}

impl LookupParameters for BindingKey<'_> {
    fn parameters(&self) -> [&(dyn ToSql + Sync); 3] {
        [&self.provider, &self.tenant_id, &self.client_id]
    }
}

impl LookupParameters for SubjectKey<'_> {
    fn parameters(&self) -> [&(dyn ToSql + Sync); 3] {
        [&self.issuer, &self.tenant_id, &self.oid]
    }
}

fn decode_json_row<Record>(row: &Row) -> Result<Record, AgentStoreError>
where
    Record: DeserializeOwned,
{
    let payload: String = row
        .try_get(0)
        .map_err(|source| AgentStoreError::Database(backend(source)))?;
    serde_json::from_str(&payload).map_err(AgentStoreError::Row)
}

fn decode_identity_row(row: &Row) -> Result<IdentityRecord, AgentStoreError> {
    let agent_id = row
        .try_get("agent_id")
        .map_err(|source| AgentStoreError::Database(backend(source)))?;
    Ok(IdentityRecord { agent_id })
}

fn decode_retired_identity_row(row: &Row) -> Result<RetiredIdentityRecord, AgentStoreError> {
    let binding_id = row
        .try_get("binding_id")
        .map_err(|source| AgentStoreError::Database(backend(source)))?;
    Ok(RetiredIdentityRecord { binding_id })
}

fn decode_retired_agent_row(row: &Row) -> Result<RetiredAgentRecord, AgentStoreError> {
    let original_agent_id = row
        .try_get("original_agent_id")
        .map_err(|source| AgentStoreError::Database(backend(source)))?;
    Ok(RetiredAgentRecord { original_agent_id })
}

#[async_trait]
impl AgentRecords for SqlRecords {
    async fn find_unique(&self, agent_id: &str) -> FindResult<AgentResponse> {
        let row = look_up(&self.store, sql::AGENT_SQL, &[&agent_id]).await?;
        row.as_ref()
            .map(decode_json_row::<AgentResponse>)
            .transpose()
    }
}

#[async_trait]
impl IdentityRecords for SqlRecords {
    async fn find_unique(&self, key: &BindingKey<'_>) -> FindResult<IdentityRecord> {
        let row = look_up(&self.store, sql::IDENTITY_SQL, &key.parameters()).await?;
        row.as_ref().map(decode_identity_row).transpose()
    }

    async fn update_many(
        &self,
        filter: &AuthenticationFilter<'_>,
        at: DateTime<Utc>,
    ) -> Result<u64, AgentStoreError> {
        let client = self
            .store
            .session()
            .await
            .map_err(AgentStoreError::Database)?;
        let instant = at.naive_utc();
        client
            .execute(
                sql::AUTHENTICATION_UPDATE_SQL,
                &[
                    &filter.agent_id,
                    &filter.revision,
                    &filter.active,
                    &filter.agent.enabled,
                    &filter.agent.identity_managed,
                    &instant,
                ],
            )
            .await
            .map_err(|source| AgentStoreError::Database(backend(source)))
    }
}

#[async_trait]
impl VerifiedSubjectRecords for SqlRecords {
    async fn find_unique(&self, key: &SubjectKey<'_>) -> FindResult<VerifiedSubjectRow> {
        let row = look_up(&self.store, sql::VERIFIED_SUBJECT_SQL, &key.parameters()).await?;
        row.as_ref()
            .map(decode_json_row::<VerifiedSubjectRow>)
            .transpose()
    }

    async fn upsert(
        &self,
        enrollment: &InteractiveEnrollment<'_>,
    ) -> Result<VerifiedSubjectRow, AgentStoreError> {
        let client = self
            .store
            .session()
            .await
            .map_err(AgentStoreError::Database)?;
        let subject_id = Uuid::new_v4().to_string();
        let row = client
            .query_one(
                sql::ENROLLMENT_UPSERT_SQL,
                &[
                    &subject_id,
                    &enrollment.issuer,
                    &enrollment.tenant_id,
                    &enrollment.oid,
                    &enrollment.user_id,
                    &enrollment.verified_via,
                ],
            )
            .await
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        decode_json_row(&row)
    }
}

#[async_trait]
impl RetiredIdentityRecords for SqlRecords {
    async fn find_unique(&self, key: &BindingKey<'_>) -> FindResult<RetiredIdentityRecord> {
        let row = look_up(&self.store, sql::RETIRED_IDENTITY_SQL, &key.parameters()).await?;
        row.as_ref().map(decode_retired_identity_row).transpose()
    }
}

#[async_trait]
impl RetiredAgentRecords for SqlRecords {
    async fn find_unique(&self, agent_id: &str) -> FindResult<RetiredAgentRecord> {
        let row = look_up(&self.store, sql::RETIRED_AGENT_SQL, &[&agent_id]).await?;
        row.as_ref().map(decode_retired_agent_row).transpose()
    }
}
