use betterlite_config::sections::tier_artifact_predictor as section;
use betterlite_router::tier_artifact_predictor::artifact::resolve_tier_artifact;
use betterlite_router::tier_artifact_predictor::error::PredictorError;
use betterlite_router::tier_artifact_predictor::predictor::{
    TierArtifactPredictor, TierSuccessPredictor,
};

mod tests;

pub fn predictor(configured: &section::Config) -> Result<TierArtifactPredictor, PredictorError> {
    let artifact = resolve_tier_artifact(&configured.artifact)?;
    let predictor = TierSuccessPredictor::new(artifact, configured.success_threshold)?;
    TierArtifactPredictor::new(predictor)
}

pub fn deployment_predictors(
    settings: &section::Settings,
) -> Result<Vec<Option<TierArtifactPredictor>>, PredictorError> {
    settings
        .deployments
        .iter()
        .map(|deployment| deployment.as_ref().map(predictor).transpose())
        .collect()
}
