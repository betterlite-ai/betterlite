#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;

pub(crate) const USER_ID: &str = "user-1";
pub(crate) const TEAM_ID: &str = "team-1";
pub(crate) const ORGANIZATION_ID: &str = "org-1";
pub(crate) const PROJECT_ID: &str = "project-1";
pub(crate) const TEAM_PERMISSION_ID: &str = "perm-team";
pub(crate) const ORGANIZATION_PERMISSION_ID: &str = "perm-org";
pub(crate) const PROJECT_PERMISSION_ID: &str = "perm-project";

fn decode(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the fixture is valid json")
}

// one row per kind, shaped as the database returns it
pub(crate) fn permission_row(permission_id: &str) -> JsonData {
    decode(&format!(
        r#"{{
            "object_permission_id": "{permission_id}",
            "mcp_servers": ["mcp-1"],
            "mcp_access_groups": [],
            "mcp_tool_permissions": {{"mcp-1": ["tool-1"]}},
            "vector_stores": [],
            "agents": [],
            "agent_access_groups": [],
            "models": ["model-a", "model-a"],
            "mcp_toolsets": [],
            "blocked_tools": [],
            "search_tools": [],
            "skills": []
        }}"#
    ))
}

pub(crate) fn budget_row(budget_id: &str) -> JsonData {
    decode(&format!(
        r#"{{
            "budget_id": "{budget_id}",
            "max_budget": 10.0,
            "allowed_models": ["model-a", "model-a"]
        }}"#
    ))
}

pub(crate) fn user_row(user_id: &str) -> JsonData {
    decode(&format!(
        r#"{{
            "user_id": "{user_id}",
            "user_email": "user@example.com",
            "user_role": "internal_user",
            "teams": ["team-b", "team-a", "team-b"],
            "models": ["model-a"],
            "allowed_cache_controls": ["no-cache"],
            "policies": [],
            "spend": 0.0,
            "organization_memberships": []
        }}"#
    ))
}

pub(crate) fn team_row(team_id: &str) -> JsonData {
    decode(&format!(
        r#"{{
            "team_id": "{team_id}",
            "team_alias": "team alias",
            "admins": ["admin-1"],
            "members": ["member-b", "member-a", "member-b"],
            "members_with_roles": [],
            "models": ["model-a"],
            "team_member_permissions": [],
            "access_group_ids": [],
            "policies": [],
            "default_team_member_models": [],
            "blocked": false,
            "model_id": 7,
            "litellm_model_table": {{
                "id": 7,
                "model_aliases": {{"gpt": "gpt-4"}},
                "created_by": "tester",
                "updated_by": "tester"
            }},
            "object_permission_id": "{TEAM_PERMISSION_ID}",
            "object_permission": {}
        }}"#,
        permission_text(TEAM_PERMISSION_ID)
    ))
}

pub(crate) fn membership_row(user_id: &str, team_id: &str) -> JsonData {
    decode(&format!(
        r#"{{
            "user_id": "{user_id}",
            "team_id": "{team_id}",
            "budget_id": "budget-member",
            "spend": 0.0,
            "total_spend": 0.0,
            "litellm_budget_table": {budget_text}
        }}"#,
        budget_text = budget_text("budget-member")
    ))
}

pub(crate) fn organization_row(organization_id: &str) -> JsonData {
    decode(&format!(
        r#"{{
            "organization_id": "{organization_id}",
            "organization_alias": "org alias",
            "budget_id": "budget-org",
            "models": ["model-a"],
            "spend": 0.0,
            "created_by": "tester",
            "updated_by": "tester",
            "litellm_budget_table": {budget_text},
            "object_permission_id": "{ORGANIZATION_PERMISSION_ID}",
            "object_permission": {permission_text}
        }}"#,
        budget_text = budget_text("budget-org"),
        permission_text = permission_text(ORGANIZATION_PERMISSION_ID)
    ))
}

pub(crate) fn project_row(project_id: &str) -> JsonData {
    decode(&format!(
        r#"{{
            "project_id": "{project_id}",
            "project_alias": "project alias",
            "team_id": "{TEAM_ID}",
            "budget_id": "budget-project",
            "models": ["model-a"],
            "spend": 0.0,
            "blocked": false,
            "litellm_budget_table": {budget_text},
            "object_permission_id": "{PROJECT_PERMISSION_ID}",
            "object_permission": {permission_text}
        }}"#,
        budget_text = budget_text("budget-project"),
        permission_text = permission_text(PROJECT_PERMISSION_ID)
    ))
}

fn budget_text(budget_id: &str) -> String {
    format!(
        r#"{{"budget_id": "{budget_id}", "max_budget": 10.0, "allowed_models": ["model-a", "model-a"]}}"#
    )
}

fn permission_text(permission_id: &str) -> String {
    format!(
        r#"{{"object_permission_id": "{permission_id}", "mcp_servers": ["mcp-1"], "mcp_access_groups": [], "mcp_tool_permissions": {{"mcp-1": ["tool-1"]}}, "vector_stores": [], "agents": [], "agent_access_groups": [], "models": ["model-a", "model-a"], "mcp_toolsets": [], "blocked_tools": [], "search_tools": [], "skills": []}}"#
    )
}
