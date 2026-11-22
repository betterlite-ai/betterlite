use std::sync::Arc;

use betterlite_auth::jwt_claims::handler::{AgentLookup, JwtHandler};
use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_config::sections::jwt_settings;

// LiteLLM_JWTAuth carries no leeway in betterlite's port, so every boot starts from clock tolerance zero
const JWT_LEEWAY_SECONDS: i64 = 0;

#[must_use]
pub fn init_jwt_auth(
    settings: &jwt_settings::Settings,
    agent_lookup: Arc<dyn AgentLookup>,
    mut handler: JwtHandler,
) -> JwtHandler {
    let auth_settings = JwtAuthSettings {
        team_id_jwt_field: settings.teams.team_id_jwt_field.clone(),
        team_ids_jwt_field: settings.teams.team_ids_jwt_field.clone(),
        team_id_default: settings.teams.team_id_default.clone(),
        team_alias_jwt_field: settings.teams.team_alias_jwt_field.clone(),
        org_id_jwt_field: settings.claims.org_id_jwt_field.clone(),
        org_alias_jwt_field: settings.claims.org_alias_jwt_field.clone(),
        user_id_jwt_field: settings.claims.user_id_jwt_field.clone(),
        user_email_jwt_field: settings.claims.user_email_jwt_field.clone(),
        user_allowed_email_domain: settings.claims.user_allowed_email_domain.clone(),
        user_id_upsert: settings.claims.user_id_upsert,
        end_user_id_jwt_field: settings.claims.end_user_id_jwt_field.clone(),
        agent_id_jwt_field: settings.claims.agent_id_jwt_field.clone(),
        object_id_jwt_field: settings.rbac.object_id_jwt_field.clone(),
    };
    let trusted_issuers = settings
        .routing
        .issuers
        .iter()
        .flatten()
        .map(|issuer| issuer.issuer.clone())
        .collect();
    handler.update_environment(auth_settings, trusted_issuers, JWT_LEEWAY_SECONDS);
    handler.bind_agent_lookup(agent_lookup)
}
