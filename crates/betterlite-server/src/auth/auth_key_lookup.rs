use std::fmt;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::as_proxy_exception;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, TeamIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, TeamRole};
use betterlite_auth::auth_key_lookup::{
    KeyAuthenticator, KeyLookupOutcome, key_identity, key_roles,
};
use secrecy::{ExposeSecret as _, SecretString};

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput, master_key_context};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

pub(crate) mod key_headers;
mod tests;

// The presented credential and the collaborators the lookup needs; the master key is compared in the prefetch.
#[derive(Clone)]
pub struct KeyLookupStageInput {
    pub(crate) presented: Option<SecretString>,
    pub(crate) authenticator: Option<Arc<dyn KeyAuthenticator>>,
    master_key: Option<SecretString>,
}

impl fmt::Debug for KeyLookupStageInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyLookupStageInput")
            .field("presented", &self.presented.is_some())
            .field("authenticator", &self.authenticator.is_some())
            .field("master_key", &self.master_key.is_some())
            .finish()
    }
}

pub(crate) fn lookup_stage_input(request: &Request<Body>) -> StageInput {
    StageInput::AuthKeyLookup(Box::new(KeyLookupStageInput {
        presented: super::extract_key(request.headers()),
        authenticator: authenticator_for(request),
        master_key: master_key_for(request),
    }))
}

// An earlier stage that read the key leaves the identity here, and the master key stage only forwards it.
pub(crate) fn master_stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> StageInput {
    authenticated.map_or_else(
        || StageInput::MasterKey(super::extract_key(request.headers())),
        |context| StageInput::Authenticated(Box::new(context.clone())),
    )
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

// routes hands the middleware this same key, so the prefetch compares against the configured master key.
fn master_key_for(request: &Request<Body>) -> Option<SecretString> {
    request
        .extensions()
        .get::<Arc<ServerState>>()
        .and_then(|state| state.config.general_settings.master_key.clone())
}

// The master key wins as it does in LiteLLM, then the virtual key is read through get_key_object.
pub(crate) async fn prefetch(stage: KeyLookupStageInput) -> Result<StageInput, StageFailure> {
    let master = stage
        .master_key
        .as_ref()
        .and_then(|key| master_key_context(stage.presented.as_ref(), key));
    if let Some(context) = master {
        return Ok(StageInput::Authenticated(Box::new(context)));
    }
    let presented = stage
        .presented
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty());
    let Some(credential) = presented else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    let Some(authenticator) = stage.authenticator.as_ref() else {
        return Err(StageFailure::from(AuthenticationFailure::Invalid));
    };
    match authenticator.authenticate(credential.expose_secret()).await {
        Ok(outcome) => Ok(StageInput::Authenticated(Box::new(key_context(outcome)))),
        Err(error) => Err(StageFailure::KeyLookup(Box::new(
            as_proxy_exception(&error.failure()).exception().clone(),
        ))),
    }
}

fn key_context(outcome: KeyLookupOutcome) -> AuthContext {
    let identity = key_identity(&outcome.auth);
    let subject = identity
        .user_id
        .clone()
        .or_else(|| identity.key_name.clone())
        .unwrap_or_else(|| outcome.hashed_token.clone());
    let teams = identity.team_id.clone().map_or_else(Vec::new, |id| {
        vec![TeamIdentity {
            id,
            name: identity.team_alias.clone(),
            role: TeamRole::Member,
        }]
    });
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
