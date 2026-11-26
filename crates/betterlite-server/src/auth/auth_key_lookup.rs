use std::fmt;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, TeamIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, TeamRole};
use betterlite_auth::auth_key_lookup::error::proxy_exception;
use betterlite_auth::auth_key_lookup::{
    KeyAuthenticator, KeyLookupOutcome, key_identity, key_roles,
};
use secrecy::{ExposeSecret as _, SecretString};

use crate::auth::auth_identity::{AuthContext, StageFailure, master_key_context};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// The presented credential and the collaborators the lookup needs; the master key is compared in the prefetch.
#[derive(Clone)]
pub(crate) struct KeyLookupStageInput {
    pub(crate) presented: Option<SecretString>,
    pub(crate) authenticator: Option<Arc<dyn KeyAuthenticator>>,
}

impl fmt::Debug for KeyLookupStageInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeyLookupStageInput")
            .field("presented", &self.presented.is_some())
            .field("authenticator", &self.authenticator.is_some())
            .finish()
    }
}

pub(crate) fn stage_input(request: &Request<Body>) -> KeyLookupStageInput {
    KeyLookupStageInput {
        presented: super::extract_key(request.headers()),
        authenticator: authenticator_for(request),
    }
}

// A test or a later composition installs the authenticator as an extension or in the state slot.
fn authenticator_for(request: &Request<Body>) -> Option<Arc<dyn KeyAuthenticator>> {
    request
        .extensions()
        .get::<Arc<dyn KeyAuthenticator>>()
        .map(Arc::clone)
        .or_else(|| {
            request
                .extensions()
                .get::<Arc<ServerState>>()
                .and_then(|state| state.auth_key_lookup.authenticator.clone())
        })
}

// The master key wins as it does in LiteLLM, then the virtual key is read through get_key_object.
pub(crate) async fn prefetch(
    stage: &KeyLookupStageInput,
    master_key: &SecretString,
) -> Result<AuthContext, StageFailure> {
    if let Some(context) = master_key_context(stage.presented.as_ref(), master_key) {
        return Ok(context);
    }
    let presented = stage
        .presented
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty());
    let Some(presented) = presented else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    let Some(authenticator) = stage.authenticator.as_ref() else {
        return Err(StageFailure::from(AuthenticationFailure::Invalid));
    };
    match authenticator.authenticate(presented.expose_secret()).await {
        Ok(outcome) => Ok(key_context(outcome)),
        Err(error) => Err(StageFailure::KeyLookup(Box::new(proxy_exception(&error)))),
    }
}

fn key_context(outcome: KeyLookupOutcome) -> AuthContext {
    let identity = key_identity(&outcome.auth);
    let subject = identity
        .user_id
        .clone()
        .or_else(|| identity.key_name.clone())
        .unwrap_or_else(|| outcome.hashed_token.clone());
    let teams = match identity.team_id.clone() {
        Some(id) => vec![TeamIdentity {
            id,
            name: identity.team_alias.clone(),
            role: TeamRole::Member,
        }],
        None => Vec::new(),
    };
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject,
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams,
            project: None,
            end_user: None,
            roles: key_roles(identity.user_role),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef {
                key_id: identity.key_name,
                token_id: Some(outcome.hashed_token),
            },
            network: NetworkContext::default(),
            source_key: SourceKey(Some(outcome.auth)),
        },
    }
}
