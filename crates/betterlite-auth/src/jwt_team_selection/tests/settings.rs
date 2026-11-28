use betterlite_config::sections::jwt_settings::Settings as JwtSettings;

use super::super::settings::SelectionSettings;

#[test]
fn from_section_copies_every_group() {
    let mut section = JwtSettings::default();
    section.routes.team_allowed_routes = vec!["/v1/messages".to_owned()];
    section.teams.team_id_jwt_field = Some("team_id".to_owned());
    section.scopes.team_model_access = true;
    section.routing.fallback_to_db_teams = true;
    section.routing.team_claim_fallback = true;
    let selection = SelectionSettings::from_section(&section);
    assert_eq!(
        selection.teams.team_id_jwt_field.as_deref(),
        Some("team_id")
    );
    assert_eq!(
        selection.routes.team_allowed_routes,
        vec!["/v1/messages".to_owned()]
    );
    assert!(selection.scopes.team_model_access);
    assert!(selection.routing.fallback_to_db_teams);
    assert!(selection.routing.team_claim_fallback);
}
