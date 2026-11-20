pub mod error;
pub mod params;
pub mod reply;
pub mod upload;

mod tests;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::api::image_contracts::records::ImageHeaders;
use secrecy::{ExposeSecret as _, SecretString};

use self::error::TopazError;

pub const DEFAULT_API_BASE: &str = "https://api.topazlabs.com";

const API_KEY_VARIABLE: &str = "TOPAZ_API_KEY";
const API_BASE_VARIABLE: &str = "TOPAZ_API_BASE";

// TopazModelInfo.get_models: the five pinned display names, each behind the provider prefix
pub const MODELS: [&str; 5] = [
    "topaz/Standard V2",
    "topaz/Low Resolution V2",
    "topaz/CGI",
    "topaz/High Resolution V2",
    "topaz/Text Refine",
];

// TopazModelInfo's credential settings; get_api_base falls back to the default host after TOPAZ_API_BASE
#[derive(Debug, Clone)]
pub struct TopazSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl TopazSettings {
    // get_secret_str reads an absent or empty variable as unset, so the loader does the same
    #[must_use]
    pub fn from_environment(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present(API_KEY_VARIABLE).map(SecretString::from),
            api_base: present(API_BASE_VARIABLE).unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        }
    }
}

// TopazModelInfo.get_api_key takes a truthy explicit key before TOPAZ_API_KEY
#[must_use]
pub fn get_api_key(
    explicit: Option<&SecretString>,
    settings: &TopazSettings,
) -> Option<SecretString> {
    explicit
        .filter(|key| !key.expose_secret().is_empty())
        .cloned()
        .or_else(|| settings.api_key.clone())
}

// TopazModelInfo.get_api_base prefers a truthy explicit base, then TOPAZ_API_BASE, then the default host
#[must_use]
pub fn get_api_base(explicit: Option<&str>, settings: &TopazSettings) -> String {
    explicit
        .filter(|base| !base.is_empty())
        .unwrap_or(settings.api_base.as_str())
        .to_owned()
}

// TopazModelInfo.validate_environment rejects only a missing key and answers a fresh map, so no caller header survives
pub fn validate_environment(api_key: Option<&SecretString>) -> Result<ImageHeaders, TopazError> {
    let key = api_key.ok_or(TopazError::MissingApiKey)?;
    let mut headers = ImageHeaders::new();
    headers.insert("Accept".to_owned(), "image/jpeg".to_owned());
    headers.insert("X-API-Key".to_owned(), key.expose_secret().to_owned());
    Ok(headers)
}
