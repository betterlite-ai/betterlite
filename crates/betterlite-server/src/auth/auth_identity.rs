use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_auth::auth_identity::records::Principal;
use secrecy::SecretString;

use self::{AuthBackend::MasterKey, StageInput::Authenticated};
use crate::auth::auth_end_users::EndUserStageInput;
use crate::auth::auth_key_budgets::KeyBudgetStageInput;
use crate::auth::auth_key_lookup::KeyLookupStageInput;
pub(crate) use crate::auth::auth_key_lookup::master_key_context;
use crate::auth::auth_model_budgets::ModelBudgetStageInput;
use crate::auth::auth_model_grants::AgentModelGrantsStageInput;
use crate::auth::auth_model_patterns::{ModelPatternStageInput, run_stage};
use crate::auth::auth_org_lookup::OrgStageInput;
use crate::auth::auth_policy_support::{self, AuthPolicySupportStageInput};
use crate::auth::auth_pricing::PricingStageInput;
use crate::auth::auth_request_models::RequestModelStageInput;
use crate::auth::auth_request_policy::{self, AuthRequestPolicyStageInput};
use crate::auth::auth_resource_grants::{self, ResourceGrantStageInput};
use crate::auth::auth_route_policy::{self, RoutePolicyStageInput};
use crate::auth::auth_tag_budgets::TagBudgetStageInput;
use crate::auth::auth_tenant_budgets::TenantBudgetStageInput;
use crate::auth::entity_model_access::EntityModelStageInput;
pub(crate) use crate::auth::failures::StageFailure;
use crate::auth::identity_store::IdentityStageInput;
use crate::auth::jwt_keys::{self, JwtKeyStageInput};
use crate::auth::master_key_stage;
use crate::auth::model_grant_intersection::{self, GrantStageInput};
use crate::auth::organization_route_policy::{self, OrganizationPolicyStageInput};
use crate::auth::request_body_safety::{self, RequestBodySafetyStageInput};
use crate::auth::team_budget_admission::TeamBudgetStageInput;
use crate::auth::trusted_network::{self, TrustedNetworkStageInput};
use crate::auth::virtual_key_routes::{self, VirtualKeyRouteStageInput};
use crate::error_body::AuthenticationFailure;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthBackend {
    AuthKeyLookup,
    JwtKeys,
    MasterKey,
    TrustedNetwork,
    AuthRequestPolicy,
    RequestBodySafety,
    AuthVirtualKeyRoutes,
    AuthRequestModels,
    AuthModelPatterns,
    AuthModelGrants,
    EntityModelAccess,
    AuthResourceGrants,
    AuthPricing,
    AuthTagBudgets,
    AuthKeyBudgets,
    AuthModelBudgets,
    TeamBudgetAdmission,
    AuthTenantBudgets,
    OrganizationRoutePolicy,
    AuthRoutePolicy,
    AuthOrgLookup,
    AuthEndUsers,
    AuthPolicySupport,
    ModelGrantIntersection,
}

// B052 owns the stage order; the JWT fetch runs before the master-key comparison, as in LiteLLM.
pub(crate) const INSTALLED_STAGES: &[AuthBackend] = &[
    AuthBackend::JwtKeys,
    AuthBackend::AuthKeyLookup,
    MasterKey,
    AuthBackend::TrustedNetwork,
    AuthBackend::AuthRequestPolicy,
    AuthBackend::RequestBodySafety,
    AuthBackend::AuthVirtualKeyRoutes,
    AuthBackend::AuthRequestModels,
    AuthBackend::AuthModelPatterns,
    AuthBackend::AuthModelGrants,
    AuthBackend::EntityModelAccess,
    AuthBackend::AuthResourceGrants,
    AuthBackend::AuthPricing,
    AuthBackend::AuthTagBudgets,
    AuthBackend::AuthKeyBudgets,
    AuthBackend::TeamBudgetAdmission,
    AuthBackend::OrganizationRoutePolicy,
    AuthBackend::AuthTenantBudgets,
    AuthBackend::AuthOrgLookup,
    AuthBackend::AuthEndUsers,
    AuthBackend::AuthPolicySupport,
    AuthBackend::ModelGrantIntersection,
    AuthBackend::AuthRoutePolicy,
    AuthBackend::AuthModelBudgets,
];

