#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;

const PERMISSION: &str = r#"{"object_permission_id": "$id", "mcp_servers": ["mcp-1"], "mcp_access_groups": [], "mcp_tool_permissions": {"mcp-1": ["tool-1"]}, "vector_stores": [], "agents": [], "agent_access_groups": [], "models": ["model-a", "model-a"], "mcp_toolsets": [], "blocked_tools": [], "search_tools": [], "skills": []}"#;

const USER: &str = r#"{"user_id": "$id", "user_email": "user@example.com", "user_role": "internal_user", "teams": ["team-b", "team-a", "team-b"], "models": ["model-a"], "allowed_cache_controls": ["no-cache"], "policies": [], "spend": 0.0, "organization_memberships": []}"#;

const TEAM: &str = r#"{"team_id": "$id", "team_alias": "team alias", "admins": ["admin-1"], "members": ["member-b", "member-a", "member-b"], "members_with_roles": [], "models": ["model-a"], "team_member_permissions": [], "access_group_ids": [], "policies": [], "default_team_member_models": [], "blocked": false, "model_id": 7, "litellm_model_table": {"id": 7, "model_aliases": {"gpt": "gpt-4"}, "created_by": "tester", "updated_by": "tester"}, "object_permission_id": "perm-team", "object_permission": {"object_permission_id": "perm-team", "mcp_servers": ["mcp-1"], "mcp_access_groups": [], "mcp_tool_permissions": {"mcp-1": ["tool-1"]}, "vector_stores": [], "agents": [], "agent_access_groups": [], "models": ["model-a", "model-a"], "mcp_toolsets": [], "blocked_tools": [], "search_tools": [], "skills": []}}"#;

const MEMBERSHIP: &str = r#"{"user_id": "$user", "team_id": "$team", "budget_id": "budget-member", "spend": 0.0, "total_spend": 0.0, "litellm_budget_table": {"budget_id": "budget-member", "max_budget": 10.0, "allowed_models": ["model-a", "model-a"]}}"#;

const ORGANIZATION: &str = r#"{"organization_id": "$id", "organization_alias": "org alias", "budget_id": "budget-org", "models": ["model-a"], "spend": 0.0, "created_by": "tester", "updated_by": "tester", "litellm_budget_table": {"budget_id": "budget-org", "max_budget": 10.0, "allowed_models": ["model-a", "model-a"]}, "object_permission_id": "perm-org", "object_permission": {"object_permission_id": "perm-org", "mcp_servers": ["mcp-1"], "mcp_access_groups": [], "mcp_tool_permissions": {"mcp-1": ["tool-1"]}, "vector_stores": [], "agents": [], "agent_access_groups": [], "models": ["model-a", "model-a"], "mcp_toolsets": [], "blocked_tools": [], "search_tools": [], "skills": []}}"#;

const PROJECT: &str = r#"{"project_id": "$id", "project_alias": "project alias", "team_id": "team-1", "budget_id": "budget-project", "models": ["model-a"], "spend": 0.0, "blocked": false, "litellm_budget_table": {"budget_id": "budget-project", "max_budget": 10.0, "allowed_models": ["model-a", "model-a"]}, "object_permission_id": "perm-project", "object_permission": {"object_permission_id": "perm-project", "mcp_servers": ["mcp-1"], "mcp_access_groups": [], "mcp_tool_permissions": {"mcp-1": ["tool-1"]}, "vector_stores": [], "agents": [], "agent_access_groups": [], "models": ["model-a", "model-a"], "mcp_toolsets": [], "blocked_tools": [], "search_tools": [], "skills": []}}"#;

// the row the pinned SELECT returns for a project: every JSONB column defaults to {}, nulls stay null
const STORED_PROJECT: &str = r#"{"project_id": "$id", "project_alias": null, "description": null, "team_id": "team-1", "budget_id": "budget-project", "metadata": {}, "models": ["model-a"], "spend": 0, "model_spend": {}, "model_rpm_limit": {}, "model_tpm_limit": {}, "blocked": false, "object_permission_id": "perm-project", "created_by": "tester", "updated_by": "tester", "created_at": "2026-11-19T12:00:00+00:00", "updated_at": "2026-11-19T12:00:00+00:00", "litellm_budget_table": {"budget_id": "budget-project", "soft_budget": null, "max_budget": null, "allowed_models": [], "created_at": "2026-11-19T12:00:00+00:00", "updated_at": "2026-11-19T12:00:00+00:00"}, "object_permission": {"object_permission_id": "perm-project", "mcp_servers": [], "mcp_access_groups": [], "mcp_tool_permissions": null, "vector_stores": [], "agents": [], "agent_access_groups": [], "models": [], "mcp_toolsets": [], "blocked_tools": [], "search_tools": [], "skills": []}}"#;

fn decode(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the fixture is valid json")
}

fn filled(template: &str, id: &str) -> JsonData {
    decode(&template.replace("$id", id))
}

pub(crate) fn permission_row(permission_id: &str) -> JsonData {
    filled(PERMISSION, permission_id)
}

pub(crate) fn user_row(user_id: &str) -> JsonData {
    filled(USER, user_id)
}

pub(crate) fn team_row(team_id: &str) -> JsonData {
    filled(TEAM, team_id)
}

pub(crate) fn membership_row(user_id: &str, team_id: &str) -> JsonData {
    let text = MEMBERSHIP
        .replace("$user", user_id)
        .replace("$team", team_id);
    decode(&text)
}

pub(crate) fn organization_row(organization_id: &str) -> JsonData {
    filled(ORGANIZATION, organization_id)
}

pub(crate) fn project_row(project_id: &str) -> JsonData {
    filled(PROJECT, project_id)
}

pub(crate) fn stored_project_row(project_id: &str) -> JsonData {
    filled(STORED_PROJECT, project_id)
}
