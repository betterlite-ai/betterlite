use std::fmt;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_auth::jwt_keys::errors::exception_for;
use betterlite_auth::jwt_keys::resolver::JwksResolver;
use betterlite_auth::jwt_keys::resolver::keys::token_kid;
use secrecy::{ExposeSecret as _, SecretString};

use super::auth_identity::{AuthContext, StageFailure, StageInput};
use super::extract_key;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// The request facts the key fetch needs, plus the resolver the state built or a test injected.
pub struct JwtKeyStageInput {
    pub resolver: Option<Arc<JwksResolver>>,
    pub token: Option<SecretString>,
}

impl fmt::Debug for JwtKeyStageInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JwtKeyStageInput")
            .field("resolver_present", &self.resolver.is_some())
            .field("token_present", &self.token.is_some())
            .finish()
    }
}

pub(crate) fn stage_input(request: &Request<Body>) -> StageInput {
    let resolver = request
        .extensions()
        .get::<Arc<JwksResolver>>()
        .map(Arc::clone)
        .or_else(|| {
            request
                .extensions()
                .get::<Arc<ServerState>>()
                .and_then(|state| state.jwt_keys.resolver.clone())
        });
    StageInput::JwtKeys(Box::new(JwtKeyStageInput {
        resolver,
        token: extract_key(request.headers()),
    }))
}

// get_public_key runs for a JWT-shaped credential only when a key URL is configured, exactly where LiteLLM's
// JWT branch sits: before the master key comparison, so an unavailable or unknown key answers as its own 503
// or 401 instead of a bad-key rejection.
pub(crate) async fn prefetched(stage: JwtKeyStageInput) -> Result<StageInput, StageFailure> {
    let resolver = stage.resolver.clone();
    let token = stage
        .token
        .as_ref()
        .map(|token| token.expose_secret().to_owned());
    let Some(resolver) = resolver else {
        return Ok(StageInput::JwtKeys(Box::new(stage)));
    };
    if resolver.settings.public_key_url.is_none() {
        return Ok(StageInput::JwtKeys(Box::new(stage)));
    }
    let Some(token) = token else {
        return Ok(StageInput::JwtKeys(Box::new(stage)));
    };
    if token.split('.').count() != 3 {
        return Ok(StageInput::JwtKeys(Box::new(stage)));
    }
    let kid = token_kid(&token);
    match resolver.get_public_key(kid.as_deref()).await {
        Ok(_) => Ok(StageInput::JwtKeys(Box::new(stage))),
        Err(error) => Err(StageFailure::JwtKeys(Box::new(exception_for(&error)))),
    }
}

pub(crate) fn run_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::JwtKeys(_) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    // The key fetch and its rejection already happened. B053 builds the JWT identity, so this stage carries
    // a placeholder that the master-key stage replaces whenever it authenticates; until then a JWT-shaped
    // credential whose key fetched still has to match the master key.
    Ok(AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: String::new(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: Vec::new(),
            scopes: Vec::new(),
            auth_method: AuthMethod::BearerJwt,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    })
}
