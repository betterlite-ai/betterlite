pub mod response_schema;

use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VertexCapabilityProvider {
    VertexAi,
    VertexAiBeta,
    Gemini,
}

impl VertexCapabilityProvider {
    // the model catalog has no vertex_ai_beta entries, so a lookup asks for vertex_ai instead
    #[must_use]
    pub const fn lookup_name(self) -> &'static str {
        match self {
            Self::VertexAi | Self::VertexAiBeta => "vertex_ai",
            Self::Gemini => "gemini",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityKind {
    SystemMessages,
    ResponseSchema,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct CapabilityLookupError {
    pub message: String,
}

pub trait ModelCapabilityLookup {
    fn supports(
        &self,
        kind: CapabilityKind,
        model: &str,
        provider: &str,
    ) -> Result<bool, CapabilityLookupError>;
}

#[must_use]
pub fn get_supports_system_message(
    model: &str,
    provider: VertexCapabilityProvider,
    lookup: &dyn ModelCapabilityLookup,
) -> bool {
    let provider_name = provider.lookup_name();
    lookup
        .supports(CapabilityKind::SystemMessages, model, provider_name)
        .map_or_else(
            |error| {
                tracing::warn!(
                    "Unable to identify if system message supported. Defaulting to 'False'. Received error message - {error}\nAdd it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json"
                );
                false
            },
            |supported| supported || model.contains("gemini/"),
        )
}
