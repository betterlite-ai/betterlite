pub mod adaptive_persistence;
pub mod auth_admission;
pub mod auth_backend_dispatch;
pub mod auth_cached_sessions;
pub mod auth_common_checks;
pub mod auth_context_sources;
pub mod auth_end_users;
pub mod auth_errors;
pub mod auth_headers;
pub mod auth_key_budgets;
pub mod auth_key_lookup;
pub mod auth_key_validation;
pub mod auth_model_budgets;
pub mod auth_model_grants;
pub mod auth_model_lists;
pub mod auth_model_patterns;
pub mod auth_org_lookup;
pub mod auth_policy_support;
pub mod auth_pricing;
pub mod auth_request_models;
pub mod auth_request_policy;
pub mod auth_resource_grants;
pub mod auth_route_policy;
pub mod auth_tag_budgets;
pub mod auth_tenant_budgets;
pub mod budget_admission_handoff;
pub mod callback_loggers;
pub mod central_auth_checks;
pub mod cost_service;
pub mod credentials;
pub mod current_spend;
pub mod custom_auth;
pub mod end_user_identity;
pub mod entity_model_access;
pub mod health;
pub mod identity_store;
pub mod jwt_auth;
pub mod jwt_db_teams;
pub mod jwt_key_resolution;
pub mod jwt_keys;
pub mod jwt_permissions;
pub mod jwt_provisioning;
pub mod jwt_team_authorization;
pub mod jwt_team_selection;
pub mod jwt_verification;
pub mod key_context;
pub mod key_rotation;
pub mod master_database_auth;
pub mod model_grant_intersection;
pub mod oauth_tokens;
pub mod organization_route_policy;
pub mod ptu_rollup;
pub mod request_body_safety;
pub mod request_intake;
pub mod request_setup;
pub mod sdk_callbacks;
pub mod shared;
pub mod spend_runtime;
pub mod team_budget_admission;
pub mod trusted_network;
pub mod virtual_key_auth;
pub mod virtual_key_routes;

use crate::state::shared::Shared;
use betterlite_config::types::Config;
use betterlite_openai::chat::execution::ChatExecutor;
use std::sync::Arc;

