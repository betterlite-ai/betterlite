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

// Prisma's find_unique with the identity and object_permission includes, decoded as AgentResponse
const AGENT_SQL: &str = r#"SELECT jsonb_build_object(
    'agent_id', a.agent_id,
    'agent_name', a.agent_name,
    'litellm_params', a.litellm_params,
    'agent_card_params', a.agent_card_params,
    'identity', CASE WHEN i.agent_id IS NULL THEN NULL ELSE to_jsonb(i.*) END,
    'identity_managed', a.identity_managed,
    'enabled', a.enabled,
    'execution_mode', a.execution_mode,
    'object_permission', CASE WHEN p.object_permission_id IS NULL THEN NULL ELSE to_jsonb(p.*) END,
    'spend', a.spend,
    'tpm_limit', a.tpm_limit,
    'rpm_limit', a.rpm_limit,
    'session_tpm_limit', a.session_tpm_limit,
    'session_rpm_limit', a.session_rpm_limit,
    'static_headers', a.static_headers,
    'extra_headers', a.extra_headers,
    'access_group_ids', a.access_group_ids,
    'kill_switch', a.kill_switch,
    'created_at', a.created_at,
    'created_by', a.created_by,
    'updated_at', a.updated_at,
    'updated_by', a.updated_by
)::text
FROM "LiteLLM_AgentsTable" a
LEFT JOIN "LiteLLM_AgentIdentity" i ON i.agent_id = a.agent_id
LEFT JOIN "LiteLLM_ObjectPermissionTable" p ON p.object_permission_id = a.object_permission_id
WHERE a.agent_id = $1"#;

// the identity table's provider_tenant_id_client_id unique key
const IDENTITY_SQL: &str = r#"SELECT agent_id
FROM "LiteLLM_AgentIdentity"
WHERE provider = $1 AND tenant_id = $2 AND client_id = $3"#;

// update_many's where clause is the final binding check: revision, active and the nested agent filter
const AUTHENTICATION_UPDATE_SQL: &str = r#"UPDATE "LiteLLM_AgentIdentity" AS i
SET last_authenticated_at = $6
FROM "LiteLLM_AgentsTable" AS a
WHERE i.agent_id = a.agent_id
  AND i.agent_id = $1
  AND i.revision = $2
  AND i.active = $3
  AND a.enabled = $4
  AND a.identity_managed = $5"#;

// the verified-subject table's issuer_tenant_id_oid unique key
const VERIFIED_SUBJECT_SQL: &str = r#"SELECT to_jsonb(s.*)::text
FROM "LiteLLM_VerifiedSubject" s
WHERE s.issuer = $1 AND s.tenant_id = $2 AND s.oid = $3"#;

// Prisma's upsert with empty update data still returns the stored row, leaving it unchanged
const ENROLLMENT_UPSERT_SQL: &str = r#"INSERT INTO "LiteLLM_VerifiedSubject" (subject_id, issuer, tenant_id, oid, user_id, verified_via)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (issuer, tenant_id, oid) DO UPDATE SET user_id = "LiteLLM_VerifiedSubject".user_id
RETURNING to_jsonb("LiteLLM_VerifiedSubject".*)::text"#;

// the retired table's provider_tenant_id_client_id unique key
const RETIRED_IDENTITY_SQL: &str = r#"SELECT binding_id
FROM "LiteLLM_RetiredAgentIdentity"
WHERE provider = $1 AND tenant_id = $2 AND client_id = $3"#;

const RETIRED_AGENT_SQL: &str = r#"SELECT original_agent_id
FROM "LiteLLM_RetiredAgent"
WHERE original_agent_id = $1"#;

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
            .query_opt(AGENT_SQL, &[&agent_id])
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
            .query_opt(IDENTITY_SQL, &[&provider, &tenant_id, &client_id])
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
                AUTHENTICATION_UPDATE_SQL,
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
            .query_opt(VERIFIED_SUBJECT_SQL, &[&issuer, &tenant_id, &oid])
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
                ENROLLMENT_UPSERT_SQL,
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
            .query_opt(RETIRED_IDENTITY_SQL, &[&provider, &tenant_id, &client_id])
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
            .query_opt(RETIRED_AGENT_SQL, &[&agent_id])
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
