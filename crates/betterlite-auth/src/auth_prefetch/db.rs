use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;

use super::entries::AuthObjectRefs;
use super::rows::{RowKind, RowValues};

// the pinned SELECT keeps relations null and COALESCEs every scalar list to [], as prisma reads them
const ROWS_SQL: &str = r#"SELECT
  (SELECT to_jsonb(u) || jsonb_build_object(
    'teams', COALESCE(to_jsonb(u.teams), '[]'::jsonb),
    'models', COALESCE(to_jsonb(u.models), '[]'::jsonb),
    'allowed_cache_controls', COALESCE(to_jsonb(u.allowed_cache_controls), '[]'::jsonb),
    'policies', COALESCE(to_jsonb(u.policies), '[]'::jsonb),
    'organization_memberships', COALESCE((
      SELECT jsonb_agg(to_jsonb(om)) FROM "LiteLLM_OrganizationMembership" om WHERE om.user_id = u.user_id
    ), '[]'::jsonb)
  ) FROM "LiteLLM_UserTable" u WHERE u.user_id = $1)::text,
  (SELECT to_jsonb(t) || jsonb_build_object(
    'admins', COALESCE(to_jsonb(t.admins), '[]'::jsonb),
    'members', COALESCE(to_jsonb(t.members), '[]'::jsonb),
    'models', COALESCE(to_jsonb(t.models), '[]'::jsonb),
    'team_member_permissions', COALESCE(to_jsonb(t.team_member_permissions), '[]'::jsonb),
    'access_group_ids', COALESCE(to_jsonb(t.access_group_ids), '[]'::jsonb),
    'policies', COALESCE(to_jsonb(t.policies), '[]'::jsonb),
    'default_team_member_models', COALESCE(to_jsonb(t.default_team_member_models), '[]'::jsonb),
    'litellm_model_table', (
      SELECT (to_jsonb(m) - 'aliases') || jsonb_build_object('model_aliases', m.aliases)
      FROM "LiteLLM_ModelTable" m WHERE m.id = t.model_id
    ),
    'object_permission', (SELECT to_jsonb(op) || jsonb_build_object(
      'mcp_servers', COALESCE(to_jsonb(op.mcp_servers), '[]'::jsonb),
      'mcp_access_groups', COALESCE(to_jsonb(op.mcp_access_groups), '[]'::jsonb),
      'mcp_toolsets', COALESCE(to_jsonb(op.mcp_toolsets), '[]'::jsonb),
      'blocked_tools', COALESCE(to_jsonb(op.blocked_tools), '[]'::jsonb),
      'vector_stores', COALESCE(to_jsonb(op.vector_stores), '[]'::jsonb),
      'agents', COALESCE(to_jsonb(op.agents), '[]'::jsonb),
      'agent_access_groups', COALESCE(to_jsonb(op.agent_access_groups), '[]'::jsonb),
      'models', COALESCE(to_jsonb(op.models), '[]'::jsonb),
      'search_tools', COALESCE(to_jsonb(op.search_tools), '[]'::jsonb),
      'skills', COALESCE(to_jsonb(op.skills), '[]'::jsonb)
    ) FROM "LiteLLM_ObjectPermissionTable" op WHERE op.object_permission_id = t.object_permission_id)
  ) FROM "LiteLLM_TeamTable" t WHERE t.team_id = $2)::text,
  (SELECT to_jsonb(tm) || jsonb_build_object('litellm_budget_table', (SELECT to_jsonb(b) || jsonb_build_object(
    'allowed_models', COALESCE(to_jsonb(b.allowed_models), '[]'::jsonb)
  ) FROM "LiteLLM_BudgetTable" b WHERE b.budget_id = tm.budget_id))
  FROM "LiteLLM_TeamMembership" tm WHERE tm.user_id = $3 AND tm.team_id = $2)::text,
  (SELECT to_jsonb(o) || jsonb_build_object(
    'models', COALESCE(to_jsonb(o.models), '[]'::jsonb),
    'litellm_budget_table', (SELECT to_jsonb(b) || jsonb_build_object(
      'allowed_models', COALESCE(to_jsonb(b.allowed_models), '[]'::jsonb)
    ) FROM "LiteLLM_BudgetTable" b WHERE b.budget_id = o.budget_id),
    'object_permission', (SELECT to_jsonb(op) || jsonb_build_object(
      'mcp_servers', COALESCE(to_jsonb(op.mcp_servers), '[]'::jsonb),
      'mcp_access_groups', COALESCE(to_jsonb(op.mcp_access_groups), '[]'::jsonb),
      'mcp_toolsets', COALESCE(to_jsonb(op.mcp_toolsets), '[]'::jsonb),
      'blocked_tools', COALESCE(to_jsonb(op.blocked_tools), '[]'::jsonb),
      'vector_stores', COALESCE(to_jsonb(op.vector_stores), '[]'::jsonb),
      'agents', COALESCE(to_jsonb(op.agents), '[]'::jsonb),
      'agent_access_groups', COALESCE(to_jsonb(op.agent_access_groups), '[]'::jsonb),
      'models', COALESCE(to_jsonb(op.models), '[]'::jsonb),
      'search_tools', COALESCE(to_jsonb(op.search_tools), '[]'::jsonb),
      'skills', COALESCE(to_jsonb(op.skills), '[]'::jsonb)
    ) FROM "LiteLLM_ObjectPermissionTable" op WHERE op.object_permission_id = o.object_permission_id)
  ) FROM "LiteLLM_OrganizationTable" o WHERE o.organization_id = $4)::text,
  (SELECT to_jsonb(p) || jsonb_build_object(
    'models', COALESCE(to_jsonb(p.models), '[]'::jsonb),
    'litellm_budget_table', (SELECT to_jsonb(b) || jsonb_build_object(
      'allowed_models', COALESCE(to_jsonb(b.allowed_models), '[]'::jsonb)
    ) FROM "LiteLLM_BudgetTable" b WHERE b.budget_id = p.budget_id),
    'object_permission', (SELECT to_jsonb(op) || jsonb_build_object(
      'mcp_servers', COALESCE(to_jsonb(op.mcp_servers), '[]'::jsonb),
      'mcp_access_groups', COALESCE(to_jsonb(op.mcp_access_groups), '[]'::jsonb),
      'mcp_toolsets', COALESCE(to_jsonb(op.mcp_toolsets), '[]'::jsonb),
      'blocked_tools', COALESCE(to_jsonb(op.blocked_tools), '[]'::jsonb),
      'vector_stores', COALESCE(to_jsonb(op.vector_stores), '[]'::jsonb),
      'agents', COALESCE(to_jsonb(op.agents), '[]'::jsonb),
      'agent_access_groups', COALESCE(to_jsonb(op.agent_access_groups), '[]'::jsonb),
      'models', COALESCE(to_jsonb(op.models), '[]'::jsonb),
      'search_tools', COALESCE(to_jsonb(op.search_tools), '[]'::jsonb),
      'skills', COALESCE(to_jsonb(op.skills), '[]'::jsonb)
    ) FROM "LiteLLM_ObjectPermissionTable" op WHERE op.object_permission_id = p.object_permission_id)
  ) FROM "LiteLLM_ProjectTable" p WHERE p.project_id = $5)::text"#;

