use betterlite_models::model_capability_predicates::CapabilityPredicates;
use sha2::{Digest, Sha256};

use crate::settings::ROISettings;

use super::json::{JsonError, json_dumps_text};
use super::models::{ESTIMATE_VERSION, EstimatorModel, RESPONSE_CONTRACT};
use super::options::{configured_models, estimator_options};

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
    let context = serde_json::json!([
        ESTIMATE_VERSION,
        &settings.estimator_model,
        &settings.estimator_prompt,
        RESPONSE_CONTRACT,
        options,
    ]);
    let text = json_dumps_text(&context)?;
    Ok(hex::encode(Sha256::digest(text.as_bytes())))
}
