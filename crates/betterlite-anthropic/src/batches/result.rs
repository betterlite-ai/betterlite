use std::collections::BTreeMap;
use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{
    CompletionTokensDetails as SummedCompletionDetails, PromptTokensDetails as SummedPromptDetails,
    Usage as SummedUsage, UsageSumError, usage_sum,
};
use http::HeaderMap;
use indexmap::IndexMap;
use thiserror::Error;

use crate::response::error::ResponseError;
use crate::response::wire::ResponseEnvelope;
use crate::response::{Logging, RawResponse, ResponseRequest, transform_response};
use crate::usage::{
    AnthropicUsage, CompletionTokensDetails as AnthropicCompletionDetails,
    PromptTokensDetails as AnthropicPromptDetails, ReasoningTokenCounter,
};

use super::decode::{BatchBody, DecodeError, DecodedBody, decode_anthropic_batch_result};

#[derive(Clone, Copy)]
pub struct BatchResponse<'call> {
    pub status: u16,
    pub headers: &'call HeaderMap,
    pub body: &'call [u8],
    pub now: i64,
}

#[derive(Debug, Error)]
pub enum ResultError<CounterFailure>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Projection(#[from] ResponseError<CounterFailure>),
    #[error("the batch result message could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the batch result usage could not be combined")]
    Usage {
        #[from]
        source: UsageSumError,
    },
    #[error("the batch body is not a result body")]
    NotResults,
}

// the response, request and counter every result row shares; the envelope and its logging arrive per row
struct RowProjector<'call, Counter> {
    status: u16,
    headers: &'call HeaderMap,
    request: ResponseRequest<'call>,
    counter: &'call Counter,
}

impl<Counter> RowProjector<'_, Counter>
where
    Counter: ReasoningTokenCounter,
{
    // one row is encoded, projected onto the shared envelope, and its usage kept for the sum
    fn project(
        &self,
        row: &Json,
        envelope: &mut ResponseEnvelope,
        logging: Logging<'_>,
    ) -> Result<Option<AnthropicUsage>, ResultError<Counter::Failure>> {
        let body = serde_json::to_vec(row).map_err(|source| ResultError::Encode { source })?;
        let raw = RawResponse {
            status: self.status,
            headers: self.headers,
            body: &body,
        };
        transform_response(envelope, &raw, &self.request, logging, self.counter)?;
        Ok(envelope.usage.take())
    }
}

// LiteLLM projects every row onto the same response, so the last row wins and each row's usage is added up
pub fn project_batch_results<Counter>(
    envelope: &mut ResponseEnvelope,
    response: BatchResponse<'_>,
    logging: Logging<'_>,
    counter: &Counter,
) -> Result<(), ResultError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    // the caller's ModelResponse() stamps its construction time before any row is projected
    envelope.created = response.now;
    let messages = match decode_anthropic_batch_result(BatchBody::Result, response.body)? {
        DecodedBody::Results(messages) => messages,
        DecodedBody::Lifecycle(_) => return Err(ResultError::NotResults),
    };
    let no_messages: [Json; 0] = [];
    let optional_params = IndexMap::new();
    let litellm_params = IndexMap::new();
    let request_data = IndexMap::new();
    let projector = RowProjector {
        status: response.status,
        headers: response.headers,
        request: ResponseRequest {
            messages: &no_messages,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            request_data: &request_data,
            json_mode: None,
            now: response.now,
            disable_add_prefix_to_prompt: false,
        },
        counter,
    };
    let Logging { state, env } = logging;
    let mut usages = Vec::new();
    for message in messages {
        usages.extend(projector.project(
            &message,
            envelope,
            Logging {
                state: &mut *state,
                env,
            },
        )?);
    }
    envelope.usage = Some(combine(&usages)?);
    Ok(())
}

fn combine(usages: &[AnthropicUsage]) -> Result<AnthropicUsage, UsageSumError> {
    let projected: Vec<SummedUsage> = usages.iter().map(project_usage).collect();
    let summed = usage_sum(&projected)?;
    let prompt = summed.prompt_tokens_details.clone().unwrap_or_default();
    let completion = summed.completion_tokens_details.clone().unwrap_or_default();
    Ok(AnthropicUsage {
        prompt_tokens: summed.prompt_tokens,
        completion_tokens: summed.completion_tokens,
        total_tokens: summed.total_tokens,
        prompt_tokens_details: AnthropicPromptDetails {
            text: prompt.text_tokens.unwrap_or(0),
            cached: prompt.cached_tokens.unwrap_or(0),
            cache_creation: prompt.cache_creation_tokens.unwrap_or(0),
            cache_write: prompt.cache_write_tokens.unwrap_or(0),
            audio: Some(prompt.audio_tokens.unwrap_or(0)),
            image: Some(prompt.image_tokens.unwrap_or(0)),
            video: Some(prompt.video_tokens.unwrap_or(0)),
            cache_creation_details: None,
        },
        completion_tokens_details: AnthropicCompletionDetails {
            reasoning: Some(completion.reasoning_tokens.unwrap_or(0)),
            text: Some(completion.text_tokens.unwrap_or(0)),
            accepted_prediction: Some(completion.accepted_prediction_tokens.unwrap_or(0)),
            rejected_prediction: Some(completion.rejected_prediction_tokens.unwrap_or(0)),
            audio: Some(completion.audio_tokens.unwrap_or(0)),
            image: Some(completion.image_tokens.unwrap_or(0)),
            video: Some(completion.video_tokens.unwrap_or(0)),
        },
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
        iterations: None,
        inference_geo: None,
        speed: None,
        service_tier: None,
        server_tool_use: None,
    })
}

const fn project_usage(usage: &AnthropicUsage) -> SummedUsage {
    SummedUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: Some(SummedPromptDetails {
            text_tokens: Some(usage.prompt_tokens_details.text),
            audio_tokens: usage.prompt_tokens_details.audio,
            image_tokens: usage.prompt_tokens_details.image,
            video_tokens: usage.prompt_tokens_details.video,
            web_search_requests: None,
            google_maps_grounding_requests: None,
            tool_use_tokens: None,
            character_count: None,
            image_count: None,
            video_length_seconds: None,
            audio_length_seconds: None,
            query_count: None,
            cache_write_tokens: Some(usage.prompt_tokens_details.cache_write),
            cache_creation_tokens: Some(usage.prompt_tokens_details.cache_creation),
            cached_tokens: Some(usage.prompt_tokens_details.cached),
            cached_tokens_details: None,
        }),
        completion_tokens_details: Some(SummedCompletionDetails {
            text_tokens: usage.completion_tokens_details.text,
            audio_tokens: usage.completion_tokens_details.audio,
            image_tokens: usage.completion_tokens_details.image,
            video_tokens: usage.completion_tokens_details.video,
            reasoning_tokens: usage.completion_tokens_details.reasoning,
            accepted_prediction_tokens: usage.completion_tokens_details.accepted_prediction,
            rejected_prediction_tokens: usage.completion_tokens_details.rejected_prediction,
        }),
        numeric_extras: BTreeMap::new(),
    }
}
