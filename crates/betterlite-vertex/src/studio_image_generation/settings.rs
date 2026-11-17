use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

// GoogleImageGenConfig.DEFAULT_BASE_URL, which already carries the API version
pub const DEFAULT_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

// The two values this adapter reads itself: GEMINI_API_KEY alone, and GEMINI_API_BASE before the default.
#[derive(Clone, Debug)]
pub struct Settings {
    pub gemini_api_key: Option<SecretString>,
    pub api_base: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gemini_api_key: None,
            api_base: DEFAULT_API_BASE.to_owned(),
        }
    }
}

impl Settings {
    #[must_use]
    pub fn from_environment(lookup: &dyn EnvironmentLookup) -> Self {
        Self {
            gemini_api_key: lookup
                .lookup("GEMINI_API_KEY")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            api_base: lookup
                .lookup("GEMINI_API_BASE")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        }
    }
}
