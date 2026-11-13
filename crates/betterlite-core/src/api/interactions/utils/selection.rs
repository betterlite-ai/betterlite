#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionsProvider {
    GoogleAiStudio,
    VertexAi,
}

// LiteLLM compares against LlmProviders' values, so the match is case-sensitive and exact
#[must_use]
pub fn select_interactions_provider(provider: Option<&str>) -> Option<InteractionsProvider> {
    match provider? {
        "gemini" => Some(InteractionsProvider::GoogleAiStudio),
        "vertex_ai" | "vertex_ai_beta" => Some(InteractionsProvider::VertexAi),
        _ => None,
    }
}