pub struct ServerState {
    pub config: Config,
    pub executor: Arc<ChatExecutor>,
    pub shared: Arc<Shared>,
    pub adaptive_persistence: adaptive_persistence::Slot,
    pub auth_admission: auth_admission::Slot,
    pub auth_backend_dispatch: auth_backend_dispatch::Slot,
    pub auth_cached_sessions: auth_cached_sessions::Slot,
    pub auth_common_checks: auth_common_checks::Slot,
    pub auth_context_sources: auth_context_sources::Slot,
    pub auth_end_users: auth_end_users::Slot,
    pub auth_errors: auth_errors::Slot,
    pub auth_headers: auth_headers::Slot,
    pub auth_key_budgets: auth_key_budgets::Slot,
    pub auth_key_lookup: auth_key_lookup::Slot,
    pub auth_key_validation: auth_key_validation::Slot,
    pub auth_model_budgets: auth_model_budgets::Slot,
    pub auth_model_grants: auth_model_grants::Slot,
    pub auth_model_lists: auth_model_lists::Slot,
    pub auth_model_patterns: auth_model_patterns::Slot,
    pub auth_org_lookup: auth_org_lookup::Slot,
    pub auth_policy_support: auth_policy_support::Slot,
    pub auth_pricing: auth_pricing::Slot,
    pub auth_request_models: auth_request_models::Slot,
    pub auth_request_policy: auth_request_policy::Slot,
    pub auth_resource_grants: auth_resource_grants::Slot,
    pub auth_route_policy: auth_route_policy::Slot,
    pub auth_tag_budgets: auth_tag_budgets::Slot,
    pub auth_tenant_budgets: auth_tenant_budgets::Slot,
    pub budget_admission_handoff: budget_admission_handoff::Slot,
    pub callback_loggers: callback_loggers::Slot,
    pub central_auth_checks: central_auth_checks::Slot,
    pub cost_service: cost_service::Slot,
    pub credentials: credentials::Slot,
    pub current_spend: current_spend::Slot,
    pub custom_auth: custom_auth::Slot,
    pub end_user_identity: end_user_identity::Slot,
    pub entity_model_access: entity_model_access::Slot,
    pub identity_store: identity_store::Slot,
    pub jwt_auth: jwt_auth::Slot,
    pub jwt_db_teams: jwt_db_teams::Slot,
    pub jwt_key_resolution: jwt_key_resolution::Slot,
    pub jwt_keys: jwt_keys::Slot,
    pub jwt_permissions: jwt_permissions::Slot,
    pub jwt_provisioning: jwt_provisioning::Slot,
    pub jwt_team_authorization: jwt_team_authorization::Slot,
    pub jwt_team_selection: jwt_team_selection::Slot,
    pub jwt_verification: jwt_verification::Slot,
    pub key_context: key_context::Slot,
    pub key_rotation: key_rotation::Slot,
    pub master_database_auth: master_database_auth::Slot,
    pub model_grant_intersection: model_grant_intersection::Slot,
    pub oauth_tokens: oauth_tokens::Slot,
    pub organization_route_policy: organization_route_policy::Slot,
    pub ptu_rollup: ptu_rollup::Slot,
    pub request_body_safety: request_body_safety::Slot,
    pub request_intake: request_intake::Slot,
    pub request_setup: request_setup::Slot,
    pub spend_runtime: spend_runtime::Slot,
    pub team_budget_admission: team_budget_admission::Slot,
    pub trusted_network: trusted_network::Slot,
    pub virtual_key_auth: virtual_key_auth::Slot,
    pub virtual_key_routes: virtual_key_routes::Slot,
}

// each entry gets one `Slot::new` call; the macro keeps the 59-line list out of one oversized function
macro_rules! server_state_constructor {
    ($($slot:ident),* $(,)?) => {
        impl ServerState {
            // the slots resolve their settings once here, so a request never reads the config again
            pub fn new(config: &Config, executor: Arc<ChatExecutor>, shared: Arc<Shared>) -> Self {
                Self {
                    config: config.clone(),
                    executor,
                    $($slot: $slot::Slot::new(config, &shared),)*
                    shared,
                }
            }
        }
    };
}

server_state_constructor! {
    adaptive_persistence,
    auth_admission,
    auth_backend_dispatch,
    auth_cached_sessions,
    auth_common_checks,
    auth_context_sources,
    auth_end_users,
    auth_errors,
    auth_headers,
    auth_key_budgets,
    auth_key_lookup,
    auth_key_validation,
    auth_model_budgets,
    auth_model_grants,
    auth_model_lists,
    auth_model_patterns,
    auth_org_lookup,
    auth_policy_support,
    auth_pricing,
    auth_request_models,
    auth_request_policy,
    auth_resource_grants,
    auth_route_policy,
    auth_tag_budgets,
    auth_tenant_budgets,
    budget_admission_handoff,
    callback_loggers,
    central_auth_checks,
    cost_service,
    credentials,
    current_spend,
    custom_auth,
    end_user_identity,
    entity_model_access,
    identity_store,
    jwt_auth,
    jwt_db_teams,
    jwt_key_resolution,
    jwt_keys,
    jwt_permissions,
    jwt_provisioning,
    jwt_team_authorization,
    jwt_team_selection,
    jwt_verification,
    key_context,
    key_rotation,
    master_database_auth,
    model_grant_intersection,
    oauth_tokens,
    organization_route_policy,
    ptu_rollup,
    request_body_safety,
    request_intake,
    request_setup,
    spend_runtime,
    team_budget_admission,
    trusted_network,
    virtual_key_auth,
    virtual_key_routes,
}
