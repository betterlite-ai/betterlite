#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplicitRoute {
    ClaudePlatform,
    Converse,
    Mantle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BedrockMessagesConfigTag {
    ClaudePlatform,
    Mantle,
    AnthropicClaude,
}

impl BedrockMessagesConfigTag {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::ClaudePlatform => "BedrockClaudePlatformMessagesConfig",
            Self::Mantle => "AmazonMantleMessagesConfig",
            Self::AnthropicClaude => "AmazonAnthropicClaudeMessagesConfig",
        }
    }
}

// B598 computes the matching explicit routes; the source tests each prefix independently, not the merged route
#[must_use]
pub fn select_bedrock_messages_config(
    model: &str,
    explicit: &[ExplicitRoute],
) -> Option<BedrockMessagesConfigTag> {
    if explicit.contains(&ExplicitRoute::ClaudePlatform) {
        return Some(BedrockMessagesConfigTag::ClaudePlatform);
    }
    if explicit.contains(&ExplicitRoute::Converse) {
        return None;
    }
    if explicit.contains(&ExplicitRoute::Mantle) {
        return Some(BedrockMessagesConfigTag::Mantle);
    }
    model
        .contains("claude")
        .then_some(BedrockMessagesConfigTag::AnthropicClaude)
}
