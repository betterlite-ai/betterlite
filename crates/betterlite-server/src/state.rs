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

impl ServerState {
    // the slots resolve their settings once here, so a request never reads the config again
    #[expect(
        clippy::too_many_lines,
        reason = "one Slot::new per planned state entry"
    )]
    pub fn new(config: &Config, executor: Arc<ChatExecutor>, shared: Arc<Shared>) -> Self {
        Self {
            config: config.clone(),
            executor,
            adaptive_persistence: adaptive_persistence::Slot::new(config, &shared),
            auth_admission: auth_admission::Slot::new(config, &shared),
            auth_backend_dispatch: auth_backend_dispatch::Slot::new(config, &shared),
            auth_cached_sessions: auth_cached_sessions::Slot::new(config, &shared),
            auth_common_checks: auth_common_checks::Slot::new(config, &shared),
            auth_context_sources: auth_context_sources::Slot::new(config, &shared),
            auth_end_users: auth_end_users::Slot::new(config, &shared),
            auth_errors: auth_errors::Slot::new(config, &shared),
            auth_headers: auth_headers::Slot::new(config, &shared),
            auth_key_budgets: auth_key_budgets::Slot::new(config, &shared),
            auth_key_lookup: auth_key_lookup::Slot::new(config, &shared),
            auth_key_validation: auth_key_validation::Slot::new(config, &shared),
            auth_model_budgets: auth_model_budgets::Slot::new(config, &shared),
            auth_model_grants: auth_model_grants::Slot::new(config, &shared),
            auth_model_lists: auth_model_lists::Slot::new(config, &shared),
            auth_model_patterns: auth_model_patterns::Slot::new(config, &shared),
            auth_org_lookup: auth_org_lookup::Slot::new(config, &shared),
            auth_policy_support: auth_policy_support::Slot::new(config, &shared),
            auth_pricing: auth_pricing::Slot::new(config, &shared),
            auth_request_models: auth_request_models::Slot::new(config, &shared),
            auth_request_policy: auth_request_policy::Slot::new(config, &shared),
            auth_resource_grants: auth_resource_grants::Slot::new(config, &shared),
            auth_route_policy: auth_route_policy::Slot::new(config, &shared),
            auth_tag_budgets: auth_tag_budgets::Slot::new(config, &shared),
            auth_tenant_budgets: auth_tenant_budgets::Slot::new(config, &shared),
            budget_admission_handoff: budget_admission_handoff::Slot::new(config, &shared),
            callback_loggers: callback_loggers::Slot::new(config, &shared),
            central_auth_checks: central_auth_checks::Slot::new(config, &shared),
            cost_service: cost_service::Slot::new(config, &shared),
            credentials: credentials::Slot::new(config, &shared),
            current_spend: current_spend::Slot::new(config, &shared),
            custom_auth: custom_auth::Slot::new(config, &shared),
            end_user_identity: end_user_identity::Slot::new(config, &shared),
            entity_model_access: entity_model_access::Slot::new(config, &shared),
            identity_store: identity_store::Slot::new(config, &shared),
            jwt_auth: jwt_auth::Slot::new(config, &shared),
            jwt_db_teams: jwt_db_teams::Slot::new(config, &shared),
            jwt_key_resolution: jwt_key_resolution::Slot::new(config, &shared),
            jwt_keys: jwt_keys::Slot::new(config, &shared),
            jwt_permissions: jwt_permissions::Slot::new(config, &shared),
            jwt_provisioning: jwt_provisioning::Slot::new(config, &shared),
            jwt_team_authorization: jwt_team_authorization::Slot::new(config, &shared),
            jwt_team_selection: jwt_team_selection::Slot::new(config, &shared),
            jwt_verification: jwt_verification::Slot::new(config, &shared),
            key_context: key_context::Slot::new(config, &shared),
            key_rotation: key_rotation::Slot::new(config, &shared),
            master_database_auth: master_database_auth::Slot::new(config, &shared),
            model_grant_intersection: model_grant_intersection::Slot::new(config, &shared),
            oauth_tokens: oauth_tokens::Slot::new(config, &shared),
            organization_route_policy: organization_route_policy::Slot::new(config, &shared),
            ptu_rollup: ptu_rollup::Slot::new(config, &shared),
            request_body_safety: request_body_safety::Slot::new(config, &shared),
            request_intake: request_intake::Slot::new(config, &shared),
            request_setup: request_setup::Slot::new(config, &shared),
            spend_runtime: spend_runtime::Slot::new(config, &shared),
            team_budget_admission: team_budget_admission::Slot::new(config, &shared),
            trusted_network: trusted_network::Slot::new(config, &shared),
            virtual_key_auth: virtual_key_auth::Slot::new(config, &shared),
            virtual_key_routes: virtual_key_routes::Slot::new(config, &shared),
            shared,
        }
    }
}
