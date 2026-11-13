use super::tags::ChatConfigTag;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureAiRoute {
    Agents,
    ModelRouter,
    Default,
}

#[must_use]
pub fn get_azure_ai_route(model: &str) -> AzureAiRoute {
    if model.contains("agents/") {
        return AzureAiRoute::Agents;
    }
    let lowered = model.to_lowercase();
    // a "model_router/" or "model-router/" hit already contains the substring checked here
    if lowered.contains("model_router") || lowered.contains("model-router") {
        return AzureAiRoute::ModelRouter;
    }
    AzureAiRoute::Default
}

#[must_use]
pub fn select_config(model: &str) -> ChatConfigTag {
    if get_azure_ai_route(model) == AzureAiRoute::ModelRouter {
        return ChatConfigTag::AzureModelRouter;
    }
    if model.to_lowercase().contains("claude") {
        return ChatConfigTag::AzureAnthropic;
    }
    ChatConfigTag::AzureAiStudio
}
