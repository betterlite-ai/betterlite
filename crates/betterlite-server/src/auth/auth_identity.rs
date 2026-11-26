use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_error::proxy_exception::ProxyException;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use crate::auth::auth_context_sources;
use crate::auth::auth_end_users::EndUserStageInput;
use crate::auth::auth_key_budgets::KeyBudgetStageInput;
use crate::auth::auth_model_patterns::{ModelAccessRejection, ModelPatternStageInput, run_stage};
use crate::auth::auth_org_lookup::OrgStageInput;
use crate::auth::auth_request_models::{RequestModelStageInput, resolve};
use crate::auth::auth_request_policy::{self, AuthRequestPolicyStageInput};
use crate::auth::auth_resource_grants::{self, ResourceGrantStageInput};
use crate::auth::auth_tag_budgets::TagBudgetStageInput;
use crate::auth::entity_model_access::EntityModelStageInput;
use crate::auth::organization_route_policy::{self, OrganizationPolicyStageInput};
use crate::auth::request_body_safety::{self, RequestBodySafetyStageInput};
use crate::auth::team_budget_admission::TeamBudgetStageInput;
use crate::auth::virtual_key_routes::{self, VirtualKeyRouteStageInput};
use crate::error_body::AuthenticationFailure;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthBackend {
    MasterKey,
    AuthRequestPolicy,
    RequestBodySafety,
    AuthVirtualKeyRoutes,
    AuthRequestModels,
    AuthModelPatterns,
    EntityModelAccess,
    AuthResourceGrants,
    AuthTagBudgets,
    AuthKeyBudgets,
    TeamBudgetAdmission,
    OrganizationRoutePolicy,
    AuthOrgLookup,
    AuthEndUsers,
    AuthContextSources,
}

// B052 owns the final stage order; the master-key stage authenticates before the model stages run.
pub(crate) const INSTALLED_STAGES: &[AuthBackend] = &[
    AuthBackend::MasterKey,
    AuthBackend::AuthRequestPolicy,
    AuthBackend::RequestBodySafety,
    AuthBackend::AuthVirtualKeyRoutes,
    AuthBackend::AuthRequestModels,
    AuthBackend::AuthModelPatterns,
    AuthBackend::EntityModelAccess,
    AuthBackend::AuthResourceGrants,
    AuthBackend::AuthTagBudgets,
    AuthBackend::AuthKeyBudgets,
    AuthBackend::TeamBudgetAdmission,
    AuthBackend::OrganizationRoutePolicy,
    AuthBackend::AuthOrgLookup,
    AuthBackend::AuthEndUsers,
    AuthBackend::AuthContextSources,
];

#[derive(Clone, Debug)]
pub enum StageInput {
    MasterKey(Option<SecretString>),
    AuthRequestPolicy(Box<AuthRequestPolicyStageInput>),
    RequestBodySafety(Box<RequestBodySafetyStageInput>),
    AuthVirtualKeyRoutes(Box<VirtualKeyRouteStageInput>),
    AuthRequestModels(Box<RequestModelStageInput>),
    AuthModelPatterns(Box<ModelPatternStageInput>),
    EntityModelAccess(Box<EntityModelStageInput>),
    AuthResourceGrants(Box<ResourceGrantStageInput>),
    AuthTagBudgets(Box<TagBudgetStageInput>),
    AuthKeyBudgets(Box<KeyBudgetStageInput>),
    TeamBudgetAdmission(Box<TeamBudgetStageInput>),
    OrganizationRoutePolicy(Box<OrganizationPolicyStageInput>),
    AuthOrgLookup(Box<OrgStageInput>),
    AuthEndUsers(Box<EndUserStageInput>),
    AuthContextSources(Box<RequestModelStageInput>),
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
    EntityModelAccess(Box<ProxyException>),
    AuthRequestPolicy(Box<ProxyException>),
    ResourceGrants(Box<ProxyException>),
    TagBudgets(Box<ProxyException>),
    RequestBodySafety(Box<ProxyException>),
    VirtualKeyRoutes(Box<ProxyException>),
    KeyBudgets(Box<ProxyException>),
    TeamBudgets(Box<ProxyException>),
    OrganizationRoutePolicy(Box<ProxyException>),
    OrgLookup(Box<ProxyException>),
    EndUsers(Box<ProxyException>),
    AuthContextSources(Box<ProxyException>),
}

