use indexmap::IndexMap;
use secrecy::SecretString;

pub struct BedrockModelInfo;

pub trait ModelInfo {
    #[must_use]
    fn get_api_base(api_base: Option<String>) -> Option<String> {
        api_base
    }

    #[must_use]
    fn get_api_key(api_key: Option<SecretString>) -> Option<SecretString> {
        api_key
    }

    #[must_use]
    fn validate_environment(headers: IndexMap<String, String>) -> IndexMap<String, String> {
        headers
    }

    #[must_use]
    fn get_models() -> Vec<String> {
        Vec::new()
    }
}

impl ModelInfo for BedrockModelInfo {}
