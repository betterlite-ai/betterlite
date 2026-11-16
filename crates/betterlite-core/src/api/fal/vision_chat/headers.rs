use betterlite_openai::api::image_contracts::records::ImageHeaders;
use secrecy::SecretString;

use crate::api::fal::images::settings::FalImageSettings;
use crate::api::fal::vision_chat::error::FalAIError;

pub const PROVIDER_PREFIX: &str = "fal_ai/";

// the caller's headers sit over the JSON default and the Fal key goes in last, so it always wins
pub fn validate_environment(
    settings: &FalImageSettings,
    headers: &ImageHeaders,
    api_key: Option<&SecretString>,
) -> Result<ImageHeaders, FalAIError> {
    let mut merged =
        ImageHeaders::from([("content-type".to_owned(), "application/json".to_owned())]);
    merged.extend(headers.clone());
    settings
        .authorize(&merged, api_key)
        .map_err(FalAIError::MissingApiKey)
}

// the endpoint is the model minus one leading fal_ai/, and an empty one still leaves the slash
#[must_use]
pub fn complete_url(settings: &FalImageSettings, api_base: Option<&str>, model: &str) -> String {
    let endpoint = model.strip_prefix(PROVIDER_PREFIX).unwrap_or(model);
    format!("{}/{endpoint}", settings.complete_url(api_base, ""))
}
