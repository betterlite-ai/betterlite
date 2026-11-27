use std::fmt;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_auth::jwt_keys::errors::{KeyError, exception_for};
use betterlite_auth::jwt_verification::TypedValue;
use betterlite_auth::jwt_verification::errors::into_proxy_exception;
use betterlite_auth::jwt_verification::source::PublicKeySource;
use betterlite_auth::jwt_verification::verifier::JwtVerifier;
use betterlite_auth::resolved_key::credential::is_jwt;
use secrecy::{ExposeSecret as _, SecretString};

use super::auth_identity::{AuthContext, StageFailure, StageInput};
use super::extract_key;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// The credential, the verifier and the key source the state built or a test injected, plus what verification produced.
#[derive(Clone)]
pub struct JwtVerificationStageInput {
    pub verifier: Option<Arc<JwtVerifier>>,
    pub source: Option<Arc<dyn PublicKeySource>>,
    pub token: Option<SecretString>,
    verified: Option<TypedValue>,
}

impl fmt::Debug for JwtVerificationStageInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JwtVerificationStageInput")
            .field("verifier_present", &self.verifier.is_some())
            .field("source_present", &self.source.is_some())
            .field("token_present", &self.token.is_some())
            .field("verified", &self.verified.is_some())
            .finish()
    }
}

pub(crate) fn stage_input(request: &Request<Body>) -> StageInput {
    let extensions = request.extensions();
    let state = extensions.get::<Arc<ServerState>>();
    let injected_verifier = extensions.get::<Arc<JwtVerifier>>();
    let verifier = match injected_verifier {
        Some(injected) => Some(Arc::clone(injected)),
        None => state.and_then(|server| server.jwt_verification.verifier.clone()),
    };
    let injected_source = extensions.get::<Arc<dyn PublicKeySource>>();
    let source: Option<Arc<dyn PublicKeySource>> = match injected_source {
        Some(injected) => Some(Arc::clone(injected)),
        None => match state.and_then(|server| server.jwt_keys.resolver.as_ref()) {
            Some(resolver) => {
                let source: Arc<dyn PublicKeySource> = Arc::clone(resolver);
                Some(source)
            }
            None => None,
        },
    };
    StageInput::JwtVerification(Box::new(JwtVerificationStageInput {
        verifier,
        source,
        token: extract_key(request.headers()),
        verified: None,
    }))
}

// auth_jwt: the signature and claims are verified before any identity lookup, and a failure keeps its HTTP mapping.
pub(crate) async fn prefetched(
    mut stage: JwtVerificationStageInput,
) -> Result<StageInput, StageFailure> {
    let Some(verifier) = stage.verifier.as_ref() else {
        return Ok(StageInput::JwtVerification(Box::new(stage)));
    };
    let Some(credential) = stage.token.as_ref() else {
        return Ok(StageInput::JwtVerification(Box::new(stage)));
    };
    let token = credential.expose_secret();
    if !is_jwt(Some(token)) {
        return Ok(StageInput::JwtVerification(Box::new(stage)));
    }
    let Some(source) = stage.source.as_ref() else {
        return Err(StageFailure::JwtVerification(Box::new(exception_for(
            &KeyError::Unreachable {
                message: "no public key source is configured".to_owned(),
            },
        ))));
    };
    match verifier.auth_jwt(token, source.as_ref()).await {
        Ok(verified) => {
            stage.verified = Some(verified);
            Ok(StageInput::JwtVerification(Box::new(stage)))
        }
        Err(error) => Err(StageFailure::JwtVerification(Box::new(
            into_proxy_exception(&error),
        ))),
    }
}

pub(crate) fn run_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::JwtVerification(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    // B053 builds the JWT identity from these claims; until then the stage owns the check and its failure.
    let (issuer, audience) = stage
        .verified
        .as_ref()
        .map_or_else(|| (None, Vec::new()), claims_scope);
    Ok(AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: String::new(),
            issuer,
            audience,
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

// The verified scope a caller can see: a string iss and a string or list aud, with no other claim read here.
fn claims_scope(payload: &TypedValue) -> (Option<String>, Vec<String>) {
    let issuer = payload
        .json
        .get("iss")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    let audience = payload
        .json
        .get("aud")
        .map_or_else(Vec::new, |value| match value.as_array() {
            Some(items) => items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::to_owned)
                .collect(),
            None => value
                .as_str()
                .map(str::to_owned)
                .map_or_else(Vec::new, |text| vec![text]),
        });
    (issuer, audience)
}
