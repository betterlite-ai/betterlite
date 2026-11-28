use betterlite_auth::jwt_team_selection::settings::SelectionSettings;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// the typed litellm_jwtauth values the JWT team selection stage reads, parsed once with the section
#[derive(Clone, Debug)]
pub struct Slot {
    pub settings: SelectionSettings,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        let settings = &config.sections.jwt_settings;
        Self {
            settings: SelectionSettings {
                teams: settings.teams.clone(),
                routes: settings.routes.clone(),
                scopes: settings.scopes.clone(),
                routing: settings.routing.clone(),
            },
        }
    }
}
