use betterlite_auth::managed_admission::managed_marks;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use serde::Deserialize;
use thiserror::Error;

use super::reads::ObjectPermission;

pub const MCP_ALL_TOOLS_WILDCARD: &str = "*";
pub const AGENT_NO_PERMISSION_SENTINEL: &str = "__agent_no_mcp_permission__";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct McpAuth {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub org_id: Option<String>,
    pub end_user_id: Option<String>,
    pub agent_id: Option<String>,
    pub object_permission_id: Option<String>,
    pub object_permission: Option<ObjectPermission>,
    pub requires_fresh_policy: bool,
}

#[derive(Debug, Error)]
#[error("the user api key auth could not be decoded for MCP permission hydration")]
pub struct AuthDecodeError {
    #[source]
    source: serde_json::Error,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct AuthFields {
    user_id: Option<String>,
    team_id: Option<String>,
    org_id: Option<String>,
    end_user_id: Option<String>,
    agent_id: Option<String>,
    object_permission_id: Option<String>,
    object_permission: Option<ObjectPermission>,
}

impl McpAuth {
    pub fn from_auth(auth: &UserAPIKeyAuth) -> Result<Self, AuthDecodeError> {
        // UserAPIKeyAuth is Serialize-only, so another crate reads the view through its wire form
        let text = serde_json::to_string(auth).map_err(|source| AuthDecodeError { source })?;
        let fields = serde_json::from_str::<AuthFields>(&text)
            .map_err(|source| AuthDecodeError { source })?;
        Ok(Self {
            user_id: fields.user_id,
            team_id: fields.team_id,
            org_id: fields.org_id,
            end_user_id: fields.end_user_id,
            agent_id: fields.agent_id,
            object_permission_id: fields.object_permission_id,
            object_permission: fields.object_permission,
            requires_fresh_policy: managed_marks(auth).requires_fresh_policy,
        })
    }
}
