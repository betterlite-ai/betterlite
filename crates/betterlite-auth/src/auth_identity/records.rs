use std::fmt;

use serde::{Deserialize, Serialize};

use crate::auth_identity::roles::{AuthMethod, PrincipalType, Role, TeamRole};
use crate::resolved_key::auth::UserAPIKeyAuth;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UserIdentity {
    pub id: String,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub user_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrganizationIdentity {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectIdentity {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TeamIdentity {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub role: TeamRole,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EndUserIdentity {
    pub id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CredentialRef {
    #[serde(default)]
    pub key_id: Option<String>,
    #[serde(default)]
    pub token_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct NetworkContext {
    #[serde(default)]
    pub client_ip: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub via_trusted_proxy: bool,
}

#[derive(Clone, Default)]
pub struct SourceKey(pub Option<UserAPIKeyAuth>);

impl fmt::Debug for SourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SourceKey(..)")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Principal {
    pub principal_type: PrincipalType,
    pub subject: String,
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub audience: Vec<String>,
    #[serde(default)]
    pub user: Option<UserIdentity>,
    #[serde(default)]
    pub organization: Option<OrganizationIdentity>,
    #[serde(default)]
    pub teams: Vec<TeamIdentity>,
    #[serde(default)]
    pub project: Option<ProjectIdentity>,
    #[serde(default)]
    pub end_user: Option<EndUserIdentity>,
    #[serde(default)]
    pub roles: Vec<Role>,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub credential_ref: CredentialRef,
    #[serde(default)]
    pub network: NetworkContext,
    #[serde(skip)]
    pub source_key: SourceKey,
}

impl Principal {
    #[must_use]
    pub fn new(principal_type: PrincipalType, subject: String, auth_method: AuthMethod) -> Self {
        Self {
            principal_type,
            subject,
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: Vec::new(),
            scopes: Vec::new(),
            auth_method,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        }
    }
}

mod tests;
