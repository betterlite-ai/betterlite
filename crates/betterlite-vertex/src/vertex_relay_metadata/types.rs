use betterlite_models::cost_usage::CostUsage;
use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomLlmProvider {
    VertexAi,
    Gemini,
}

impl CustomLlmProvider {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VertexAi => "vertex_ai",
            Self::Gemini => "gemini",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexPublisher {
    MistralAi,
    Anthropic,
    Ai21,
    OpenApi,
}

impl VertexPublisher {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MistralAi => "mistralai",
            Self::Anthropic => "anthropic",
            Self::Ai21 => "ai21",
            Self::OpenApi => "openapi",
        }
    }
}

// The fields of LiteLLM's response object the passthrough logging payload reads or writes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletionResponse {
    pub id: Option<String>,
    pub model: Option<String>,
    pub usage: Option<CostUsage>,
}

// The model_call_details keys the passthrough payloads write; the Vertex factory leaves response_cost absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelCallDetails {
    pub model: Option<String>,
    pub custom_llm_provider: Option<CustomLlmProvider>,
    pub response_cost: Option<Decimal>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoggingObject {
    pub litellm_call_id: String,
    pub model: Option<String>,
    pub custom_llm_provider: Option<CustomLlmProvider>,
    pub model_call_details: ModelCallDetails,
}

// The request-context keys a passthrough handler adds before the success dispatch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponseLoggingKwargs {
    pub response_cost: Option<Decimal>,
    pub model: Option<String>,
    pub custom_llm_provider: Option<CustomLlmProvider>,
}

pub struct CompletionCostRequest<'input> {
    pub response: &'input CompletionResponse,
    pub model: &'input str,
    pub custom_llm_provider: CustomLlmProvider,
    pub vertex_location: Option<&'input str>,
}

// Betterlite-vertex is a leaf, so the cost service enters through this trait instead of betterlite-core.
pub trait CompletionCost {
    fn completion_cost(&self, request: &CompletionCostRequest<'_>) -> Option<Decimal>;
}
