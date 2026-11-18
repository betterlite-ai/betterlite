use betterlite_config::sections::tier_artifact_predictor as section;
use betterlite_router::tier_artifact_predictor::artifact::resolve_tier_artifact;
use betterlite_router::tier_artifact_predictor::error::PredictorError;
use betterlite_router::tier_artifact_predictor::predictor::TierArtifactPredictor;

mod tests;

// the artifact is loaded and validated once, before the deployment publishes any router state
pub fn predictor(configured: &section::Config) -> Result<TierArtifactPredictor, PredictorError> {
    TierArtifactPredictor::new(
        resolve_tier_artifact(&configured.artifact)?,
        configured.success_threshold,
    )
}

// index-aligned with model_list; None where the deployment configures no complexity router
pub fn deployment_predictors(
    settings: &section::Settings,
) -> Result<Vec<Option<TierArtifactPredictor>>, PredictorError> {
    settings
        .deployments
        .iter()
        .map(|deployment| deployment.as_ref().map(predictor).transpose())
        .collect()
}
