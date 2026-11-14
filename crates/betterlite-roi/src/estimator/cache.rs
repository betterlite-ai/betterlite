use betterlite_models::model_capability_predicates::CapabilityPredicates;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::settings::ROISettings;

use super::json::{JsonError, json_dumps_text};
use super::models::{ESTIMATE_VERSION, EstimatorModel, RESPONSE_CONTRACT};
use super::options::{EstimatorOptions, configured_models, estimator_options};

#[derive(Serialize)]
struct CacheContext<'text>(
    &'text str,
    &'text str,
    &'text str,
    &'text str,
    &'text EstimatorOptions,
);

pub async fn cache_context<Predicates>(
    settings: &ROISettings,
    models: Option<&[EstimatorModel]>,
    predicates: &mut Predicates,
) -> Result<String, JsonError>
where
    Predicates: CapabilityPredicates + Send,
{
    let resolved = configured_models(settings, models);
    let options = estimator_options(predicates, &resolved).await;
    let context = CacheContext(
        ESTIMATE_VERSION,
        &settings.estimator_model,
        &settings.estimator_prompt,
        RESPONSE_CONTRACT,
        &options,
    );
    let text = json_dumps_text(&context)?;
    Ok(hex::encode(Sha256::digest(text.as_bytes())))
}