#[derive(Clone, Debug)]
pub enum StageInput {
    AuthKeyLookup(Box<KeyLookupStageInput>),
    AuthIdentity(Box<IdentityStageInput>),
    Authenticated(Box<AuthContext>),
    JwtKeys(Box<JwtKeyStageInput>),
    MasterKey(Option<SecretString>),
    TrustedNetwork(Box<TrustedNetworkStageInput>),
    AuthRequestPolicy(Box<AuthRequestPolicyStageInput>),
    RequestBodySafety(Box<RequestBodySafetyStageInput>),
    AuthVirtualKeyRoutes(Box<VirtualKeyRouteStageInput>),
    AuthRequestModels(Box<RequestModelStageInput>),
    AuthModelPatterns(Box<ModelPatternStageInput>),
    AuthModelGrants(Box<AgentModelGrantsStageInput>),
    EntityModelAccess(Box<EntityModelStageInput>),
    AuthResourceGrants(Box<ResourceGrantStageInput>),
    AuthPricing(Box<PricingStageInput>),
    AuthTagBudgets(Box<TagBudgetStageInput>),
    AuthKeyBudgets(Box<KeyBudgetStageInput>),
    AuthModelBudgets(Box<ModelBudgetStageInput>),
    TeamBudgetAdmission(Box<TeamBudgetStageInput>),
    AuthTenantBudgets(Box<TenantBudgetStageInput>),
    OrganizationRoutePolicy(Box<OrganizationPolicyStageInput>),
    AuthRoutePolicy(Box<RoutePolicyStageInput>),
    AuthOrgLookup(Box<OrgStageInput>),
    AuthEndUsers(Box<EndUserStageInput>),
    AuthPolicySupport(Box<AuthPolicySupportStageInput>),
    ModelGrantIntersection(Box<GrantStageInput>),
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

impl StageInput {
    // the prefetch reads the cache, database and router, so the caller awaits it on the owned input
    pub(crate) async fn prefetch_model_grants(
        stage: Box<GrantStageInput>,
    ) -> Result<Self, StageFailure> {
        model_grant_intersection::prefetch(&stage)
            .await
            .map_err(StageFailure::ModelGrants)?;
        Ok(Self::ModelGrantIntersection(stage))
    }

