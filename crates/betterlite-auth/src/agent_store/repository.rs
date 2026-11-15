use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::AgentIdentityStore;
use super::cache::ManagementObjectCache;
use super::error::AgentStoreError;
use super::records::{
    AgentRecords, AuthenticationFilter, IdentityRecord, IdentityRecords, InteractiveEnrollment,
    RetiredAgentRecord, RetiredAgentRecords, RetiredIdentityRecord, RetiredIdentityRecords,
    VerifiedSubjectRecords, VerifiedSubjectRow,
};
use crate::agent_policy::AgentResponse;

mod sql;
mod tests;

struct SqlAgentRecords {
    store: PostgresStore,
}

struct SqlIdentityRecords {
    store: PostgresStore,
}

struct SqlVerifiedSubjectRecords {
    store: PostgresStore,
}

struct SqlRetiredIdentityRecords {
    store: PostgresStore,
}

struct SqlRetiredAgentRecords {
    store: PostgresStore,
}

impl AgentIdentityStore {
    #[must_use]
    pub fn from_client(
        client: &PostgresStore,
        cache: Option<Arc<dyn ManagementObjectCache>>,
    ) -> Self {
        Self {
            agents: Arc::new(SqlAgentRecords {
                store: client.clone(),
            }),
            identities: Arc::new(SqlIdentityRecords {
                store: client.clone(),
            }),
            humans: Arc::new(SqlVerifiedSubjectRecords {
                store: client.clone(),
            }),
            retired: Some(Arc::new(SqlRetiredIdentityRecords {
                store: client.clone(),
            })),
            retired_agents: Some(Arc::new(SqlRetiredAgentRecords {
                store: client.clone(),
            })),
            cache,
            clock: Utc::now,
        }
    }
}

#[async_trait]
impl AgentRecords for SqlAgentRecords {
    async fn find_unique(&self, agent_id: &str) -> Result<Option<AgentResponse>, AgentStoreError> {
        let client = self
            .store
            .session()
            .await
            .map_err(AgentStoreError::Database)?;
        let row = client
            .query_opt(sql::AGENT_SQL, &[&agent_id])
            .await
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let payload: String = row
            .try_get(0)
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        serde_json::from_str::<AgentResponse>(&payload)
            .map(Some)
            .map_err(AgentStoreError::Row)
    }
}

#[async_trait]
impl IdentityRecords for SqlIdentityRecords {
    async fn find_unique(
        &self,
        provider: &str,
        tenant_id: &str,
        client_id: &str,
    ) -> Result<Option<IdentityRecord>, AgentStoreError> {
        let client = self
            .store
            .session()
            .await
            .map_err(AgentStoreError::Database)?;
        let row = client
            .query_opt(sql::IDENTITY_SQL, &[&provider, &tenant_id, &client_id])
            .await
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let agent_id: String = row
            .try_get("agent_id")
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        Ok(Some(IdentityRecord { agent_id }))
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
        let at = at.naive_utc();
        client
            .execute(
                sql::AUTHENTICATION_UPDATE_SQL,
                &[
                    &filter.agent_id,
                    &filter.revision,
                    &filter.active,
                    &filter.agent.enabled,
                    &filter.agent.identity_managed,
                    &at,
                ],
            )
            .await
            .map_err(|source| AgentStoreError::Database(backend(source)))
    }
}

#[async_trait]
impl VerifiedSubjectRecords for SqlVerifiedSubjectRecords {
    async fn find_unique(
        &self,
        issuer: &str,
        tenant_id: &str,
        oid: &str,
    ) -> Result<Option<VerifiedSubjectRow>, AgentStoreError> {
        let client = self
            .store
            .session()
            .await
            .map_err(AgentStoreError::Database)?;
        let row = client
            .query_opt(sql::VERIFIED_SUBJECT_SQL, &[&issuer, &tenant_id, &oid])
            .await
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let payload: String = row
            .try_get(0)
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        serde_json::from_str::<VerifiedSubjectRow>(&payload)
            .map(Some)
            .map_err(AgentStoreError::Row)
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
        let payload: String = row
            .try_get(0)
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        serde_json::from_str::<VerifiedSubjectRow>(&payload).map_err(AgentStoreError::Row)
    }
}

#[async_trait]
impl RetiredIdentityRecords for SqlRetiredIdentityRecords {
    async fn find_unique(
        &self,
        provider: &str,
        tenant_id: &str,
        client_id: &str,
    ) -> Result<Option<RetiredIdentityRecord>, AgentStoreError> {
        let client = self
            .store
            .session()
            .await
            .map_err(AgentStoreError::Database)?;
        let row = client
            .query_opt(
                sql::RETIRED_IDENTITY_SQL,
                &[&provider, &tenant_id, &client_id],
            )
            .await
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let binding_id: String = row
            .try_get("binding_id")
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        Ok(Some(RetiredIdentityRecord { binding_id }))
    }
}

#[async_trait]
impl RetiredAgentRecords for SqlRetiredAgentRecords {
    async fn find_unique(
        &self,
        agent_id: &str,
    ) -> Result<Option<RetiredAgentRecord>, AgentStoreError> {
        let client = self
            .store
            .session()
            .await
            .map_err(AgentStoreError::Database)?;
        let row = client
            .query_opt(sql::RETIRED_AGENT_SQL, &[&agent_id])
            .await
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let original_agent_id: String = row
            .try_get("original_agent_id")
            .map_err(|source| AgentStoreError::Database(backend(source)))?;
        Ok(Some(RetiredAgentRecord { original_agent_id }))
    }
}
