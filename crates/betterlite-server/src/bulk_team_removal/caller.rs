use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, is_team_service_account};
use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::settings::JsonValue;
use secrecy::SecretString;
use serde::Deserialize;

use super::error::BulkRemovalError;
use crate::audit_events::identity::CallerIdentity;
use crate::key_events::types::KeyEventCaller;
use crate::team_access::caller::TeamCaller;

// the UserAPIKeyAuth fields the removal reads, taken once from the key's serde form
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RemovalCaller {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_role: Option<UserRole>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub token: Option<SecretString>,
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub metadata: Option<JsonValue>,
    #[serde(default)]
    pub team_metadata: Option<JsonValue>,
    #[serde(skip)]
    pub team_service_account: bool,
}

impl RemovalCaller {
    pub fn from_auth(auth: &UserAPIKeyAuth) -> Result<Self, BulkRemovalError> {
        let mut caller: Self = serde_json::to_value(auth)
            .and_then(serde_json::from_value)
            .map_err(BulkRemovalError::Encode)?;
        caller.team_service_account = is_team_service_account(auth);
        Ok(caller)
    }

    #[must_use]
    pub fn team_caller(&self) -> TeamCaller {
        TeamCaller {
            user_id: self.user_id.clone(),
            user_role: self.user_role,
            team_id: self.team_id.clone(),
            team_service_account: self.team_service_account,
        }
    }

    #[must_use]
    pub fn identity(&self) -> CallerIdentity {
        CallerIdentity {
            user_id: self.user_id.clone(),
            api_key: self.api_key.clone(),
            metadata: self.metadata.clone(),
            team_metadata: self.team_metadata.clone(),
        }
    }

    #[must_use]
    pub fn key_event_caller(&self) -> KeyEventCaller {
        KeyEventCaller {
            identity: self.identity(),
            token: self.token.clone(),
        }
    }
}
