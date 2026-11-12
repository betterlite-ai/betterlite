pub const INFERENCE_API_BASE: &str = "https://inference.baseten.co/v1";
const MODEL_PREFIX: &str = "baseten/";
const DEDICATED_PREFIX: &str = "https://model-";
const DEDICATED_SUFFIX: &str = ".api.baseten.co/environments/production/sync/v1";
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
        return format!("{DEDICATED_PREFIX}{deployment_id}{DEDICATED_SUFFIX}");
    }
    INFERENCE_API_BASE.to_owned()
}
