pub const COMBINED_KEY_SQL: &str = r#"
SELECT row_to_json(view_row)::text
FROM (
    SELECT
        v.*,
        t.spend AS team_spend,
        t.max_budget AS team_max_budget,
        t.soft_budget AS team_soft_budget,
        t.model_max_budget AS team_model_max_budget,
        t.tpm_limit AS team_tpm_limit,
        t.rpm_limit AS team_rpm_limit,
        t.tpd_limit AS team_tpd_limit,
        t.models AS team_models,
        t.metadata AS team_metadata,
        t.blocked AS team_blocked,
        t.team_alias AS team_alias,
        t.members_with_roles AS team_members_with_roles,
        t.object_permission_id AS team_object_permission_id,
        t.organization_id AS org_id,
        p.project_alias AS project_alias,
        tm.spend AS team_member_spend,
        b_tm.tpm_limit AS team_member_tpm_limit,
        b_tm.rpm_limit AS team_member_rpm_limit,
        m.aliases AS team_model_aliases,
        b.max_budget AS litellm_budget_table_max_budget,
        b.tpm_limit AS litellm_budget_table_tpm_limit,
        b.rpm_limit AS litellm_budget_table_rpm_limit,
        b.tpd_limit AS litellm_budget_table_tpd_limit,
        b.model_max_budget AS litellm_budget_table_model_max_budget,
        b.soft_budget AS litellm_budget_table_soft_budget,
        o.metadata AS organization_metadata,
        o.organization_alias AS organization_alias,
        b2.max_budget AS organization_max_budget,
        b2.tpm_limit AS organization_tpm_limit,
        b2.rpm_limit AS organization_rpm_limit
    FROM "LiteLLM_VerificationToken" AS v
    LEFT JOIN "LiteLLM_TeamTable" AS t ON v.team_id = t.team_id
    LEFT JOIN "LiteLLM_TeamMembership" AS tm ON v.team_id = tm.team_id AND tm.user_id = v.user_id
    LEFT JOIN "LiteLLM_BudgetTable" AS b_tm ON tm.budget_id = b_tm.budget_id
    LEFT JOIN "LiteLLM_ModelTable" AS m ON t.model_id = m.id
    LEFT JOIN "LiteLLM_BudgetTable" AS b ON v.budget_id = b.budget_id
    LEFT JOIN "LiteLLM_ProjectTable" AS p ON v.project_id = p.project_id
    LEFT JOIN "LiteLLM_OrganizationTable" AS o ON v.organization_id = o.organization_id
    LEFT JOIN "LiteLLM_BudgetTable" AS b2 ON o.budget_id = b2.budget_id
    WHERE v.token = $1
) AS view_row
"#;

pub const DEPRECATED_KEY_SQL: &str = r#"
SELECT active_token_id, revoke_at
FROM "LiteLLM_DeprecatedVerificationToken"
WHERE token = $1 AND revoke_at > $2
LIMIT 1
"#;
