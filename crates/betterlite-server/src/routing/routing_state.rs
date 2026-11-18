use std::sync::Arc;

use betterlite_config::types::Config;
use betterlite_models::catalog::BundledCatalog;
use betterlite_router::generalization_integration::error::GeneralizationError;
use betterlite_router::generalization_integration::model_generalizations::ModelGeneralizations;
use betterlite_router::least_busy::LeastBusy;
use betterlite_router::router_core::services::RouteServices;

use crate::routing::model_generalizations::GeneralizationServices;

mod tests;

// The routing state one server serves: the generalization rules its config installed, and the in-flight counters
#[derive(Default)]
pub struct RoutingState {
    generalizations: GeneralizationServices,
    least_busy: Option<Arc<LeastBusy>>,
}

impl RoutingState {
    // a rule document that cannot be installed is logged, and the server serves no rules instead of failing to start
    #[must_use]
    pub fn new(config: &Config) -> Self {
        let mut state = Self::default();
        if let Err(error) = state.reload(config) {
            tracing::error!(%error, "the configured fallback_generalizations rules are unreadable");
        }
        state
    }

    // A failed reload leaves the served rules untouched, and a config naming none takes the loaded cost map's own.
    pub fn reload(&mut self, config: &Config) -> Result<(), GeneralizationError> {
        self.least_busy = crate::routing::least_busy::build(config);
        let section = &config.sections.model_generalizations;
        if section.rules.is_empty() {
            let cost_map = BundledCatalog::default().get();
            return self
                .generalizations
                .reload_cost_map(&cost_map.generalizations);
        }
        self.generalizations.reload(section)
    }
}

impl RouteServices for RoutingState {
    fn model_generalizations(&self) -> Option<&ModelGeneralizations> {
        self.generalizations.model_generalizations()
    }

    fn least_busy(&self) -> Option<Arc<LeastBusy>> {
        self.least_busy.clone()
    }
}
