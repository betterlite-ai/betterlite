use super::tags::ChatConfigTag;

#[derive(Debug, Clone, Copy, Default)]
pub struct VertexModelLists<'source> {
    pub mistral_models: &'source [String],
    pub ai21_models: &'source [String],
}

#[must_use]
pub fn select_config(model: &str, lists: VertexModelLists<'_>) -> ChatConfigTag {
    if model.contains("gemini") {
        return ChatConfigTag::VertexGemini;
    }
    if model.contains("claude") {
        return ChatConfigTag::VertexAiAnthropic;
    }
    if model.contains("gpt-oss") {
        return ChatConfigTag::VertexAiGptOss;
    }
    if lists
        .mistral_models
        .iter()
        .any(|entry| entry.as_str() == model)
    {
        return if model.contains("codestral") {
            ChatConfigTag::CodestralTextCompletion
        } else {
            ChatConfigTag::VertexAiMistral
        };
    }
    if lists
        .ai21_models
        .iter()
        .any(|entry| entry.as_str() == model)
    {
        return ChatConfigTag::VertexAiAi21;
    }
    ChatConfigTag::VertexAiLlama3
}
