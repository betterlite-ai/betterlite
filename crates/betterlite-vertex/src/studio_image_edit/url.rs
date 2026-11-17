use crate::studio_image_generation::settings::Settings;

// get_complete_url: every edit model answers on generateContent, and a trailing slash never doubles
#[must_use]
pub fn complete_url(model: &str, api_base: Option<&str>, settings: &Settings) -> String {
    let base = api_base.unwrap_or(settings.api_base.as_str());
    let trimmed = base.trim_end_matches('/');
    format!("{trimmed}/models/{model}:generateContent")
}
