pub mod aiml;
pub mod baseten;
pub mod cerebras;
pub mod error;
pub mod featherless;
mod tests;
use betterlite_config::sections::openai_like_settings::Settings as OpenAiLikeSettings;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
// each class is an empty OpenAILikeChatConfig subclass, so B1425's compatible chat policy is all of its behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibleChatAlias {
    Empower,
    Friendliai,
    Galadriel,
    Github,
}
impl CompatibleChatAlias {
    #[must_use]
    pub fn identity(
        self,
        settings: &OpenAiLikeSettings,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> (Option<String>, SecretString) {
        match self {
            Self::Empower | Self::Friendliai | Self::Galadriel | Self::Github => {
                settings.resolve(api_base, api_key)
            }
        }
    }
    #[must_use]
    pub const fn provider(self) -> &'static str {
        match self {
            Self::Empower => "empower",
            Self::Friendliai => "friendliai",
            Self::Galadriel => "galadriel",
            Self::Github => "github",
        }
    }
}
// Baseten and Cerebras walk the caller's order, rename the token limit even when null, and copy supported values as given
pub fn map_renaming_token_limit(
    non_default: &IndexMap<String, Value>,
    existing: &mut IndexMap<String, Value>,
    supported: &[&str],
) {
    for (name, value) in non_default {
        match name.as_str() {
            "max_completion_tokens" => {
                existing.insert(String::from("max_tokens"), value.clone());
            }
            _ if supported.contains(&name.as_str()) => {
                existing.insert(name.clone(), value.clone());
            }
            _ => {}
        }
    }
}
// LiteLLM chains the explicit value, each environment alias, then the default, skipping an empty one at every step
fn resolved_identity(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    environment_base: Option<&str>,
    environment_key: Option<&SecretString>,
    default_base: &str,
) -> (String, Option<SecretString>) {
    let base = api_base
        .filter(|value| !value.is_empty())
        .or(environment_base)
        .unwrap_or(default_base)
        .to_owned();
    let key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .or(environment_key)
        .cloned();
    (base, key)
}