#[derive(Debug, thiserror::Error)]
pub enum PrefetchDatabaseError {
    #[error("the auth prefetch query failed")]
    Database(#[source] DbError),
    #[error("the auth prefetch {row} column is not readable JSON")]
    Column {
        row: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

// one query for every requested kind; the kinds not requested bind SQL NULL
#[async_trait]
pub trait PrefetchDatabase: Send + Sync {
    async fn fetch_rows(
        &self,
        refs: &AuthObjectRefs,
        kinds: &[RowKind],
    ) -> Result<RowValues, PrefetchDatabaseError>;
}

// the raw SELECT reads the writer, as LiteLLM's prisma client does: a replica can lag the row
pub struct WriterRows {
    pub store: PostgresStore,
}

#[async_trait]
impl PrefetchDatabase for WriterRows {
    async fn fetch_rows(
        &self,
        refs: &AuthObjectRefs,
        kinds: &[RowKind],
    ) -> Result<RowValues, PrefetchDatabaseError> {
        let user = refs
            .user_id
            .as_deref()
            .filter(|_| kinds.contains(&RowKind::User));
        let team = refs.team_id.as_deref().filter(|_| {
            kinds
                .iter()
                .any(|kind| matches!(kind, RowKind::Team | RowKind::Membership))
        });
        let membership_user = refs
            .membership_user_id
            .as_deref()
            .filter(|_| kinds.contains(&RowKind::Membership));
        let organization = refs
            .organization_id
            .as_deref()
            .filter(|_| kinds.contains(&RowKind::Organization));
        let project = refs
            .project_id
            .as_deref()
            .filter(|_| kinds.contains(&RowKind::Project));
        let client = self
            .store
            .session()
            .await
            .map_err(PrefetchDatabaseError::Database)?;
        let row = client
            .query_opt(
                ROWS_SQL,
                &[&user, &team, &membership_user, &organization, &project],
            )
            .await
            .map_err(|source| PrefetchDatabaseError::Database(backend(source)))?;
        let Some(row) = row else {
            return Ok(RowValues::default());
        };
        let mut values = RowValues::default();
        for (index, kind) in [
            RowKind::User,
            RowKind::Team,
            RowKind::Membership,
            RowKind::Organization,
            RowKind::Project,
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(value) = decode_column(&row, index, kind)? {
                values.insert(kind, value);
            }
        }
        Ok(values)
    }
}

fn decode_column(
    row: &tokio_postgres::Row,
    index: usize,
    kind: RowKind,
) -> Result<Option<JsonData>, PrefetchDatabaseError> {
    let text: Option<String> = row
        .try_get(index)
        .map_err(|source| PrefetchDatabaseError::Database(backend(source)))?;
    text.map(|value| {
        serde_json::from_str(&value).map_err(|source| PrefetchDatabaseError::Column {
            row: kind.label(),
            source,
        })
    })
    .transpose()
}
