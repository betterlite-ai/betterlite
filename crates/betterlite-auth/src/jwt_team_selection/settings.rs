use betterlite_config::sections::jwt_settings::Settings as JwtSettings;
use betterlite_config::sections::jwt_settings::groups::{
    RouteFields, RoutingFields, ScopeFields, TeamFields,
};

// the typed jwt_settings values this leaf reads; B869 parses and validates them once, from the one section
#[derive(Clone, Debug, Default)]
pub struct SelectionSettings {
    pub teams: TeamFields,
    pub routes: RouteFields,
    pub scopes: ScopeFields,
    pub routing: RoutingFields,
}

impl SelectionSettings {
    #[must_use]
    pub fn from_section(section: &JwtSettings) -> Self {
        Self {
            teams: section.teams.clone(),
            routes: section.routes.clone(),
            scopes: section.scopes.clone(),
            routing: section.routing.clone(),
        }
    }
}
