use betterlite_config::sections::model_generalizations as config;
use betterlite_router::generalization_integration::error::GeneralizationError;
use betterlite_router::generalization_integration::model_generalizations::ModelGeneralizations;
use betterlite_router::router_core::services::RouteServices;

mod tests;

// The server's copy of the fallback_generalizations rules, which a route reads through RouteServices.
#[derive(Debug)]
pub struct GeneralizationServices {
    installed: ModelGeneralizations,
}

impl GeneralizationServices {
    pub fn build(section: &config::Settings) -> Result<Self, GeneralizationError> {
        let rules = serde_json::to_vec(&section.rules)?;
        let installed = ModelGeneralizations::install(&rules)?;
        for warning in &installed.warnings {
            tracing::warn!(
                kind = warning.kind.label(),
                pattern = %warning.pattern,
                "generalization rule skipped by its configured bound"
            );
        }
        for warning in &installed.engine_warnings {
            tracing::warn!(warning = %warning, "generalization rule skipped");
        }
        Ok(Self { installed })
    }

    // A section that fails to install leaves the rules the router already serves untouched.
    pub fn reload(&mut self, section: &config::Settings) -> Result<(), GeneralizationError> {
        *self = Self::build(section)?;
        Ok(())
    }
}

impl RouteServices for GeneralizationServices {
    fn model_generalizations(&self) -> Option<&ModelGeneralizations> {
        Some(&self.installed)
    }
}
