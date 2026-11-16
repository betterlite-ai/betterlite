pub(super) const RETIRED_AGENT_SQL: &str = r#"SELECT original_agent_id
FROM "LiteLLM_RetiredAgent"
WHERE original_agent_id = $1"#;

// Prisma's find_unique with the identity and object_permission includes, decoded as AgentResponse
pub(super) const AGENT_SQL: &str = r#"SELECT jsonb_build_object(
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
pub(super) const IDENTITY_SQL: &str = r#"SELECT agent_id
FROM "LiteLLM_AgentIdentity"
WHERE provider = $1 AND tenant_id = $2 AND client_id = $3"#;

// update_many's where clause is the final binding check: revision, active and the nested agent filter
pub(super) const AUTHENTICATION_UPDATE_SQL: &str = r#"UPDATE "LiteLLM_AgentIdentity" AS i
SET last_authenticated_at = $6
FROM "LiteLLM_AgentsTable" AS a
WHERE i.agent_id = a.agent_id
  AND i.agent_id = $1
  AND i.revision = $2
  AND i.active = $3
  AND a.enabled = $4
  AND a.identity_managed = $5"#;

// the verified-subject table's issuer_tenant_id_oid unique key
pub(super) const VERIFIED_SUBJECT_SQL: &str = r#"SELECT to_jsonb(s.*)::text
FROM "LiteLLM_VerifiedSubject" s
WHERE s.issuer = $1 AND s.tenant_id = $2 AND s.oid = $3"#;

// Prisma's upsert with empty update data still returns the stored row, leaving it unchanged
pub(super) const ENROLLMENT_UPSERT_SQL: &str = r#"INSERT INTO "LiteLLM_VerifiedSubject" (subject_id, issuer, tenant_id, oid, user_id, verified_via)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (issuer, tenant_id, oid) DO UPDATE SET user_id = "LiteLLM_VerifiedSubject".user_id
RETURNING to_jsonb("LiteLLM_VerifiedSubject".*)::text"#;

// the retired table's provider_tenant_id_client_id unique key
pub(super) const RETIRED_IDENTITY_SQL: &str = r#"SELECT binding_id
FROM "LiteLLM_RetiredAgentIdentity"
WHERE provider = $1 AND tenant_id = $2 AND client_id = $3"#;
