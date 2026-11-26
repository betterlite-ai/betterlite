use axum::body::{Body, Bytes};
use axum::http::{Request, header};
use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_error::proxy_exception::ProxyException;
use secrecy::{ExposeSecret as _, SecretString};
use std::sync::Arc;
use subtle::ConstantTimeEq as _;

use crate::auth::auth_end_users::EndUserStageInput;
use crate::auth::auth_key_budgets::KeyBudgetStageInput;
use crate::auth::auth_model_patterns::{ModelAccessRejection, ModelPatternStageInput, run_stage};
use crate::auth::auth_org_lookup::OrgStageInput;
use crate::auth::auth_request_models::{RequestModelStageInput, resolve};
use crate::auth::auth_request_policy::{self, AuthRequestPolicyStageInput};
use crate::auth::auth_resource_grants::{self, ResourceGrantStageInput};
use crate::auth::auth_tag_budgets::TagBudgetStageInput;
use crate::auth::end_user_identity::EndUserIdentityStageInput;
use crate::auth::request_body_safety::{self, RequestBodySafetyStageInput};
use crate::auth::team_budget_admission::TeamBudgetStageInput;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthBackend {
    MasterKey,
    AuthRequestPolicy,
    RequestBodySafety,
    AuthRequestModels,
    AuthModelPatterns,
    AuthResourceGrants,
    AuthTagBudgets,
    AuthKeyBudgets,
    TeamBudgetAdmission,
    AuthOrgLookup,
    AuthEndUserIdentity,
    AuthEndUsers,
}

// B052 owns the final stage order; the master-key stage authenticates before the model stages run.
pub(crate) const INSTALLED_STAGES: &[AuthBackend] = &[
    AuthBackend::MasterKey,
    AuthBackend::AuthRequestPolicy,
    AuthBackend::RequestBodySafety,
    AuthBackend::AuthRequestModels,
    AuthBackend::AuthModelPatterns,
    AuthBackend::AuthResourceGrants,
    AuthBackend::AuthTagBudgets,
    AuthBackend::AuthKeyBudgets,
    AuthBackend::TeamBudgetAdmission,
    AuthBackend::AuthOrgLookup,
    AuthBackend::AuthEndUserIdentity,
    AuthBackend::AuthEndUsers,
];

#[derive(Clone, Debug)]
pub enum StageInput {
    MasterKey(Option<SecretString>),
    AuthRequestPolicy(Box<AuthRequestPolicyStageInput>),
    RequestBodySafety(Box<RequestBodySafetyStageInput>),
    AuthRequestModels(Box<RequestModelStageInput>),
    AuthModelPatterns(Box<ModelPatternStageInput>),
    AuthResourceGrants(Box<ResourceGrantStageInput>),
    AuthTagBudgets(Box<TagBudgetStageInput>),
    AuthKeyBudgets(Box<KeyBudgetStageInput>),
    TeamBudgetAdmission(Box<TeamBudgetStageInput>),
    AuthOrgLookup(Box<OrgStageInput>),
    AuthEndUserIdentity(Box<EndUserIdentityStageInput>),
    AuthEndUsers(Box<EndUserStageInput>),
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
    AuthRequestPolicy(Box<ProxyException>),
    ResourceGrants(Box<ProxyException>),
    TagBudgets(Box<ProxyException>),
    RequestBodySafety(Box<ProxyException>),
    KeyBudgets(Box<ProxyException>),
    TeamBudgets(Box<ProxyException>),
    OrgLookup(Box<ProxyException>),
    EndUserIdentity(Box<ProxyException>),
    EndUsers(Box<ProxyException>),
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
        AuthBackend::AuthRequestPolicy => auth_request_policy_stage(input),
        AuthBackend::RequestBodySafety => auth_request_body_safety_stage(input),
        AuthBackend::AuthRequestModels => auth_request_models_stage(input)
            .map_err(|failure| StageFailure::Authentication(Box::new(failure))),
        AuthBackend::AuthModelPatterns => auth_model_patterns_stage(input),
        AuthBackend::AuthResourceGrants => auth_resource_grants_stage(input),
        // these six stages read their probes while the input was assembled, so only the identity is left
        AuthBackend::AuthTagBudgets
        | AuthBackend::AuthKeyBudgets
        | AuthBackend::TeamBudgetAdmission
        | AuthBackend::AuthOrgLookup
        | AuthBackend::AuthEndUserIdentity
        | AuthBackend::AuthEndUsers => prefetched_identity(backend, input),
    }
}

// a prefetched stage hands back the identity its prefetch installed, and only for its own input
fn prefetched_identity(
    backend: AuthBackend,
    input: &StageInput,
) -> Result<AuthContext, StageFailure> {
    let (installed, context) = match input {
        StageInput::AuthTagBudgets(stage) => (AuthBackend::AuthTagBudgets, &stage.context),
        StageInput::AuthKeyBudgets(stage) => (AuthBackend::AuthKeyBudgets, &stage.context),
        StageInput::TeamBudgetAdmission(stage) => {
            (AuthBackend::TeamBudgetAdmission, &stage.context)
        }
        StageInput::AuthOrgLookup(stage) => (AuthBackend::AuthOrgLookup, &stage.context),
        StageInput::AuthEndUserIdentity(stage) => {
            (AuthBackend::AuthEndUserIdentity, &stage.context)
        }
        StageInput::AuthEndUsers(stage) => (AuthBackend::AuthEndUsers, &stage.context),
        StageInput::MasterKey(_)
        | StageInput::AuthRequestPolicy(_)
        | StageInput::RequestBodySafety(_)
        | StageInput::AuthRequestModels(_)
        | StageInput::AuthModelPatterns(_)
        | StageInput::AuthResourceGrants(_) => {
            return Err(StageFailure::from(AuthenticationFailure::Missing));
        }
    };
    if installed == backend {
        Ok(context.clone())
    } else {
        Err(StageFailure::from(AuthenticationFailure::Missing))
    }
}

fn auth_request_policy_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::AuthRequestPolicy(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    auth_request_policy::run_stage(stage).map_err(StageFailure::AuthRequestPolicy)
}

fn auth_request_body_safety_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::RequestBodySafety(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    request_body_safety::run_stage(stage).map_err(StageFailure::RequestBodySafety)
}

// auth.rs sits at its 300-line limit, so the body-safety stage input is built beside its dispatcher here
pub(crate) fn safety_stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    body: Option<&Bytes>,
) -> Result<RequestBodySafetyStageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let slot = request
        .extensions()
        .get::<Arc<ServerState>>()
        .map(|state| state.request_body_safety.clone());
    Ok(RequestBodySafetyStageInput {
        context,
        content_type,
        body: body.cloned().unwrap_or_default(),
        slot,
    })
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
