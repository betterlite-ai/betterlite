use betterlite_config::sections::openai_like_settings::Settings as OpenAiLikeSettings;
use secrecy::SecretString;

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

    // the alias adds no identity hook, so B1425's OpenAI-like settings resolve the credentials unchanged
    #[must_use]
    pub fn provider_info(
        self,
        settings: &OpenAiLikeSettings,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> (&'static str, Option<String>, SecretString) {
        let (base, key) = settings.resolve(api_base, api_key);
        (self.provider(), base, key)
    }
}
