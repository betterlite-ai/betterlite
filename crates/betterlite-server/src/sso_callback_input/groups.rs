use betterlite_auth::jwt_claims::handler::JwtHandler;
use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_config::sections::sso_callback_input;

// LiteLLM_JWTAuth sets only team_ids_jwt_field here, so every other auth field keeps its declared default
#[must_use]
pub fn sso_jwt_handler(settings: &sso_callback_input::Settings) -> Option<JwtHandler> {
    let projection = settings.group_projection.as_ref()?;
    let mut handler = JwtHandler::default();
    handler.update_environment(
        JwtAuthSettings {
            team_id_jwt_field: None,
            team_ids_jwt_field: projection.sso_group_jwt_field.clone(),
            team_id_default: None,
            team_alias_jwt_field: None,
            org_id_jwt_field: None,
            org_alias_jwt_field: None,
            user_id_jwt_field: None,
            user_email_jwt_field: None,
            user_allowed_email_domain: None,
            user_id_upsert: false,
            end_user_id_jwt_field: None,
            agent_id_jwt_field: None,
            object_id_jwt_field: None,
        },
        Vec::new(),
        0,
    );
    Some(handler)
}
