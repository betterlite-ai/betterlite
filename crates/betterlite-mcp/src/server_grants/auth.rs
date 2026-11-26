use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use serde::Deserialize;
use thiserror::Error;

use crate::auth::object_permission_hydration::record::{AuthDecodeError, McpAuth};

// B4668's MCP view of the caller plus the key's unified access_group_ids, which only the key path reads
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GrantAuth {
    pub mcp: McpAuth,
    pub access_group_ids: Vec<String>,
}

#[derive(Debug, Error)]
pub enum GrantAuthError {
    #[error(transparent)]
    Mcp(#[from] AuthDecodeError),
    #[error("the key's access_group_ids could not be decoded")]
    AccessGroups(#[source] serde_json::Error),
}

#[derive(Deserialize)]
struct KeyAccessGroups {
    #[serde(default)]
    access_group_ids: Option<Vec<String>>,
}

impl GrantAuth {
    pub fn from_auth(auth: &UserAPIKeyAuth) -> Result<Self, GrantAuthError> {
        let mcp = McpAuth::from_auth(auth)?;
        // UserAPIKeyAuth is Serialize-only, so another crate reads the key's groups through its wire form
        let groups = serde_json::to_string(auth)
            .and_then(|text| serde_json::from_str::<KeyAccessGroups>(&text))
            .map_err(GrantAuthError::AccessGroups)?;
        Ok(Self {
            mcp,
            access_group_ids: groups.access_group_ids.unwrap_or_default(),
        })
    }
}
