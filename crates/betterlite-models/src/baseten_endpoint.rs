pub const INFERENCE_API_BASE: &str = "https://inference.baseten.co/v1";
const MODEL_PREFIX: &str = "baseten/";
mod tests;
// LiteLLM strips every baseten/ occurrence, then matches exactly eight ASCII alphanumerics
#[must_use]
pub fn is_dedicated_deployment(model: &str) -> bool {
    let deployment_id = model.replace(MODEL_PREFIX, "");
    deployment_id.len() == 8
        && deployment_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
}
#[must_use]
pub fn get_api_base_for_model(model: &str) -> String {
    if is_dedicated_deployment(model) {
        let deployment_id = model.replace(MODEL_PREFIX, "");
        return format!(
            "https://model-{deployment_id}.api.baseten.co/environments/production/sync/v1"
        );
    }
    INFERENCE_API_BASE.to_owned()
}
