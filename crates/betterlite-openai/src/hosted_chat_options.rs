pub mod baseten;
pub mod cerebras;
pub mod featherless;
mod tests;
use indexmap::IndexMap;
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
