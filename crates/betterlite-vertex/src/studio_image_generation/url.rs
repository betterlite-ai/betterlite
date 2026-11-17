use super::settings::Settings;
use crate::gemini_image_policy::policy::is_gemini_image_model;

// get_complete_url: only an absent base falls back to GEMINI_API_BASE, and a trailing slash never doubles
#[must_use]
pub fn complete_url(model: &str, api_base: Option<&str>, settings: &Settings) -> String {
    let base = api_base.unwrap_or(settings.api_base.as_str());
    let trimmed = base.trim_end_matches('/');
    if is_gemini_image_model(model) {
        format!("{trimmed}/models/{model}:generateContent")
    } else {
        format!("{trimmed}/models/{model}:predict")
    }
}
