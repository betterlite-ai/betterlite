use betterlite_config::types::Config;
use betterlite_router::generalization_integration::error::GeneralizationError;
use betterlite_router::generalization_integration::model_generalizations::ModelGeneralizations;
use betterlite_router::router_core::services::RouteServices;

use crate::routing::model_generalizations::GeneralizationServices;

mod tests;

// The routing state one server serves: the fallback_generalizations rules its loaded config installed.
#[derive(Debug, Default)]
pub struct RoutingState {
    generalizations: GeneralizationServices,
}

impl RoutingState {
    // a rule document that cannot be installed is logged, and the server serves no rules instead of failing to start
    #[must_use]
    pub fn new(config: &Config) -> Self {
        match GeneralizationServices::build(&config.sections.model_generalizations) {
            Ok(generalizations) => Self { generalizations },
            Err(error) => {
                tracing::error!(%error, "the configured fallback_generalizations rules are unreadable");
                Self::default()
            }
        }
    }

    // A config reload that fails to install leaves the rules the server already serves untouched.
    pub fn reload(&mut self, config: &Config) -> Result<(), GeneralizationError> {
        self.generalizations
            .reload(&config.sections.model_generalizations)
    }
}

impl RouteServices for RoutingState {
    fn model_generalizations(&self) -> Option<&ModelGeneralizations> {
        self.generalizations.model_generalizations()
    }
}