    const fn backend(&self) -> AuthBackend {
        match self {
            Self::AuthKeyLookup(_) | Self::AuthIdentity(_) | Self::Authenticated(_) => {
                AuthBackend::AuthKeyLookup
            }
            Self::JwtKeys(_) => AuthBackend::JwtKeys,
            Self::MasterKey(_) => MasterKey,
            Self::TrustedNetwork(_) => AuthBackend::TrustedNetwork,
            Self::AuthRequestPolicy(_) => AuthBackend::AuthRequestPolicy,
            Self::RequestBodySafety(_) => AuthBackend::RequestBodySafety,
            Self::AuthVirtualKeyRoutes(_) => AuthBackend::AuthVirtualKeyRoutes,
            Self::AuthRequestModels(_) => AuthBackend::AuthRequestModels,
            Self::AuthModelPatterns(_) => AuthBackend::AuthModelPatterns,
            Self::EntityModelAccess(_) => AuthBackend::EntityModelAccess,
            Self::AuthModelGrants(_) => AuthBackend::AuthModelGrants,
            Self::AuthResourceGrants(_) => AuthBackend::AuthResourceGrants,
            Self::AuthPricing(_) => AuthBackend::AuthPricing,
            Self::AuthTagBudgets(_) => AuthBackend::AuthTagBudgets,
            Self::AuthKeyBudgets(_) => AuthBackend::AuthKeyBudgets,
            Self::AuthModelBudgets(_) => AuthBackend::AuthModelBudgets,
            Self::TeamBudgetAdmission(_) => AuthBackend::TeamBudgetAdmission,
            Self::AuthTenantBudgets(_) => AuthBackend::AuthTenantBudgets,
            Self::OrganizationRoutePolicy(_) => AuthBackend::OrganizationRoutePolicy,
            Self::AuthRoutePolicy(_) => AuthBackend::AuthRoutePolicy,
            Self::AuthOrgLookup(_) => AuthBackend::AuthOrgLookup,
            Self::AuthEndUsers(_) => AuthBackend::AuthEndUsers,
            Self::AuthPolicySupport(_) => AuthBackend::AuthPolicySupport,
            Self::ModelGrantIntersection(_) => AuthBackend::ModelGrantIntersection,
        }
    }
}

// the master key stage forwards the identity key lookup already read; no other stage runs another stage's input
fn require_backend(backend: AuthBackend, input: &StageInput) -> Result<(), StageFailure> {
    let key_pass_through = backend == MasterKey && matches!(input, Authenticated(_));
    if backend == input.backend() || key_pass_through {
        return Ok(());
    }
    Err(StageFailure::from(AuthenticationFailure::Missing))
}

pub(crate) fn dispatch(
    backend: AuthBackend,
    input: &StageInput,
    master_key: &SecretString,
) -> Result<AuthContext, StageFailure> {
    require_backend(backend, input)?;
    match input {
        Authenticated(context) => Ok((**context).clone()),
        StageInput::EntityModelAccess(stage) => Ok(stage.context.clone()),
        StageInput::AuthModelGrants(stage) => Ok(stage.context.clone()),
        StageInput::AuthPricing(stage) => Ok(stage.context.clone()),
        StageInput::AuthTagBudgets(stage) => Ok(stage.context.clone()),
        StageInput::AuthKeyBudgets(stage) => Ok(stage.context.clone()),
        StageInput::AuthModelBudgets(stage) => Ok(stage.context.clone()),
        StageInput::TeamBudgetAdmission(stage) => Ok(stage.context.clone()),
        StageInput::AuthTenantBudgets(stage) => Ok(stage.context.clone()),
        StageInput::AuthOrgLookup(stage) => Ok(stage.context.clone()),
        StageInput::AuthEndUsers(stage) => Ok(stage.context.clone()),
        StageInput::ModelGrantIntersection(stage) => Ok(stage.context.clone()),
        StageInput::AuthKeyLookup(_) | StageInput::AuthIdentity(_) => {
            Err(StageFailure::from(AuthenticationFailure::Missing))
        }
        StageInput::JwtKeys(_) => jwt_keys::run_stage(input),
        StageInput::MasterKey(_) => master_key_stage(input, master_key).map_err(StageFailure::from),
        StageInput::TrustedNetwork(stage) => trusted_network::run_stage(stage),
        StageInput::AuthRequestPolicy(stage) => {
            auth_request_policy::run_stage(stage).map_err(StageFailure::AuthRequestPolicy)
        }
        StageInput::RequestBodySafety(stage) => {
            request_body_safety::run_stage(stage).map_err(StageFailure::RequestBodySafety)
        }
        StageInput::AuthVirtualKeyRoutes(stage) => {
            virtual_key_routes::run_stage(stage).map_err(StageFailure::VirtualKeyRoutes)
        }
        StageInput::AuthRequestModels(stage) => Ok(stage.context.clone()),
        StageInput::AuthModelPatterns(stage) => run_stage(stage).map_err(StageFailure::from),
        StageInput::AuthResourceGrants(stage) => {
            auth_resource_grants::run_stage(stage).map_err(StageFailure::ResourceGrants)
        }
        StageInput::OrganizationRoutePolicy(stage) => organization_route_policy::run_stage(stage)
            .map_err(StageFailure::OrganizationRoutePolicy),
        StageInput::AuthRoutePolicy(stage) => auth_route_policy::run_stage(stage),
        StageInput::AuthPolicySupport(stage) => {
            auth_policy_support::run_stage(stage).map_err(StageFailure::AuthPolicySupport)
        }
    }
}

mod tests;
