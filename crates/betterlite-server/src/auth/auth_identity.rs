use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use crate::error_body::AuthenticationFailure;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthBackend {
    MasterKey,
}

// B052 owns the final stage order; only the master-key stage exists so far.
pub(crate) const INSTALLED_STAGES: &[AuthBackend] = &[AuthBackend::MasterKey];

#[derive(Clone, Debug)]
pub enum StageInput {
    MasterKey(Option<SecretString>),
}

#[derive(Clone, Debug)]
pub struct AuthContext {
    pub principal: Principal,
}

#[derive(Clone, Debug)]
pub struct RequestScope {
    pub route: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionLease;

pub trait BudgetAdmission {
    fn admit(
        &self,
        context: &AuthContext,
        scope: &RequestScope,
    ) -> Result<AdmissionLease, AuthError>;
}

pub(crate) fn dispatch(
    backend: AuthBackend,
    input: &StageInput,
    master_key: &SecretString,
) -> Result<AuthContext, AuthenticationFailure> {
    match backend {
        AuthBackend::MasterKey => master_key_stage(input, master_key),
    }
}

fn master_key_stage(
    input: &StageInput,
    master_key: &SecretString,
) -> Result<AuthContext, AuthenticationFailure> {
    let StageInput::MasterKey(presented) = input;
    let matched = presented.as_ref().is_some_and(|key| {
        !key.expose_secret().is_empty()
            && bool::from(
                key.expose_secret()
                    .as_bytes()
                    .ct_eq(master_key.expose_secret().as_bytes()),
            )
    });
    if matched {
        return Ok(AuthContext {
            principal: Principal {
                principal_type: PrincipalType::ServiceAccount,
                subject: "master_key".to_owned(),
                issuer: None,
                audience: Vec::new(),
                user: None,
                organization: None,
                teams: Vec::new(),
                project: None,
                end_user: None,
                roles: Vec::new(),
                scopes: Vec::new(),
                auth_method: AuthMethod::ApiKey,
                credential_ref: CredentialRef::default(),
                network: NetworkContext::default(),
                source_key: SourceKey::default(),
            },
        });
    }
    let failure = match presented.as_ref() {
        Some(key) if !key.expose_secret().is_empty() => AuthenticationFailure::Invalid,
        Some(_) | None => AuthenticationFailure::Missing,
    };
    Err(failure)
}

mod tests;
