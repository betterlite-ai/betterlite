use std::fmt;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::other_message_exception;
use betterlite_auth::auth_identity::records::{CredentialRef, Principal};
use betterlite_auth::auth_identity::roles::AuthMethod;
use betterlite_auth::auth_key_lookup::{KeyAuthenticator, KeyLookupOutcome};
use betterlite_auth::identity_store::principal::{Projection, principal_from_key};
use betterlite_auth::master_key_admin::{MASTER_KEY_ALIAS, master_key_admin_auth};
use betterlite_db::model_secrets::DEFAULT_USER_ID;
use betterlite_models::key_digest::hashing::hash_token;
use secrecy::{ExposeSecret as _, SecretString};

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

pub(crate) mod key_headers;
pub(crate) mod tests;

// authenticate copies it from the middleware state, so a route merged after the state layer still compares it
#[derive(Clone)]
pub(crate) struct ConfiguredMasterKey(pub(crate) SecretString);

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

// authenticate installs the configured key on the request the stage loop runs for, whatever router owns the route.
fn master_key_for(request: &Request<Body>) -> Option<SecretString> {
    request
        .extensions()
        .get::<ConfiguredMasterKey>()
        .map(|key| key.0.clone())
}

// The master key wins as it does in LiteLLM, then the virtual key is read through get_key_object.
pub(crate) async fn prefetch(stage: KeyLookupStageInput) -> Result<StageInput, StageFailure> {
    let master = match stage.master_key.as_ref() {
        Some(key) => master_key_context(stage.presented.as_ref(), key)?,
        None => None,
    };
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
        Ok(outcome) => Ok(StageInput::Authenticated(Box::new(key_context(&outcome)))),
        Err(error) => Err(StageFailure::KeyLookup(Box::new(
            as_proxy_exception(&error.failure()).exception().clone(),
        ))),
    }
}

// The fallback path projects the key exactly as IdentityStore::resolve_as does, so both paths agree.
fn key_context(outcome: &KeyLookupOutcome) -> AuthContext {
    let projection = Projection {
        auth_method: AuthMethod::ApiKey,
        issuer: None,
        subject_fallback: Some(&outcome.hashed_token),
        scopes: &[],
        credential_ref: Some(CredentialRef {
            key_id: None,
            token_id: Some(outcome.hashed_token.clone()),
        }),
        network: None,
    };
    AuthContext {
        principal: principal_from_key(&outcome.auth, projection),
    }
}

// The master key authenticates as LiteLLM's master key record, projected the way _resolve_request_principal projects any key.
pub(crate) fn master_key_context(
    presented: Option<&SecretString>,
    master_key: &SecretString,
) -> Result<Option<AuthContext>, StageFailure> {
    let Some(key) = presented else {
        return Ok(None);
    };
    let digest = hash_token(key.expose_secret());
    // The record is built from fixed fields, so a construct error is answered the way LiteLLM's handler answers an exception in this branch.
    let Some(record) =
        master_key_admin_auth(&digest, Some(master_key), DEFAULT_USER_ID).map_err(|error| {
            StageFailure::KeyLookup(Box::new(other_message_exception(&error.to_string())))
        })?
    else {
        return Ok(None);
    };
    let projection = Projection {
        auth_method: AuthMethod::ApiKey,
        issuer: None,
        subject_fallback: Some(MASTER_KEY_ALIAS),
        scopes: &[],
        credential_ref: Some(CredentialRef {
            key_id: None,
            token_id: Some(MASTER_KEY_ALIAS.to_owned()),
        }),
        network: None,
    };
    let principal = Principal {
        subject: "master_key".to_owned(),
        ..principal_from_key(&record, projection)
    };
    Ok(Some(AuthContext { principal }))
}
