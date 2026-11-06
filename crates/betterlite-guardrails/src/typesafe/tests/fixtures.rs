#![cfg(test)]

use betterlite_prompts::anthropic_history::groups::group_tool_exchanges;
use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;

use crate::typesafe::typesafe_config::TypeSafeSettings;
use crate::typesafe::typesafe_protected_indices::compression_protected_indices;
use crate::typesafe::{DEFAULT_JEV_TIMEOUT, TypeSafeGuardrail};

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture json")
}

// Mirrors the guardrail framework's content_to_text: a string stays as it is and text parts join with blank lines.
pub(crate) fn content_to_text(content: Option<&Json>) -> String {
    match content {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::List(parts)) => parts
            .iter()
            .filter_map(Json::as_object)
            .filter(|fields| fields.get("type").and_then(Json::as_str) == Some("text"))
            .filter_map(|fields| fields.get("text").and_then(Json::as_str))
            .collect::<Vec<&str>>()
            .join("\n\n"),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) | None => {
            String::new()
        }
    }
}

pub(crate) fn settings() -> TypeSafeSettings {
    TypeSafeSettings::default()
}

pub(crate) fn guardrail(settings: TypeSafeSettings) -> TypeSafeGuardrail {
    TypeSafeGuardrail {
        settings,
        api_key: SecretString::from("secret-key"),
        client: reqwest::Client::default(),
        timeout: DEFAULT_JEV_TIMEOUT,
        content_to_text,
        get_protected_indices: compression_protected_indices,
        group_tool_exchanges,
    }
}