impl From<AuthenticationFailure> for StageFailure {
    fn from(failure: AuthenticationFailure) -> Self {
        Self::Authentication(Box::new(failure))
    }
}

impl StageInput {
    const fn backend(&self) -> AuthBackend {
        match self {
            Self::MasterKey(_) => AuthBackend::MasterKey,
            Self::AuthRequestPolicy(_) => AuthBackend::AuthRequestPolicy,
            Self::RequestBodySafety(_) => AuthBackend::RequestBodySafety,
            Self::AuthVirtualKeyRoutes(_) => AuthBackend::AuthVirtualKeyRoutes,
            Self::AuthRequestModels(_) => AuthBackend::AuthRequestModels,
            Self::AuthModelPatterns(_) => AuthBackend::AuthModelPatterns,
            Self::EntityModelAccess(_) => AuthBackend::EntityModelAccess,
            Self::AuthResourceGrants(_) => AuthBackend::AuthResourceGrants,
            Self::AuthTagBudgets(_) => AuthBackend::AuthTagBudgets,
            Self::AuthKeyBudgets(_) => AuthBackend::AuthKeyBudgets,
            Self::TeamBudgetAdmission(_) => AuthBackend::TeamBudgetAdmission,
            Self::OrganizationRoutePolicy(_) => AuthBackend::OrganizationRoutePolicy,
            Self::AuthOrgLookup(_) => AuthBackend::AuthOrgLookup,
            Self::AuthEndUsers(_) => AuthBackend::AuthEndUsers,
            Self::AuthContextSources(_) => AuthBackend::AuthContextSources,
        }
    }
}

pub(crate) fn dispatch(
    backend: AuthBackend,
    input: &StageInput,
    master_key: &SecretString,
) -> Result<AuthContext, StageFailure> {
    if backend != input.backend() {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    }
    match input {
        StageInput::MasterKey(_) => master_key_stage(input, master_key).map_err(StageFailure::from),
        StageInput::AuthRequestPolicy(stage) => {
            auth_request_policy::run_stage(stage).map_err(StageFailure::AuthRequestPolicy)
        }
        StageInput::RequestBodySafety(stage) => {
            request_body_safety::run_stage(stage).map_err(StageFailure::RequestBodySafety)
        }
        StageInput::AuthVirtualKeyRoutes(stage) => {
            virtual_key_routes::run_stage(stage).map_err(StageFailure::VirtualKeyRoutes)
        }
        StageInput::AuthRequestModels(stage) => auth_request_models_stage(stage),
        StageInput::AuthModelPatterns(stage) => {
            run_stage(stage).map_err(|rejection| StageFailure::ModelAccess(Box::new(rejection)))
        }
        StageInput::EntityModelAccess(stage) => Ok(stage.context.clone()),
        StageInput::AuthResourceGrants(stage) => {
            auth_resource_grants::run_stage(stage).map_err(StageFailure::ResourceGrants)
        }
        StageInput::AuthTagBudgets(stage) => Ok(stage.context.clone()),
        StageInput::AuthKeyBudgets(stage) => Ok(stage.context.clone()),
        StageInput::TeamBudgetAdmission(stage) => Ok(stage.context.clone()),
        StageInput::OrganizationRoutePolicy(stage) => organization_route_policy::run_stage(stage)
            .map_err(StageFailure::OrganizationRoutePolicy),
        StageInput::AuthOrgLookup(stage) => Ok(stage.context.clone()),
        StageInput::AuthEndUsers(stage) => Ok(stage.context.clone()),
        StageInput::AuthContextSources(stage) => {
            auth_context_sources::run_stage(stage).map_err(StageFailure::AuthContextSources)
        }
    }
}

fn auth_request_models_stage(stage: &RequestModelStageInput) -> Result<AuthContext, StageFailure> {
    match resolve(stage) {
        Ok(_) => Ok(stage.context.clone()),
        Err(error) => {
            tracing::debug!(error = %error, "the request model stage failed");
            Err(StageFailure::from(AuthenticationFailure::Missing))
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
