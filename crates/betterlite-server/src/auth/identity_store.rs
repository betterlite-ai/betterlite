use std::fmt;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::as_proxy_exception;
use betterlite_auth::identity_store::IdentityStore;
use betterlite_db::key_user_writes::hash::hash_token_if_needed;
use secrecy::{ExposeSecret as _, SecretString};

use super::auth_identity::{AuthContext, StageFailure, StageInput, master_key_context};
use super::auth_key_lookup;
use super::auth_key_lookup::ConfiguredMasterKey;
use super::auth_key_lookup::key_headers::extract_key;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// The resolved identity's inputs: the credential, the composition root's store, and the master key compared first.
pub struct IdentityStageInput {
    presented: Option<SecretString>,
    store: IdentityStore,
    master_key: Option<SecretString>,
}

impl fmt::Debug for IdentityStageInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentityStageInput")
            .field("presented", &self.presented.is_some())
            .field("store", &self.store)
            .field("master_key", &self.master_key.is_some())
            .finish()
    }
}

// A state whose composition root installed the store's ports authenticates through it; otherwise B738's lookup stays.
pub(crate) fn lookup_stage_input(request: &Request<Body>) -> StageInput {
    let store = request
        .extensions()
        .get::<Arc<ServerState>>()
        .and_then(|state| state.identity_store.store());
    let Some(store) = store else {
        return auth_key_lookup::lookup_stage_input(request);
    };
    StageInput::AuthIdentity(Box::new(IdentityStageInput {
        presented: extract_key(request.headers()),
        store,
        master_key: master_key_for(request),
    }))
}

fn master_key_for(request: &Request<Body>) -> Option<SecretString> {
    request
        .extensions()
        .get::<ConfiguredMasterKey>()
        .map(|key| key.0.clone())
}

// The master key wins as it does in LiteLLM; otherwise the presented key is hashed once and resolved by the store.
pub(crate) async fn prefetch(stage: IdentityStageInput) -> Result<StageInput, StageFailure> {
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
    let hashed_token = hash_token_if_needed(credential.expose_secret());
    match stage.store.resolve(&hashed_token).await {
        Ok(principal) => Ok(StageInput::Authenticated(Box::new(AuthContext {
            principal,
        }))),
        Err(error) => Err(StageFailure::KeyLookup(Box::new(
            as_proxy_exception(&error.failure()).exception().clone(),
        ))),
    }
}
