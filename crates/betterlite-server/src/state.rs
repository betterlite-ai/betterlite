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
pub mod central_auth_checks;
pub mod cost_service;
pub mod credentials;
pub mod current_spend;
pub mod custom_auth;
pub mod end_user_identity;
pub mod entity_model_access;
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

macro_rules! server_state {
    ($($slot:ident),* $(,)?) => {
        pub struct ServerState {
            pub config: Config,
            pub executor: Arc<ChatExecutor>,
            pub shared: Arc<Shared>,
            $(pub $slot: $slot::Slot,)*
        }

        impl ServerState {
            // the slots resolve their settings once here, so a request never reads the config again
            pub fn new(config: &Config, executor: Arc<ChatExecutor>, shared: Arc<Shared>) -> Self {
                Self {
                    config: config.clone(),
                    executor,
                    shared: Arc::clone(&shared),
                    $($slot: $slot::Slot::new(config, &shared),)*
                }
            }
        }
    };
}

server_state! {
    adaptive_persistence, auth_admission, auth_backend_dispatch, auth_cached_sessions,
    auth_common_checks, auth_context_sources, auth_end_users, auth_errors,
    auth_headers, auth_key_budgets, auth_key_lookup, auth_key_validation,
    auth_model_budgets, auth_model_grants, auth_model_lists, auth_model_patterns,
    auth_org_lookup, auth_policy_support, auth_pricing, auth_request_models,
    auth_request_policy, auth_resource_grants, auth_route_policy, auth_tag_budgets,
    auth_tenant_budgets, budget_admission_handoff, central_auth_checks, cost_service,
    credentials, current_spend, custom_auth, end_user_identity,
    entity_model_access, identity_store, jwt_auth, jwt_db_teams,
    jwt_key_resolution, jwt_keys, jwt_permissions, jwt_provisioning,
    jwt_team_authorization, jwt_team_selection, jwt_verification, key_context,
    key_rotation, master_database_auth, model_grant_intersection, oauth_tokens,
    organization_route_policy, ptu_rollup, request_body_safety, request_intake,
    request_setup, spend_runtime, team_budget_admission, trusted_network,
    virtual_key_auth, virtual_key_routes,
}
