use betterlite_config::sections::studio_identity::Settings;

use crate::studio_identity::API_VERSION;

// get_complete_url: only an absent base falls back to GEMINI_API_BASE, and an empty base stays empty
#[must_use]
pub fn complete_url(model: &str, api_base: Option<&str>, settings: &Settings) -> String {
    let base = api_base.unwrap_or(settings.api_base.as_str());
    let trimmed = base.trim_end_matches('/');
    if model.is_empty() {
        return trimmed.to_owned();
    }
    let model_name = model.replace("gemini/", "");
    format!("{trimmed}/{API_VERSION}/models/{model_name}:predictLongRunning")
}
