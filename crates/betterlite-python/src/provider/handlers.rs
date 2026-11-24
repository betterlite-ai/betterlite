use betterlite_models::identity::anthropic_text::anthropic_text_provider;
use betterlite_models::identity::cohere::cohere_chat_provider;

use crate::provider::convert::HandlerRequest;
use crate::provider::snapshot::ProviderSnapshot;

// handle_cohere_chat_model_custom_llm_provider: a cohere chat model answers on the cohere_chat route.
pub(crate) fn handle_cohere(
    snapshot: &ProviderSnapshot,
    request: &HandlerRequest,
) -> (String, Option<String>) {
    let context = snapshot.context();
    cohere_chat_provider(
        request.model.as_deref().unwrap_or_default(),
        request.custom_llm_provider.as_deref(),
        context.catalog,
    )
}

// handle_anthropic_text_model_custom_llm_provider: claude-2 and claude-instant-1 are text completions.
pub(crate) fn handle_anthropic(request: &HandlerRequest) -> (String, Option<String>) {
    anthropic_text_provider(
        request.model.as_deref().unwrap_or_default(),
        request.custom_llm_provider.as_deref(),
    )
}
