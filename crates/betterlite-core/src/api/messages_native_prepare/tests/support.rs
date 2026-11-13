use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::SecretString;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("json value")
}

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("json object")
}

pub(super) fn headers(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn settings(api_base: &str) -> AnthropicSettings {
    AnthropicSettings {
        api_key: None,
        auth_token: None,
        api_base: api_base.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}
