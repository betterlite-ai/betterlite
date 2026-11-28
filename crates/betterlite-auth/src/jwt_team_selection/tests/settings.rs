use betterlite_config::sections::jwt_settings::Settings as JwtSettings;
use betterlite_config::sections::jwt_settings::groups::{
    RouteFields, RoutingFields, ScopeFields, TeamFields,
};

use super::super::settings::SelectionSettings;

#[test]
fn from_section_copies_every_group() {
    let section = JwtSettings {
        routes: RouteFields {
            team_allowed_routes: vec!["/v1/messages".to_owned()],
            ..RouteFields::default()
        },
        teams: TeamFields {
            team_id_jwt_field: Some("team_id".to_owned()),
            ..TeamFields::default()
        },
        scopes: ScopeFields {
            team_model_access: true,
            ..ScopeFields::default()
        },
        routing: RoutingFields {
            fallback_to_db_teams: true,
            team_claim_fallback: true,
            ..RoutingFields::default()
        },
        ..JwtSettings::default()
    };
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
    assert!(selection.requires_team());
}

#[test]
fn requires_team_needs_a_configured_claim_field() {
    assert!(!SelectionSettings::default().requires_team());
    let id_only = SelectionSettings {
        teams: TeamFields {
            team_id_jwt_field: Some("team_id".to_owned()),
            ..TeamFields::default()
        },
        ..SelectionSettings::default()
    };
    assert!(id_only.requires_team());
    let alias_only = SelectionSettings {
        teams: TeamFields {
            team_alias_jwt_field: Some("team_alias".to_owned()),
            ..TeamFields::default()
        },
        ..SelectionSettings::default()
    };
    assert!(alias_only.requires_team());
}
