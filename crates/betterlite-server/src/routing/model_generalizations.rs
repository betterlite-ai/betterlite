use betterlite_config::sections::model_generalizations as config;
use betterlite_router::generalization_integration::error::GeneralizationError;
use betterlite_router::generalization_integration::model_generalizations::ModelGeneralizations;

mod tests;

pub fn service(section: &config::Settings) -> Result<ModelGeneralizations, GeneralizationError> {
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
    Ok(installed)
}
