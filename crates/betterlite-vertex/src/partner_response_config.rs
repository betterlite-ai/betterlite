pub mod error;
pub mod llama;

mod ai21;
mod claude;
mod tests;

use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::partner_openai::ai21::Ai21Config;
use crate::partner_openai::llama::Llama3Config;
use crate::vertex_relay_metadata::types::VertexPublisher;

use error::PartnerResponseError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartnerResponseConfig {
    Claude,
    Ai21(Ai21Config),
    Llama3(Llama3Config),
}

// get_vertex_ai_partner_model_config: the publisher picks the class, and LiteLLM builds each with no arguments.
#[must_use]
pub fn partner_response_config(publisher: VertexPublisher) -> PartnerResponseConfig {
    match publisher {
        VertexPublisher::Anthropic => PartnerResponseConfig::Claude,
        VertexPublisher::Ai21 => PartnerResponseConfig::Ai21(Ai21Config::default()),
        VertexPublisher::OpenApi | VertexPublisher::MistralAi => {
            PartnerResponseConfig::Llama3(Llama3Config::default())
        }
    }
}

// The arguments LiteLLM's transform_response receives: the reply, the caller's request pieces and the injected clock.
pub struct PartnerResponseInput<'call> {
    pub model: &'call str,
    pub starting_id: &'call str,
    pub status: u16,
    pub headers: &'call HeaderMap,
    pub body: &'call [u8],
    pub messages: &'call [Json],
    pub request_data: &'call IndexMap<String, Json>,
    pub optional_params: &'call IndexMap<String, Json>,
    pub litellm_params: &'call IndexMap<String, Json>,
    pub json_mode: Option<bool>,
    pub now: i64,
    pub identifier: fn() -> Uuid,
    pub disable_add_prefix_to_prompt: bool,
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'call>,
}

pub struct PartnerResponseCall<'call, Counter> {
    pub input: PartnerResponseInput<'call>,
    pub counter: &'call Counter,
}

impl PartnerResponseConfig {
    // Each arm runs the leaf LiteLLM picks and answers the ModelResponse that leaf returns, as its dump.
    pub fn transform_response<Counter>(
        &self,
        call: PartnerResponseCall<'_, Counter>,
    ) -> Result<Json, PartnerResponseError<Counter::Failure>>
    where
        Counter: ReasoningTokenCounter,
    {
        match self {
            Self::Claude => claude::transform_response(call),
            Self::Ai21(_) => ai21::transform_response(call),
            Self::Llama3(_) => llama::llama3_transform_response(call.input),
        }
    }
}
