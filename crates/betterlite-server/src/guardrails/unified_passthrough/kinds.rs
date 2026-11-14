use betterlite_core::routes::CallType;
use betterlite_guardrails::llm_passthrough_dispatch::PassthroughTranslationKind;

use super::bedrock::BedrockPassthroughTranslation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassthroughKind {
    Endpoint,
    LlmRoute,
}

// the two pass-through rows of LiteLLM's guardrail_translation_mappings, mirrored from B3574's kinds
pub const PASSTHROUGH_TABLE: [(CallType, PassthroughKind); 2] = [
    (CallType::PassThroughEndpoint, PassthroughKind::Endpoint),
    (CallType::AllmPassthroughRoute, PassthroughKind::LlmRoute),
];

#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn passthrough_provider_list<G, H>(
    bedrock: BedrockPassthroughTranslation<G, H>,
) -> Vec<(String, BedrockPassthroughTranslation<G, H>)> {
    vec![("bedrock".to_owned(), bedrock)]
}

mod tests;
