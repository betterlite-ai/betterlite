use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_error::proxy_exception::ProxyException;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use crate::auth::auth_model_patterns::{ModelAccessRejection, ModelPatternStageInput, run_stage};
use crate::auth::auth_request_models::{RequestModelStageInput, resolve};
use crate::auth::auth_resource_grants::{self, ResourceGrantStageInput};
use crate::auth::auth_tag_budgets::TagBudgetStageInput;
use crate::auth::request_body_safety::{self, RequestBodySafetyStageInput};
use crate::error_body::AuthenticationFailure;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthBackend {
    MasterKey,
    RequestBodySafety,
    AuthRequestModels,
    AuthModelPatterns,
    AuthResourceGrants,
    AuthTagBudgets,
}

// B052 owns the final stage order; the master-key stage authenticates before the model stages run.
pub(crate) const INSTALLED_STAGES: &[AuthBackend] = &[
    AuthBackend::MasterKey,
    AuthBackend::RequestBodySafety,
    AuthBackend::AuthRequestModels,
    AuthBackend::AuthModelPatterns,
    AuthBackend::AuthResourceGrants,
    AuthBackend::AuthTagBudgets,
];

#[derive(Clone, Debug)]
pub enum StageInput {
    MasterKey(Option<SecretString>),
    RequestBodySafety(Box<RequestBodySafetyStageInput>),
    AuthRequestModels(Box<RequestModelStageInput>),
    AuthModelPatterns(Box<ModelPatternStageInput>),
    AuthResourceGrants(Box<ResourceGrantStageInput>),
    AuthTagBudgets(Box<TagBudgetStageInput>),
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

pub(crate) enum StageFailure {
    Authentication(Box<AuthenticationFailure>),
    ModelAccess(Box<ModelAccessRejection>),
    ResourceGrants(Box<ProxyException>),
    TagBudgets(Box<ProxyException>),
    RequestBodySafety(Box<ProxyException>),
}

impl From<AuthenticationFailure> for StageFailure {
    fn from(failure: AuthenticationFailure) -> Self {
        Self::Authentication(Box::new(failure))
    }
}

pub(crate) fn dispatch(
    backend: AuthBackend,
    input: &StageInput,
    master_key: &SecretString,
) -> Result<AuthContext, StageFailure> {
    match backend {
        AuthBackend::MasterKey => master_key_stage(input, master_key)
            .map_err(|failure| StageFailure::Authentication(Box::new(failure))),
        AuthBackend::RequestBodySafety => auth_request_body_safety_stage(input),
        AuthBackend::AuthRequestModels => auth_request_models_stage(input)
            .map_err(|failure| StageFailure::Authentication(Box::new(failure))),
        AuthBackend::AuthModelPatterns => auth_model_patterns_stage(input),
        AuthBackend::AuthResourceGrants => auth_resource_grants_stage(input),
        AuthBackend::AuthTagBudgets => {
            let StageInput::AuthTagBudgets(stage) = input else {
                return Err(StageFailure::from(AuthenticationFailure::Missing));
            };
            // the batch reads ran while this stage's input was assembled, so only the identity is left
            Ok(stage.context.clone())
        }
    }
}

fn auth_request_body_safety_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::RequestBodySafety(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    request_body_safety::run_stage(stage).map_err(StageFailure::RequestBodySafety)
}

fn auth_resource_grants_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::AuthResourceGrants(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    auth_resource_grants::run_stage(stage).map_err(StageFailure::ResourceGrants)
}

fn auth_model_patterns_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::AuthModelPatterns(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    run_stage(stage).map_err(|rejection| StageFailure::ModelAccess(Box::new(rejection)))
}
fn auth_request_models_stage(input: &StageInput) -> Result<AuthContext, AuthenticationFailure> {
    let StageInput::AuthRequestModels(stage) = input else {
        return Err(AuthenticationFailure::Missing);
    };
    match resolve(stage) {
        Ok(_) => Ok(stage.context.clone()),
        Err(error) => {
            tracing::debug!(error = %error, "the request model stage failed");
            Err(AuthenticationFailure::Missing)
        }
    }
}

fn master_key_stage(
    input: &StageInput,
    master_key: &SecretString,
) -> Result<AuthContext, AuthenticationFailure> {
    let StageInput::MasterKey(presented) = input else {
        return Err(AuthenticationFailure::Missing);
    };
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
