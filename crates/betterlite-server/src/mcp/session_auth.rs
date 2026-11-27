mod tests;

use std::fmt::{self, Debug, Formatter};
use std::sync::{Arc, Mutex, PoisonError};

use super::auth_user::{McpAuthFields, McpAuthenticatedUser};

#[derive(Clone)]
pub struct SessionAuth {
    user: Arc<Mutex<McpAuthenticatedUser>>,
}

impl SessionAuth {
    #[must_use]
    pub fn new(fields: McpAuthFields) -> Self {
        Self {
            user: Arc::new(Mutex::new(McpAuthenticatedUser::new(fields))),
        }
    }

    #[must_use]
    pub fn set_or_update(
        existing: Option<&Self>,
        fields: McpAuthFields,
        copy_existing: bool,
    ) -> Self {
        let session = if copy_existing { None } else { existing };
        let Some(stored) = session else {
            return Self::new(fields);
        };
        let mut user = stored.user.lock().unwrap_or_else(PoisonError::into_inner);
        user.user_api_key_auth = fields.user_api_key_auth;
        user.mcp_auth_header = fields.mcp_auth_header;
        user.mcp_servers = fields.mcp_servers;
        // LiteLLM's update writes {} for an absent map and never touches the protocol version
        user.mcp_server_auth_headers = fields.mcp_server_auth_headers.unwrap_or_default();
        user.oauth2_headers = fields.oauth2_headers;
        user.raw_headers = fields.raw_headers;
        user.client_ip = fields.client_ip;
        drop(user);
        stored.clone()
    }

    #[must_use]
    pub fn snapshot(&self) -> McpAuthenticatedUser {
        self.user
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Debug for SessionAuth {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SessionAuth")
            .field(&*self.user.lock().unwrap_or_else(PoisonError::into_inner))
            .finish()
    }
}
