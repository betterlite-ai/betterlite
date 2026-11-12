use secrecy::SecretString;

use crate::loader::EnvironmentLookup;

pub(crate) struct EnvironmentValues {
    pub(crate) featherless_ai_api_base: Option<String>,
    pub(crate) featherless_api_base: Option<String>,
    pub(crate) featherless_ai_api_key: Option<SecretString>,
    pub(crate) featherless_api_key: Option<SecretString>,
    pub(crate) aiml_api_base: Option<String>,
    pub(crate) aiml_api_key: Option<SecretString>,
}

// LiteLLM reads each alias with `or`, so an empty environment value falls through like an absent one
pub(crate) fn resolve(lookup: &dyn EnvironmentLookup) -> EnvironmentValues {
    EnvironmentValues {
        featherless_ai_api_base: value(lookup, "FEATHERLESS_AI_API_BASE"),
        featherless_api_base: value(lookup, "FEATHERLESS_API_BASE"),
        featherless_ai_api_key: value(lookup, "FEATHERLESS_AI_API_KEY").map(SecretString::from),
        featherless_api_key: value(lookup, "FEATHERLESS_API_KEY").map(SecretString::from),
        aiml_api_base: value(lookup, "AIML_API_BASE"),
        aiml_api_key: value(lookup, "AIML_API_KEY").map(SecretString::from),
    }
}

fn value(lookup: &dyn EnvironmentLookup, name: &str) -> Option<String> {
    match lookup.lookup(name) {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}
