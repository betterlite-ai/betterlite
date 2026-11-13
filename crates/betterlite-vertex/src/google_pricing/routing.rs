#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CostMetric {
    PerCharacter,
    PerToken,
}

const PARTNER_SUBSTRINGS: [&str; 6] = ["claude", "llama", "mistral", "jamba", "codestral", "gemma"];

#[must_use]
pub fn cost_router(model: &str, custom_llm_provider: &str, call_type: &str) -> CostMetric {
    if custom_llm_provider != "vertex_ai" {
        return CostMetric::PerCharacter;
    }
    let partner = PARTNER_SUBSTRINGS.iter().any(|name| model.contains(name));
    // every clause of the source's condition repeats the provider test, so it only ever routes a vertex_ai call
    if partner
        || call_type == "embedding"
        || call_type == "aembedding"
        || model.contains("gemini-2")
    {
        return CostMetric::PerToken;
    }
    CostMetric::PerCharacter
}
