use std::convert::Infallible;

use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::usage::ReasoningTokenCounter as AnthropicReasoningCounter;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_tokenizer::count::request::Message;
use indexmap::IndexMap;

use crate::agentcore_stream::IdSource;
use crate::aws_endpoints::provider::{BedrockInvokeProvider, invoke_provider};
use crate::converse_response::request::OptionalParams;
use crate::converse_response::wire::ModelResponse;
use crate::converse_usage::transform::ReasoningTokenCounter as ConverseReasoningCounter;
use crate::image_call::plan::LogContext;
use crate::pegasus::counter::TokenCounter;
use crate::pegasus::response::PegasusResponse;

mod body;
mod delegate;
pub mod error;
mod usage;

mod tests;

pub struct RawInvokeReply<'reply> {
    pub status: u16,
    pub headers: &'reply [(String, String)],
    pub body: &'reply [u8],
}

pub struct InvokeReplyCall<'call, 'log, 'env> {
    pub model: &'call str,
    pub messages: &'call Json,
    pub counted_messages: &'call [Message],
    pub request_data: &'call IndexMap<String, Json>,
    pub optional_params: &'call IndexMap<String, Json>,
    pub optional: &'call OptionalParams,
    pub litellm_params: &'call IndexMap<String, Json>,
    pub tool_name_mappings: &'call IndexMap<String, String>,
    pub json_mode: Option<bool>,
    pub disable_add_prefix_to_prompt: bool,
    pub now: f64,
    pub ids: &'call dyn IdSource,
    pub log: LogContext<'log, 'env>,
}

// The provider families keep their own reply wires; LiteLLM returns the same ModelResponse it was handed.
pub enum InvokeReply {
    Chat(ChatResponse),
    Anthropic(ResponseEnvelope),
    Nova(ModelResponse),
    Pegasus(PegasusResponse),
}

enum Route {
    Anthropic,
    Nova,
    Pegasus,
    Family(body::Family),
}

const fn route(provider: Option<BedrockInvokeProvider>) -> Route {
    match provider {
        Some(BedrockInvokeProvider::Anthropic) => Route::Anthropic,
        Some(BedrockInvokeProvider::Nova) => Route::Nova,
        Some(BedrockInvokeProvider::Twelvelabs) => Route::Pegasus,
        Some(BedrockInvokeProvider::Cohere) => Route::Family(body::Family::Cohere),
        Some(BedrockInvokeProvider::Ai21) => Route::Family(body::Family::Ai21),
        Some(
            BedrockInvokeProvider::Meta
            | BedrockInvokeProvider::Llama
            | BedrockInvokeProvider::DeepseekR1,
        ) => Route::Family(body::Family::Generation),
        Some(BedrockInvokeProvider::Mistral) => Route::Family(body::Family::Mistral),
        Some(
            BedrockInvokeProvider::Amazon
            | BedrockInvokeProvider::Qwen3
            | BedrockInvokeProvider::Qwen2
            | BedrockInvokeProvider::Openai
            | BedrockInvokeProvider::Stability
            | BedrockInvokeProvider::Moonshot,
        )
        | None => Route::Family(body::Family::Titan),
    }
}

pub async fn decode_invoke_reply<ChatCounter, Reasoning>(
    call: InvokeReplyCall<'_, '_, '_>,
    raw: &RawInvokeReply<'_>,
    mut response: ChatResponse,
    counter: &ChatCounter,
    reasoning: &Reasoning,
) -> Result<InvokeReply, error::InvokeReplyError>
where
    ChatCounter: TokenCounter + Sync,
    Reasoning: AnthropicReasoningCounter<Failure = Infallible>
        + ConverseReasoningCounter<Failure = Infallible>,
{
    let record = body::parse(raw.body).map_err(|_source| error::decode_error(raw))?;
    body::debug_log(&record);
    let family = match route(invoke_provider(call.model)) {
        Route::Anthropic => {
            return delegate::anthropic(call, raw, response, reasoning).map(InvokeReply::Anthropic);
        }
        Route::Nova => {
            return delegate::nova(call, raw, response, reasoning).map(InvokeReply::Nova);
        }
        Route::Pegasus => {
            return delegate::pegasus(call, raw, response, counter)
                .await
                .map(InvokeReply::Pegasus);
        }
        Route::Family(family) => family,
    };
    let output = body::family_output(&record, family, &mut response.choices)
        .map_err(|source| error::processing_error(raw, &source))?;
    body::assign_content(&mut response.choices, &output)
        .map_err(|reason| error::content_error(raw, &body::python_text(&output), reason))?;
    usage::assign(&call, raw, &mut response, counter).await?;
    Ok(InvokeReply::Chat(response))
}
