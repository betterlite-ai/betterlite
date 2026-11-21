use betterlite_context::chat_config_selection::foundry::{AzureAiRoute, get_azure_ai_route};
use thiserror::Error;

pub const AGENTS_BASE_MESSAGE: &str = "Azure AI Agents requests require an api_base. Set `api_base` or the AZURE_AI_API_BASE env var.";
pub const CLAUDE_BASE_MESSAGE: &str = "Azure Anthropic requests require an api_base. Set `api_base` or the AZURE_AI_API_BASE env var.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Agents,
    Claude,
    Chat,
}

// main.py tests the agents route first, so a model naming both an agent and claude reaches the agents leaf
#[must_use]
pub fn select(model: &str) -> Route {
    if get_azure_ai_route(model) == AzureAiRoute::Agents {
        return Route::Agents;
    }
    if model.to_lowercase().contains("claude") {
        return Route::Claude;
    }
    Route::Chat
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MissingBase {
    #[error("{}", AGENTS_BASE_MESSAGE)]
    Agents,
    #[error("{}", CLAUDE_BASE_MESSAGE)]
    Claude,
}
