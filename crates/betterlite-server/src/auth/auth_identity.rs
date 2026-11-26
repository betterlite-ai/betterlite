use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_error::proxy_exception::ProxyException;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use crate::auth::auth_end_users::EndUserStageInput;
use crate::auth::auth_key_budgets::KeyBudgetStageInput;
use crate::auth::auth_model_grants::AgentModelGrantsStageInput;
use crate::auth::auth_model_patterns::{ModelAccessRejection, ModelPatternStageInput, run_stage};
use crate::auth::auth_org_lookup::OrgStageInput;
use crate::auth::auth_request_models::{RequestModelStageInput, resolve};
use crate::auth::auth_request_policy::{self, AuthRequestPolicyStageInput};
use crate::auth::auth_resource_grants::{self, ResourceGrantStageInput};
use crate::auth::auth_tag_budgets::TagBudgetStageInput;
use crate::auth::entity_model_access::EntityModelStageInput;
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
    AuthModelGrants,
    EntityModelAccess,
    AuthResourceGrants,
    AuthTagBudgets,
    AuthKeyBudgets,
    TeamBudgetAdmission,
    AuthOrgLookup,
    AuthEndUsers,
}

// B052 owns the final stage order; the master-key stage authenticates before the model stages run.
pub(crate) const INSTALLED_STAGES: &[AuthBackend] = &[
    AuthBackend::MasterKey,
    AuthBackend::AuthRequestPolicy,
    AuthBackend::RequestBodySafety,
    AuthBackend::AuthVirtualKeyRoutes,
    AuthBackend::AuthRequestModels,
    AuthBackend::AuthModelPatterns,
    AuthBackend::AuthModelGrants,
    AuthBackend::EntityModelAccess,
    AuthBackend::AuthResourceGrants,
    AuthBackend::AuthTagBudgets,
    AuthBackend::AuthKeyBudgets,
    AuthBackend::TeamBudgetAdmission,
    AuthBackend::AuthOrgLookup,
    AuthBackend::AuthEndUsers,
];

#[derive(Clone, Debug)]
pub enum StageInput {
    MasterKey(Option<SecretString>),
    AuthRequestPolicy(Box<AuthRequestPolicyStageInput>),
    RequestBodySafety(Box<RequestBodySafetyStageInput>),
    AuthVirtualKeyRoutes(Box<VirtualKeyRouteStageInput>),
    AuthRequestModels(Box<RequestModelStageInput>),
    AuthModelPatterns(Box<ModelPatternStageInput>),
    AuthModelGrants(Box<AgentModelGrantsStageInput>),
    EntityModelAccess(Box<EntityModelStageInput>),
    AuthResourceGrants(Box<ResourceGrantStageInput>),
    AuthTagBudgets(Box<TagBudgetStageInput>),
    AuthKeyBudgets(Box<KeyBudgetStageInput>),
    TeamBudgetAdmission(Box<TeamBudgetStageInput>),
    AuthOrgLookup(Box<OrgStageInput>),
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
    ModelGrants(Box<ProxyException>),
    EntityModelAccess(Box<ProxyException>),
    AuthRequestPolicy(Box<ProxyException>),
    ResourceGrants(Box<ProxyException>),
    TagBudgets(Box<ProxyException>),
    RequestBodySafety(Box<ProxyException>),
    VirtualKeyRoutes(Box<ProxyException>),
    KeyBudgets(Box<ProxyException>),
    TeamBudgets(Box<ProxyException>),
    OrgLookup(Box<ProxyException>),
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
        AuthBackend::MasterKey => master_key_stage(input, master_key).map_err(StageFailure::from),
        AuthBackend::AuthRequestPolicy => auth_request_policy_stage(input),
        AuthBackend::RequestBodySafety => auth_request_body_safety_stage(input),
        AuthBackend::AuthVirtualKeyRoutes => auth_virtual_key_routes_stage(input),
        AuthBackend::AuthRequestModels => auth_request_models_stage(input),
        AuthBackend::AuthModelPatterns => auth_model_patterns_stage(input),
        AuthBackend::AuthModelGrants => auth_model_grants_stage(input),
        AuthBackend::EntityModelAccess => entity_model_access_stage(input),
        AuthBackend::AuthResourceGrants => auth_resource_grants_stage(input),
        AuthBackend::AuthTagBudgets => {
            let StageInput::AuthTagBudgets(stage) = input else {
                return Err(StageFailure::from(AuthenticationFailure::Missing));
            };
            // the batch reads ran while this stage's input was assembled, so only the identity is left
            Ok(stage.context.clone())
        }
        AuthBackend::AuthKeyBudgets => {
            let StageInput::AuthKeyBudgets(stage) = input else {
                return Err(StageFailure::from(AuthenticationFailure::Missing));
            };
            // the reads ran while the stage input was assembled, so the budget failures already surfaced there
            Ok(stage.context.clone())
        }
        AuthBackend::TeamBudgetAdmission => {
            let StageInput::TeamBudgetAdmission(stage) = input else {
                return Err(StageFailure::from(AuthenticationFailure::Missing));
            };
            // the member scope and budget checks ran in the prefetch, so only the identity is left
            Ok(stage.context.clone())
        }
        AuthBackend::AuthOrgLookup => {
            let StageInput::AuthOrgLookup(stage) = input else {
                return Err(StageFailure::from(AuthenticationFailure::Missing));
            };
            // the request-scoped lookup ran in the prefetch, so only the identity is left
            Ok(stage.context.clone())
        }
        AuthBackend::AuthEndUsers => auth_end_users_stage(input),
    }
}

fn entity_model_access_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::EntityModelAccess(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    // the grant checks ran in the prefetch, so only the identity is left
    Ok(stage.context.clone())
}

fn auth_model_grants_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::AuthModelGrants(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    // the agent and caller checks ran in the prefetch, so only the identity is left
    Ok(stage.context.clone())
}

fn auth_end_users_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::AuthEndUsers(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    // the lookup and the budget check ran in the prefetch, so only the identity is left
    Ok(stage.context.clone())
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

fn auth_resource_grants_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::AuthResourceGrants(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    auth_resource_grants::run_stage(stage).map_err(StageFailure::ResourceGrants)
}

fn auth_virtual_key_routes_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::AuthVirtualKeyRoutes(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    virtual_key_routes::run_stage(stage).map_err(StageFailure::VirtualKeyRoutes)
}

fn auth_model_patterns_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::AuthModelPatterns(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
    run_stage(stage).map_err(|rejection| StageFailure::ModelAccess(Box::new(rejection)))
}
fn auth_request_models_stage(input: &StageInput) -> Result<AuthContext, StageFailure> {
    let StageInput::AuthRequestModels(stage) = input else {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    };
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
