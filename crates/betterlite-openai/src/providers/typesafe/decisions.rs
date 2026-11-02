use crate::decisions::transformation::provider_config::DecisionsProviderConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TypeSafeDecisionsConfig;

impl DecisionsProviderConfig for TypeSafeDecisionsConfig {
    const API_KEY_ENV: &'static [&'static str] = &["TYPESAFE_API_KEY"];
    const API_BASE_ENV: &'static [&'static str] = &["TYPESAFE_API_BASE"];

    fn get_default_api_base(&self) -> Option<String> {
        Some("https://api.typesafe.ai".to_owned())
    }
}

mod tests;
