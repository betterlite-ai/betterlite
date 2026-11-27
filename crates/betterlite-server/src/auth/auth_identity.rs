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
use crate::auth::auth_key_lookup::KeyLookupStageInput;
use crate::auth::auth_model_patterns::{ModelAccessRejection, ModelPatternStageInput, run_stage};
use crate::auth::auth_org_lookup::OrgStageInput;
use crate::auth::auth_policy_support::{self, AuthPolicySupportStageInput};
use crate::auth::auth_request_models::{RequestModelStageInput, resolve};
use crate::auth::auth_request_policy::{self, AuthRequestPolicyStageInput};
use crate::auth::auth_resource_grants::{self, ResourceGrantStageInput};
use crate::auth::auth_tag_budgets::TagBudgetStageInput;
use crate::auth::auth_tenant_budgets::TenantBudgetStageInput;
use crate::auth::entity_model_access::EntityModelStageInput;
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
    MasterKey,
    TrustedNetwork,
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
    AuthTenantBudgets,
    OrganizationRoutePolicy,
    AuthOrgLookup,
    AuthEndUsers,
    AuthPolicySupport,
    ModelGrantIntersection,
}

// B052 owns the final stage order; the master-key stage authenticates before the model stages run.
pub(crate) const INSTALLED_STAGES: &[AuthBackend] = &[
    AuthBackend::AuthKeyLookup,
    AuthBackend::MasterKey,
    AuthBackend::TrustedNetwork,
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
    AuthBackend::AuthTenantBudgets,
    AuthBackend::AuthOrgLookup,
    AuthBackend::AuthEndUsers,
    AuthBackend::AuthPolicySupport,
    AuthBackend::ModelGrantIntersection,
];

#[derive(Clone, Debug)]
pub enum StageInput {
    AuthKeyLookup(Box<KeyLookupStageInput>),
    Authenticated(Box<AuthContext>),
    MasterKey(Option<SecretString>),
    TrustedNetwork(Box<TrustedNetworkStageInput>),
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
    AuthTenantBudgets(Box<TenantBudgetStageInput>),
    OrganizationRoutePolicy(Box<OrganizationPolicyStageInput>),
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

pub(crate) enum StageFailure {
    Authentication(Box<AuthenticationFailure>),
    KeyLookup(Box<ProxyException>),
    ModelAccess(Box<ModelAccessRejection>),
    EntityModelAccess(Box<ProxyException>),
    TrustedNetwork(Box<ProxyException>),
    AuthRequestPolicy(Box<ProxyException>),
    ResourceGrants(Box<ProxyException>),
    TagBudgets(Box<ProxyException>),
    RequestBodySafety(Box<ProxyException>),
    VirtualKeyRoutes(Box<ProxyException>),
    KeyBudgets(Box<ProxyException>),
    TeamBudgets(Box<ProxyException>),
    TenantBudgets(Box<ProxyException>),
    OrganizationRoutePolicy(Box<ProxyException>),
    OrgLookup(Box<ProxyException>),
    EndUsers(Box<ProxyException>),
    AuthPolicySupport(Box<ProxyException>),
    ModelGrants(Box<ProxyException>),
}

impl From<AuthenticationFailure> for StageFailure {
    fn from(failure: AuthenticationFailure) -> Self {
        Self::Authentication(Box::new(failure))
    }
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
            Self::AuthKeyLookup(_) | Self::Authenticated(_) => AuthBackend::AuthKeyLookup,
            Self::MasterKey(_) => AuthBackend::MasterKey,
            Self::TrustedNetwork(_) => AuthBackend::TrustedNetwork,
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
            Self::AuthTenantBudgets(_) => AuthBackend::AuthTenantBudgets,
            Self::OrganizationRoutePolicy(_) => AuthBackend::OrganizationRoutePolicy,
            Self::AuthOrgLookup(_) => AuthBackend::AuthOrgLookup,
            Self::AuthEndUsers(_) => AuthBackend::AuthEndUsers,
            Self::AuthPolicySupport(_) => AuthBackend::AuthPolicySupport,
            Self::ModelGrantIntersection(_) => AuthBackend::ModelGrantIntersection,
        }
    }
}

pub(crate) fn dispatch(
    backend: AuthBackend,
    input: &StageInput,
    master_key: &SecretString,
) -> Result<AuthContext, StageFailure> {
    let pass_through = matches!(input, StageInput::Authenticated(_));
    if !(backend == input.backend() || (backend == AuthBackend::MasterKey && pass_through)) {
        return Err(StageFailure::from(AuthenticationFailure::Missing));
    }
    match input {
        StageInput::AuthKeyLookup(_) => Err(StageFailure::from(AuthenticationFailure::Missing)),
        StageInput::Authenticated(context) => Ok((**context).clone()),
        StageInput::MasterKey(_) => master_key_stage(input, master_key).map_err(StageFailure::from),
        StageInput::TrustedNetwork(stage) => {
            trusted_network::run_stage(stage).map_err(StageFailure::TrustedNetwork)
        }
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
        StageInput::AuthTenantBudgets(stage) => Ok(stage.context.clone()),
        StageInput::OrganizationRoutePolicy(stage) => organization_route_policy::run_stage(stage)
            .map_err(StageFailure::OrganizationRoutePolicy),
        StageInput::AuthOrgLookup(stage) => Ok(stage.context.clone()),
        StageInput::AuthEndUsers(stage) => Ok(stage.context.clone()),
        StageInput::AuthPolicySupport(stage) => {
            auth_policy_support::run_stage(stage).map_err(StageFailure::AuthPolicySupport)
        }
        StageInput::ModelGrantIntersection(stage) => Ok(stage.context.clone()),
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
    if let Some(context) = master_key_context(presented.as_ref(), master_key) {
        return Ok(context);
    }
    let failure = match presented.as_ref() {
        Some(key) if !key.expose_secret().is_empty() => AuthenticationFailure::Invalid,
        Some(_) | None => AuthenticationFailure::Missing,
    };
    Err(failure)
}

// The master key authenticates the request before any virtual key is read, as LiteLLM's first check does.
pub(crate) fn master_key_context(
    presented: Option<&SecretString>,
    master_key: &SecretString,
) -> Option<AuthContext> {
    let matched = presented.is_some_and(|key| {
        !key.expose_secret().is_empty()
            && bool::from(
                key.expose_secret()
                    .as_bytes()
                    .ct_eq(master_key.expose_secret().as_bytes()),
            )
    });
    if !matched {
        return None;
    }
    Some(AuthContext {
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
    })
}

mod tests;
