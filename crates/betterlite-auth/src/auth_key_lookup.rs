pub mod cache;
pub mod error;
pub mod jwt;
pub mod key_object;
pub mod permission;
pub mod reader;
pub mod session;

mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::key_user_writes::hash::hash_token_if_needed;

use self::cache::{KeyCache, KeyCacheWriter};
use self::error::KeyLookupError;
use self::key_object::{KeyRequest, get_key_object};
use self::permission::KeyPermissionLookup;
use self::reader::KeyReader;
use crate::auth_identity::roles::{Role, map_role};
use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::user_roles::UserRole;

// What a resolved key names, read from the typed auth without exposing its fields to the server.
#[derive(Clone, Debug, Default)]
pub struct KeyIdentity {
    pub key_name: Option<String>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub user_role: Option<UserRole>,
    pub is_session_token: bool,
}

#[must_use]
pub fn key_identity(auth: &UserAPIKeyAuth) -> KeyIdentity {
    KeyIdentity {
        key_name: auth.view.token.key_name.clone(),
        user_id: auth.view.token.user_id.clone(),
        team_id: auth.view.token.team_id.clone(),
        team_alias: auth.view.team_alias.clone(),
        user_role: auth.extra.user_role,
        is_session_token: auth.extra.is_session_token,
    }
}

#[must_use]
pub fn key_roles(role: Option<UserRole>) -> Vec<Role> {
    match role {
        Some(role) => match map_role(Some(role_str(role))) {
            Some(mapped) => vec![mapped],
            None => Vec::new(),
        },
        None => Vec::new(),
    }
}

const fn role_str(role: UserRole) -> &'static str {
    match role {
        UserRole::ProxyAdmin => "proxy_admin",
        UserRole::ProxyAdminViewer => "proxy_admin_viewer",
        UserRole::OrgAdmin => "org_admin",
        UserRole::InternalUser => "internal_user",
        UserRole::InternalUserViewer => "internal_user_viewer",
        UserRole::Team => "team",
        UserRole::Customer => "customer",
    }
}

// get_key_object's collaborators, held by the state owner that composes them.
pub struct KeyLookups {
    pub reader: Option<Arc<KeyReader>>,
    pub cache: Arc<dyn KeyCache>,
    pub writer: Arc<dyn KeyCacheWriter>,
    pub permissions: Option<Arc<dyn KeyPermissionLookup>>,
}

// The server-facing entry point: hash the presented key once, then read it under the request's span.
#[async_trait]
pub trait KeyAuthenticator: Send + Sync {
    async fn authenticate(&self, presented: &str) -> Result<KeyLookupOutcome, KeyLookupError>;
}

pub struct KeyLookupOutcome {
    pub auth: UserAPIKeyAuth,
    pub hashed_token: String,
}

pub struct HashedKeyAuthenticator {
    pub lookups: Arc<KeyLookups>,
}

#[async_trait]
impl KeyAuthenticator for HashedKeyAuthenticator {
    async fn authenticate(&self, presented: &str) -> Result<KeyLookupOutcome, KeyLookupError> {
        let hashed_token = hash_token_if_needed(presented);
        let request = KeyRequest {
            hashed_token: &hashed_token,
            parent_otel_span: None,
            check_cache_only: None,
            check_db_only: false,
        };
        let auth = get_key_object(&self.lookups, &request).await?;
        Ok(KeyLookupOutcome { auth, hashed_token })
    }
}
