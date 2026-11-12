#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibleChatAlias {
    Empower,
    Friendliai,
    Galadriel,
    Github,
}
// each class is an empty OpenAILikeChatConfig subclass, so B1425's compatible chat policy is all of its behavior
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
